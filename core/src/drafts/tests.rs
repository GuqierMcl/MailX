use rusqlite::{Connection, params};
use tempfile::TempDir;
use uuid::Uuid;

use super::{compose::prefixed_subject, types::*};
use crate::{AppError, ErrorCode, MailAddress, MailxCore};

const NONE: &str = "";

fn id() -> String {
    Uuid::new_v4().to_string()
}

fn who(name: Option<&str>, address: &str) -> MailAddress {
    MailAddress {
        name: name.map(str::to_owned),
        address: address.to_owned(),
    }
}

fn plain(raw: &str) -> Vec<u8> {
    format!("Content-Type: text/plain; charset=utf-8\r\n\r\n{raw}").into_bytes()
}

struct Fixture {
    _directory: TempDir,
    core: MailxCore,
    db: Connection,
    emails: Vec<String>,
    account: String,
}

/// One synthetic account (`OWN`) with one cached message per raw MIME.
fn fixture(raws: &[Vec<u8>]) -> Fixture {
    let directory = TempDir::new().unwrap();
    let refs: Vec<&[u8]> = raws.iter().map(Vec::as_slice).collect();
    let (core, emails) =
        MailxCore::body_test_fixture(directory.path(), tokio::runtime::Handle::current(), &refs)
            .unwrap();
    let db = Connection::open(directory.path().join("mailx.sqlite3")).unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    let account = db
        .query_row("SELECT id FROM accounts", [], |row| row.get(0))
        .unwrap();
    Fixture {
        _directory: directory,
        core,
        db,
        emails,
        account,
    }
}

impl Fixture {
    /// Adds an account row directly; sorted after the fixture's by `created_at`.
    fn add_account(&self, address: &str, created_at: &str) -> String {
        let account = id();
        self.db
            .execute(
                "INSERT INTO accounts(id, revision, display_name, email_address, mailbox_source_id, created_at, updated_at)
                 VALUES (?1, 1, 'Extra', ?2, ?3, ?4, ?4)",
                params![account, address, id(), created_at],
            )
            .unwrap();
        account
    }

    /// Rewrites the cached headers of message `index`.
    fn set_headers(
        &self,
        index: usize,
        subject: &str,
        from: (&str, &str),
        (to, cc, reply_to): (&str, &str, &str),
        message_id: Option<&str>,
    ) {
        self.db
            .execute(
                "UPDATE messages SET subject = ?2, from_name = ?3, from_address = ?4,
                 to_addresses = ?5, cc_addresses = ?6, reply_to_addresses = ?7,
                 message_id_header = ?8, received_at = '2026-10-20T08:30:00.000Z' WHERE id = ?1",
                params![
                    self.emails[index],
                    subject,
                    from.0,
                    from.1,
                    to,
                    cc,
                    reply_to,
                    message_id
                ],
            )
            .unwrap();
    }

    fn blank(&self, account: Option<&str>) -> CreateDraftRequest {
        CreateDraftRequest {
            account_id: account.map(str::to_owned),
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: String::new(),
            body_text: String::new(),
            client_mutation_id: id(),
        }
    }

    fn derive(&self, index: usize, kind: DraftKind) -> Result<Draft, AppError> {
        self.core.create_draft_from(CreateDraftFromRequest {
            email_id: self.emails[index].clone(),
            kind,
            client_mutation_id: id(),
        })
    }
}

fn code<T: std::fmt::Debug>(result: Result<T, AppError>) -> (ErrorCode, Option<String>) {
    let error = result.unwrap_err();
    (error.code, error.field)
}

#[tokio::test]
async fn create_is_idempotent_for_the_same_content_and_conflicts_otherwise() {
    let f = fixture(&[plain("x")]);
    let mut request = f.blank(None);
    request.to = vec![who(Some(" Lin "), " lin@example.com ")];
    request.subject = "Hello".into();
    let first = f.core.create_draft(request.clone(), NONE).unwrap();
    assert_eq!(first.revision, 1);
    assert_eq!(first.kind, DraftKind::New);
    assert_eq!(first.account_id, f.account);
    assert_eq!(first.to, vec![who(Some("Lin"), "lin@example.com")]);

    // A retry returns the draft, even after the user edited it meanwhile.
    f.core
        .update_draft(UpdateDraftRequest {
            id: first.id.clone(),
            expected_revision: 1,
            account_id: None,
            to: first.to.clone(),
            cc: vec![],
            bcc: vec![],
            subject: "Edited".into(),
            body_text: "text".into(),
        })
        .unwrap();
    let again = f.core.create_draft(request.clone(), NONE).unwrap();
    assert_eq!(again.id, first.id);
    assert_eq!(again.revision, 2);
    assert_eq!(f.core.list_drafts(None).unwrap().len(), 1);

    request.subject = "Different".into();
    assert_eq!(
        code(f.core.create_draft(request, NONE)).0,
        ErrorCode::IdempotencyConflict
    );
    let mut bad = f.blank(None);
    bad.client_mutation_id = "nope".into();
    assert_eq!(
        code(f.core.create_draft(bad, NONE)),
        (ErrorCode::Validation, Some("clientMutationId".into()))
    );
}

#[tokio::test]
async fn default_account_resolution() {
    let f = fixture(&[plain("x")]);
    let second = f.add_account("second@example.com", "2030-01-01T00:00:00.000Z");
    // No setting: the first account in list order.
    assert_eq!(
        f.core.create_draft(f.blank(None), NONE).unwrap().account_id,
        f.account
    );
    // The setting names an existing account.
    assert_eq!(
        f.core
            .create_draft(f.blank(None), &second)
            .unwrap()
            .account_id,
        second
    );
    // An explicit account wins over the setting.
    assert_eq!(
        f.core
            .create_draft(f.blank(Some(&f.account)), &second)
            .unwrap()
            .account_id,
        f.account
    );
    // The setting points to a deleted account, or is not an id at all.
    for stale in [id(), "garbage".to_owned()] {
        assert_eq!(
            f.core
                .create_draft(f.blank(None), &stale)
                .unwrap()
                .account_id,
            f.account
        );
    }
    // An unknown explicit account is an error, not a fallback.
    assert_eq!(
        code(f.core.create_draft(f.blank(Some(&id())), NONE)).0,
        ErrorCode::NotFound
    );
    // No accounts at all.
    f.db.execute("DELETE FROM drafts", []).unwrap();
    f.db.execute("DELETE FROM accounts WHERE id = ?1", [&second])
        .unwrap();
    f.db.execute("DELETE FROM messages", []).unwrap();
    f.db.execute("DELETE FROM folders", []).unwrap();
    f.db.execute("DELETE FROM accounts", []).unwrap();
    assert_eq!(
        code(f.core.create_draft(f.blank(None), NONE)).0,
        ErrorCode::NotFound
    );
}

#[tokio::test]
async fn update_checks_the_revision_and_may_move_the_draft_to_another_account() {
    let f = fixture(&[plain("x")]);
    let second = f.add_account("second@example.com", "2030-01-01T00:00:00.000Z");
    let draft = f.core.create_draft(f.blank(None), NONE).unwrap();
    let update = |expected: u32, account: Option<&str>, body: &str| UpdateDraftRequest {
        id: draft.id.clone(),
        expected_revision: expected,
        account_id: account.map(str::to_owned),
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: "S".into(),
        body_text: body.into(),
    };
    let saved = f.core.update_draft(update(1, None, "one")).unwrap();
    assert_eq!(saved.revision, 2);
    assert_eq!(saved.body_text, "one");
    assert!(saved.updated_at >= draft.updated_at);
    assert_eq!(saved.created_at, draft.created_at);

    // A stale revision never overwrites.
    assert_eq!(
        code(f.core.update_draft(update(1, None, "stale"))).0,
        ErrorCode::Conflict
    );
    assert_eq!(f.core.get_draft(&draft.id).unwrap().body_text, "one");

    let moved = f
        .core
        .update_draft(update(2, Some(&second), "two"))
        .unwrap();
    assert_eq!(
        (moved.account_id.as_str(), moved.revision),
        (second.as_str(), 3)
    );
    assert_eq!(
        code(f.core.update_draft(update(3, Some(&id()), "x"))).0,
        ErrorCode::NotFound
    );
    let mut missing = update(1, None, "x");
    missing.id = id();
    assert_eq!(code(f.core.update_draft(missing)).0, ErrorCode::NotFound);
}

#[tokio::test]
async fn list_is_newest_first_filterable_and_delete_removes() {
    let f = fixture(&[plain("x")]);
    let second = f.add_account("second@example.com", "2030-01-01T00:00:00.000Z");
    let make = |account: &str, subject: &str, body: &str| {
        let mut request = f.blank(Some(account));
        request.subject = subject.into();
        request.body_text = body.into();
        f.core.create_draft(request, NONE).unwrap()
    };
    let a = make(&f.account, "a", "  first \n line  ");
    let b = make(&second, "b", "");
    let c = make(&f.account, "c", "");
    // Force identical and distinct timestamps to pin the (updated_at, id) order.
    f.db.execute(
        "UPDATE drafts SET updated_at = '2026-01-01T00:00:00.000Z'",
        [],
    )
    .unwrap();
    f.db.execute(
        "UPDATE drafts SET updated_at = '2026-02-01T00:00:00.000Z' WHERE id = ?1",
        [&a.id],
    )
    .unwrap();
    let mut tied = vec![b.id.clone(), c.id.clone()];
    tied.sort();
    tied.reverse();
    let mut expected = vec![a.id.clone()];
    expected.extend(tied);
    let listed: Vec<String> = f
        .core
        .list_drafts(None)
        .unwrap()
        .into_iter()
        .map(|draft| draft.id)
        .collect();
    assert_eq!(listed, expected);
    let only: Vec<String> = f
        .core
        .list_drafts(Some(&f.account))
        .unwrap()
        .into_iter()
        .map(|draft| draft.subject)
        .collect();
    assert_eq!(only.len(), 2);
    assert_eq!(f.core.list_drafts(None).unwrap()[0].preview, "first line");

    f.core.delete_draft(&a.id).unwrap();
    assert_eq!(code(f.core.get_draft(&a.id)).0, ErrorCode::NotFound);
    assert_eq!(code(f.core.delete_draft(&a.id)).0, ErrorCode::NotFound);
    assert_eq!(f.core.list_drafts(None).unwrap().len(), 2);
}

#[tokio::test]
async fn content_limits_and_header_injection_are_rejected() {
    let f = fixture(&[plain("x")]);
    let check = |change: &dyn Fn(&mut CreateDraftRequest), field: &str| {
        let mut request = f.blank(None);
        change(&mut request);
        assert_eq!(
            code(f.core.create_draft(request, NONE)),
            (ErrorCode::Validation, Some(field.to_owned())),
            "{field}"
        );
    };
    for bad in [
        "no-at-sign",
        "@example.com",
        "a b@example.com",
        "a@exam ple.com",
        "a@example.com\r\nBcc: x@example.com",
        "a..b@example.com",
        "a@-example.com",
        "<a@example.com>",
        "a@example.com>",
        "é@example.com",
    ] {
        check(&|r| r.to = vec![who(None, bad)], "draft.to");
        check(&|r| r.cc = vec![who(None, bad)], "draft.cc");
        check(&|r| r.bcc = vec![who(None, bad)], "draft.bcc");
    }
    for name in ["x\r\nBcc: evil@example.com", "x\ny", "tab\there", "nul\0"] {
        check(
            &|r| r.to = vec![who(Some(name), "a@example.com")],
            "draft.to",
        );
    }
    check(
        &|r| r.to = vec![who(Some(&"n".repeat(257)), "a@example.com")],
        "draft.to",
    );
    for subject in ["a\r\nBcc: evil@example.com", "a\nb", "a\rb", "a\u{7f}"] {
        check(&|r| r.subject = subject.into(), "draft.subject");
    }
    check(&|r| r.subject = "s".repeat(999), "draft.subject");
    check(
        &|r| r.body_text = "b".repeat(1024 * 1024 + 1),
        "draft.bodyText",
    );
    let many = |count: usize| {
        (0..count)
            .map(|i| who(None, &format!("u{i}@example.com")))
            .collect::<Vec<_>>()
    };
    check(
        &|r| {
            r.to = many(50);
            r.cc = many(30);
            r.bcc = many(21);
        },
        "draft.recipients",
    );

    // The limits themselves are accepted, and newlines in the body are fine.
    let mut ok = f.blank(None);
    ok.to = many(50);
    ok.cc = many(30);
    ok.bcc = many(20);
    ok.subject = "s".repeat(998);
    ok.body_text = format!("line\r\nline\n{}", "b".repeat(1024 * 1024 - 11));
    let saved = f.core.create_draft(ok, NONE).unwrap();
    assert_eq!(saved.to.len() + saved.cc.len() + saved.bcc.len(), 100);

    // Updates are validated the same way and leave the revision alone.
    let update = UpdateDraftRequest {
        id: saved.id.clone(),
        expected_revision: 1,
        account_id: None,
        to: vec![who(None, "bad\r\n@example.com")],
        cc: vec![],
        bcc: vec![],
        subject: String::new(),
        body_text: String::new(),
    };
    assert_eq!(
        code(f.core.update_draft(update)),
        (ErrorCode::Validation, Some("draft.to".into()))
    );
    assert_eq!(f.core.get_draft(&saved.id).unwrap().revision, 1);
}

#[tokio::test]
async fn deleting_an_account_deletes_its_drafts_but_a_vanished_source_keeps_the_draft() {
    let f = fixture(&[plain("body")]);
    let extra = f.add_account("second@example.com", "2030-01-01T00:00:00.000Z");
    let kept = f.core.create_draft(f.blank(Some(&extra)), NONE).unwrap();
    let other = f
        .core
        .create_draft(f.blank(Some(&f.account)), NONE)
        .unwrap();
    f.db.execute("DELETE FROM accounts WHERE id = ?1", [&extra])
        .unwrap();
    assert_eq!(code(f.core.get_draft(&kept.id)).0, ErrorCode::NotFound);
    assert!(f.core.get_draft(&other.id).is_ok());

    let reply = f.derive(0, DraftKind::Reply).unwrap();
    assert_eq!(reply.source_email_id.as_deref(), Some(f.emails[0].as_str()));
    f.db.execute("DELETE FROM messages WHERE id = ?1", [&f.emails[0]])
        .unwrap();
    let survivor = f.core.get_draft(&reply.id).unwrap();
    assert_eq!(survivor.source_email_id, None);
    assert_eq!(survivor.kind, DraftKind::Reply);
}

#[test]
fn subject_prefixes_are_not_doubled() {
    for (subject, expected) in [
        ("Hello", "Re: Hello"),
        ("", "Re:"),
        ("Re: Hello", "Re: Hello"),
        ("RE: Hello", "RE: Hello"),
        ("re: hello", "re: hello"),
        ("回复：你好", "回复：你好"),
        ("回复:你好", "回复:你好"),
        ("答复：你好", "答复：你好"),
        ("  Re: padded", "Re: padded"),
        ("Fwd: x", "Re: Fwd: x"),
    ] {
        assert_eq!(
            prefixed_subject(subject, DraftKind::Reply),
            expected,
            "{subject}"
        );
        assert_eq!(prefixed_subject(subject, DraftKind::ReplyAll), expected);
    }
    for (subject, expected) in [
        ("Hello", "Fwd: Hello"),
        ("Fwd: Hello", "Fwd: Hello"),
        ("FW: Hello", "FW: Hello"),
        ("Fw: Hello", "Fw: Hello"),
        ("转发：你好", "转发：你好"),
        ("转发:你好", "转发:你好"),
        ("Re: Hello", "Fwd: Re: Hello"),
    ] {
        assert_eq!(
            prefixed_subject(subject, DraftKind::Forward),
            expected,
            "{subject}"
        );
    }
    // Control characters cannot survive into a header.
    assert_eq!(
        prefixed_subject("a\r\nBcc: x", DraftKind::Reply),
        "Re: a  Bcc: x"
    );
}

#[tokio::test]
async fn reply_goes_to_reply_to_else_from_and_quotes_the_text() {
    let raw = b"Message-ID: <orig@example.com>\r\nReferences: <root@example.com> <mid@example.com>\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nline one\r\n\r\nline three\r\n".to_vec();
    let f = fixture(&[raw]);
    f.set_headers(
        0,
        "Weekly",
        ("Ann", "ann@example.com"),
        (r#"[{"name":null,"address":"fixture@example.invalid"},{"name":"Bob","address":"bob@example.com"}]"#,
        r#"[{"name":null,"address":"cc@example.com"}]"#,
        r#"[{"name":"Ann Reply","address":"reply@example.com"}]"#),
        Some("orig@example.com"),
    );

    let reply = f.derive(0, DraftKind::Reply).unwrap();
    assert_eq!(reply.kind, DraftKind::Reply);
    assert_eq!(reply.account_id, f.account);
    assert_eq!(reply.revision, 1);
    assert_eq!(reply.to, vec![who(Some("Ann Reply"), "reply@example.com")]);
    assert!(reply.cc.is_empty() && reply.bcc.is_empty());
    assert_eq!(reply.subject, "Re: Weekly");
    assert!(
        reply.body_text.starts_with("\n\n2026-"),
        "{:?}",
        reply.body_text
    );
    assert!(reply.body_text.contains("，Ann <ann@example.com> 写道：\n"));
    assert!(reply.body_text.ends_with("> line one\n> \n> line three\n"));
    // Threading comes from the cached raw MIME and the header cache.
    assert_eq!(reply.in_reply_to.as_deref(), Some("<orig@example.com>"));
    assert_eq!(
        reply.references.as_deref(),
        Some("<root@example.com> <mid@example.com> <orig@example.com>")
    );

    // Without Reply-To the From address is used.
    f.db.execute("UPDATE messages SET reply_to_addresses = '[]'", [])
        .unwrap();
    let plain_reply = f.derive(0, DraftKind::Reply).unwrap();
    assert_eq!(plain_reply.to, vec![who(Some("Ann"), "ann@example.com")]);
}

#[tokio::test]
async fn reply_all_excludes_own_address_and_dedupes_case_insensitively() {
    let f = fixture(&[plain("hi")]);
    f.set_headers(
        0,
        "Re: Plans",
        ("Ann", "Ann@Example.com"),
        (r#"[{"name":null,"address":"FIXTURE@example.invalid"},{"name":"Bob","address":"bob@example.com"},{"name":"Ann again","address":"ann@example.com"}]"#,
        r#"[{"name":null,"address":"BOB@example.com"},{"name":"Cy","address":"cy@example.com"},{"name":null,"address":"fixture@EXAMPLE.invalid"}]"#,
        "[]"),
        None,
    );
    let draft = f.derive(0, DraftKind::ReplyAll).unwrap();
    assert_eq!(draft.kind, DraftKind::ReplyAll);
    assert_eq!(
        draft.to,
        vec![
            who(Some("Ann"), "Ann@Example.com"),
            who(Some("Bob"), "bob@example.com")
        ]
    );
    assert_eq!(draft.cc, vec![who(Some("Cy"), "cy@example.com")]);
    assert_eq!(draft.subject, "Re: Plans");
    // No Message-ID anywhere: no threading headers are invented.
    assert_eq!((draft.in_reply_to, draft.references), (None, None));

    // Plain reply ignores the other recipients.
    assert_eq!(f.derive(0, DraftKind::Reply).unwrap().to.len(), 1);

    // Replying to a message the account itself sent goes to its recipients.
    f.set_headers(
        0,
        "Mine",
        ("Me", "fixture@example.invalid"),
        (
            r#"[{"name":"Bob","address":"bob@example.com"}]"#,
            "[]",
            "[]",
        ),
        None,
    );
    assert_eq!(
        f.derive(0, DraftKind::Reply).unwrap().to,
        vec![who(Some("Bob"), "bob@example.com")]
    );
}

#[tokio::test]
async fn forward_has_a_header_block_and_the_original_text_and_no_threading() {
    let raw = b"Message-ID: <orig@example.com>\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n\xe4\xbd\xa0\xe5\xa5\xbd\r\nsecond\r\n".to_vec();
    let f = fixture(&[raw]);
    f.set_headers(
        0,
        "FW: Budget",
        ("Ann", "ann@example.com"),
        (r#"[{"name":"Bob","address":"bob@example.com"},{"name":null,"address":"x@example.com"}]"#,
        "[]",
        "[]"),
        Some("orig@example.com"),
    );
    let draft = f.derive(0, DraftKind::Forward).unwrap();
    assert_eq!(draft.kind, DraftKind::Forward);
    assert!(draft.to.is_empty() && draft.cc.is_empty());
    assert_eq!(draft.subject, "FW: Budget");
    assert_eq!(
        (draft.in_reply_to.clone(), draft.references.clone()),
        (None, None)
    );
    assert!(
        draft
            .body_text
            .starts_with("\n\n---------- 转发的邮件 ----------\n")
    );
    assert!(draft.body_text.contains("发件人：Ann <ann@example.com>\n"));
    assert!(draft.body_text.contains("日期：2026-10-"));
    assert!(draft.body_text.contains("主题：FW: Budget\n"));
    assert!(
        draft
            .body_text
            .contains("收件人：Bob <bob@example.com>, x@example.com\n\n你好\nsecond\n")
    );
}

#[tokio::test]
async fn from_requests_are_idempotent_and_reject_new() {
    let f = fixture(&[plain("hi")]);
    let request = CreateDraftFromRequest {
        email_id: f.emails[0].clone(),
        kind: DraftKind::Reply,
        client_mutation_id: id(),
    };
    let first = f.core.create_draft_from(request.clone()).unwrap();
    assert_eq!(
        f.core.create_draft_from(request.clone()).unwrap().id,
        first.id
    );
    // A replay does not need the message to be readable any more.
    f.db.execute("UPDATE messages SET remote_state = 'deleted'", [])
        .unwrap();
    assert_eq!(
        f.core.create_draft_from(request.clone()).unwrap().id,
        first.id
    );
    let other_kind = CreateDraftFromRequest {
        kind: DraftKind::Forward,
        ..request.clone()
    };
    assert_eq!(
        code(f.core.create_draft_from(other_kind)).0,
        ErrorCode::IdempotencyConflict
    );
    assert_eq!(f.core.list_drafts(None).unwrap().len(), 1);
    let new_kind = CreateDraftFromRequest {
        kind: DraftKind::New,
        client_mutation_id: id(),
        ..request
    };
    assert_eq!(
        code(f.core.create_draft_from(new_kind)),
        (ErrorCode::Validation, Some("kind".into()))
    );
}

#[tokio::test]
async fn uncached_bodies_and_invisible_messages_are_distinct_errors() {
    let f = fixture(&[plain("a"), plain("b"), plain("c")]);
    f.db.execute(
        "UPDATE messages SET body_state = 'missing' WHERE id = ?1",
        [&f.emails[0]],
    )
    .unwrap();
    let missing = f.derive(0, DraftKind::Reply);
    assert_eq!(code(missing).0, ErrorCode::BodyNotCached);
    f.db.execute(
        "UPDATE messages SET body_state = 'failed' WHERE id = ?1",
        [&f.emails[0]],
    )
    .unwrap();
    assert_eq!(
        code(f.derive(0, DraftKind::Forward)).0,
        ErrorCode::BodyNotCached
    );

    for (index, state) in [(1, "deleted"), (2, "stale")] {
        f.db.execute(
            "UPDATE messages SET remote_state = ?2 WHERE id = ?1",
            params![f.emails[index], state],
        )
        .unwrap();
        assert_eq!(
            code(f.derive(index, DraftKind::Reply)).0,
            ErrorCode::NotFound,
            "{state}"
        );
    }
    f.db.execute("UPDATE folders SET selectable = 0", [])
        .unwrap();
    assert_eq!(code(f.derive(0, DraftKind::Reply)).0, ErrorCode::NotFound);

    let unknown = f.core.create_draft_from(CreateDraftFromRequest {
        email_id: id(),
        kind: DraftKind::Reply,
        client_mutation_id: id(),
    });
    assert_eq!(code(unknown).0, ErrorCode::NotFound);
    let malformed = f.core.create_draft_from(CreateDraftFromRequest {
        email_id: "x".into(),
        kind: DraftKind::Reply,
        client_mutation_id: id(),
    });
    assert_eq!(
        code(malformed),
        (ErrorCode::Validation, Some("emailId".into()))
    );
    assert!(f.core.list_drafts(None).unwrap().is_empty());
}

#[tokio::test]
async fn quoted_text_is_bounded_and_references_fall_back_to_in_reply_to() {
    let long = "x".repeat(1024 * 1024);
    let raw = format!(
        "In-Reply-To: <parent@example.com>\r\nMessage-ID: <child@example.com>\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{long}\r\n"
    )
    .into_bytes();
    let f = fixture(&[raw]);
    f.set_headers(
        0,
        &"s".repeat(990),
        ("Ann", "ann@example.com"),
        ("[]", "[]", "[]"),
        Some("child@example.com"),
    );
    let draft = f.derive(0, DraftKind::Reply).unwrap();
    assert!(draft.body_text.len() <= 1024 * 1024);
    assert!(draft.subject.len() <= 998);
    assert_eq!(
        draft.references.as_deref(),
        Some("<parent@example.com> <child@example.com>")
    );
}
