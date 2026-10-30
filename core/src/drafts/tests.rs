use rusqlite::{Connection, params};
use tempfile::TempDir;
use uuid::Uuid;

use super::{compose::prefixed_subject, types::*};
use crate::{
    AddAttachmentRequest, AppError, AttachmentOrigin, ErrorCode, MailAddress, MailxCore,
    RemoveAttachmentRequest,
};

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
            body_html: String::new(),
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
            body_html: "text".into(),
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
        body_html: body.into(),
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
        request.body_html = super::body::plain_text_to_html(body);
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
        &|r| r.body_html = "b".repeat(1024 * 1024 + 1),
        "draft.bodyText",
    );
    check(
        &|r| r.body_html = " ".repeat(2 * 1024 * 1024 + 1),
        "draft.bodyHtml",
    );
    check(&|r| r.body_html = "<p>nul\0</p>".into(), "draft.bodyHtml");
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
    ok.body_html = format!("line\r\nline\n{}", "b".repeat(1024 * 1024 - 11));
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
        body_html: String::new(),
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
        reply.body_text.starts_with("\n2026-"),
        "{:?}",
        reply.body_text
    );
    assert!(reply.body_text.contains("，Ann <ann@example.com> 写道：\n"));
    assert!(
        reply
            .body_text
            .ends_with("> line one\n> \n> line three\n> "),
        "{:?}",
        reply.body_text,
    );
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
            .starts_with("\n---------- 转发的邮件 ----------\n")
    );
    assert!(draft.body_text.contains("发件人：Ann <ann@example.com>\n"));
    assert!(draft.body_text.contains("日期：2026-10-"));
    assert!(draft.body_text.contains("主题：FW: Budget\n"));
    assert!(
        draft
            .body_text
            .contains("收件人：Bob <bob@example.com>, x@example.com\n你好\nsecond\n")
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
async fn localized_quotes_escape_values_and_replay_the_original_language() {
    let f = fixture(&[plain("原始正文")]);
    f.set_headers(
        0,
        "Budget <script>",
        ("Ann <b>{{date}}", "ann@example.com"),
        ("[]", "[]", "[]"),
        Some("orig@example.com"),
    );
    let request = CreateDraftFromRequest {
        email_id: f.emails[0].clone(),
        kind: DraftKind::Reply,
        client_mutation_id: id(),
    };
    let english = f
        .core
        .create_draft_from_localized(request.clone(), crate::SupportedLocale::En)
        .unwrap();
    assert!(english.body_text.starts_with("\nOn 2026-10-"));
    assert!(
        english
            .body_text
            .contains("Ann <b>{{date}} <ann@example.com> wrote:")
    );
    assert!(english.body_html.contains("&lt;b&gt;{{date}}"));
    assert!(!english.body_html.contains("<b>{{date}}"));
    assert!(english.body_text.contains("原始正文"));
    assert_eq!(english.in_reply_to.as_deref(), Some("<orig@example.com>"));
    let replay = f
        .core
        .create_draft_from_localized(request, crate::SupportedLocale::ZhCn)
        .unwrap();
    assert_eq!(replay, english);
    let forward = f
        .core
        .create_draft_from_localized(
            CreateDraftFromRequest {
                email_id: f.emails[0].clone(),
                kind: DraftKind::Forward,
                client_mutation_id: id(),
            },
            crate::SupportedLocale::En,
        )
        .unwrap();
    assert!(forward.body_text.contains("Forwarded message"));
    assert!(forward.body_text.contains("Date: 2026-10-"));
    assert!(forward.body_text.contains("Subject: Budget <script>"));
    assert!(!forward.body_html.contains("<script>"));
    assert!(forward.in_reply_to.is_none());
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

#[test]
fn callers_cannot_submit_the_derived_plain_text_projection() {
    let create = serde_json::json!({
        "accountId": null, "to": [], "cc": [], "bcc": [], "subject": "",
        "bodyHtml": "<p>real</p>", "bodyText": "forged", "clientMutationId": id(),
    });
    assert!(serde_json::from_value::<CreateDraftRequest>(create).is_err());
    let update = serde_json::json!({
        "id": id(), "expectedRevision": 1, "accountId": null,
        "to": [], "cc": [], "bcc": [], "subject": "",
        "bodyHtml": "<p>real</p>", "bodyText": "forged",
    });
    assert!(serde_json::from_value::<UpdateDraftRequest>(update).is_err());
}

#[tokio::test]
async fn legacy_plain_text_drafts_upgrade_on_read_and_persist_on_save() {
    let f = fixture(&[plain("x")]);
    let draft = f.core.create_draft(f.blank(None), NONE).unwrap();
    let text = "<script>alert('&')</script>\n\n第二行\n";
    f.db.execute(
        "UPDATE drafts SET body_html = NULL, body_text = ?2 WHERE id = ?1",
        params![draft.id, text],
    )
    .unwrap();
    let legacy = f.core.get_draft(&draft.id).unwrap();
    assert_eq!(legacy.body_text, text);
    assert_eq!(legacy.body_html, super::body::plain_text_to_html(text));
    assert!(!legacy.body_html.contains("<script>"));
    assert_eq!(legacy.revision, draft.revision);
    assert_eq!(
        f.db.query_row(
            "SELECT body_html FROM drafts WHERE id = ?1",
            [&draft.id],
            |row| row.get::<_, Option<String>>(0)
        )
        .unwrap(),
        None
    );

    let saved = f
        .core
        .update_draft(UpdateDraftRequest {
            id: legacy.id.clone(),
            expected_revision: legacy.revision,
            account_id: None,
            to: legacy.to,
            cc: legacy.cc,
            bcc: legacy.bcc,
            subject: legacy.subject,
            body_html: legacy.body_html,
        })
        .unwrap();
    assert_eq!(saved.body_text, text);
    assert_eq!(saved.revision, 2);
    assert_eq!(
        f.db.query_row(
            "SELECT body_html FROM drafts WHERE id = ?1",
            [&saved.id],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        saved.body_html
    );
}

#[tokio::test]
async fn oversized_legacy_html_keeps_all_text_when_only_headers_change_or_it_is_queued() {
    const MIB: usize = 1024 * 1024;
    let f = fixture(&[plain("x")]);
    // Exactly the old text limit. Entity expansion is bounded per line so the
    // test exercises the byte limit without pathological HTML parser runtime.
    let text = format!("{}\n", "&".repeat(255)).repeat(4096);
    assert_eq!(text.len(), MIB);
    let draft = f.core.create_draft(f.blank(None), NONE).unwrap();
    f.db.execute(
        "UPDATE drafts SET body_html = NULL, body_text = ?2, to_addresses = ?3 WHERE id = ?1",
        params![
            draft.id,
            text,
            r#"[{"name":null,"address":"reader@example.com"}]"#
        ],
    )
    .unwrap();
    let legacy = f.core.get_draft(&draft.id).unwrap();
    assert!(legacy.body_html.len() > 2 * MIB);
    let service = f.core.service_for_tests();
    let queue = |draft: &Draft| {
        service
            .lock()
            .unwrap()
            .queue_send(
                &crate::QueueSendRequest {
                    draft_id: draft.id.clone(),
                    expected_revision: draft.revision,
                    client_mutation_id: id(),
                },
                30,
                chrono::Utc::now(),
            )
            .unwrap()
            .0
    };
    // Direct enqueue must not reapply the new-input limit to a legacy body.
    let queued = queue(&legacy);
    let raw = f.core.outbox_mime_for_tests(&queued.id);
    let mime = mail_parser::MessageParser::default().parse(&raw).unwrap();
    assert_eq!(mime.body_text(0).unwrap().replace("\r\n", "\n"), text);
    assert_eq!(
        mime.body_html(0).unwrap().matches("&amp;").count(),
        255 * 4096
    );
    service
        .lock()
        .unwrap()
        .cancel_outbox(&queued.id, chrono::Utc::now())
        .unwrap();

    let update = |draft: &Draft, html: String| UpdateDraftRequest {
        id: draft.id.clone(),
        expected_revision: draft.revision,
        account_id: None,
        to: draft.to.clone(),
        cc: vec![],
        bcc: vec![],
        subject: "header edit".into(),
        body_html: html,
    };
    let saved = f
        .core
        .update_draft(update(&legacy, legacy.body_html.clone()))
        .unwrap();
    assert_eq!(saved.body_html, legacy.body_html);
    assert_eq!(saved.body_text, text);
    assert_eq!(
        f.db.query_row(
            "SELECT body_html FROM drafts WHERE id = ?1",
            [&saved.id],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        saved.body_html
    );
    // Once persisted, unchanged core-owned content still stays editable.
    let again = f
        .core
        .update_draft(update(&saved, saved.body_html.clone()))
        .unwrap();
    assert_eq!(again.body_text, text);
    // An actual body change uses the ordinary new-input sanitizer and limit.
    assert_eq!(
        code(
            f.core
                .update_draft(update(&again, format!("{}<p>new</p>", again.body_html)))
        ),
        (ErrorCode::Validation, Some("draft.bodyHtml".into()))
    );
    let queued = queue(&again);
    let raw = f.core.outbox_mime_for_tests(&queued.id);
    let mime = mail_parser::MessageParser::default().parse(&raw).unwrap();
    assert_eq!(mime.body_text(0).unwrap().replace("\r\n", "\n"), text);

    // Even a valid legacy body made entirely of line breaks may exceed the
    // HTML limit. A header-only save must not truncate those lines either.
    let blank_lines = "\n".repeat(MIB);
    let other = f.core.create_draft(f.blank(None), NONE).unwrap();
    f.db.execute(
        "UPDATE drafts SET body_html = NULL, body_text = ?2 WHERE id = ?1",
        params![other.id, blank_lines],
    )
    .unwrap();
    let legacy = f.core.get_draft(&other.id).unwrap();
    let saved = f
        .core
        .update_draft(update(&legacy, legacy.body_html.clone()))
        .unwrap();
    assert_eq!(saved.body_text, blank_lines);
    assert_eq!(saved.body_html.matches("<p>").count(), MIB + 1);
}

#[tokio::test]
async fn saved_rich_content_is_sanitized_and_generates_the_mime_alternatives() {
    let f = fixture(&[plain("x")]);
    let mut request = f.blank(None);
    request.to = vec![who(None, "reader@example.com")];
    request.body_html = "<h2 data-align='center'>标题</h2><p data-align='right' onclick='steal()'>你好<span data-color='red'><mark data-highlight='yellow'><strong>朋友</strong></mark></span></p><ol start='3'><li><p>三个</p></li><li><p><a href='https://example.com' style='color:red'>链接</a></p></li></ol><blockquote><p>引用</p></blockquote><script>secret()</script><img src='https://tracker.example'>".into();
    let saved = f.core.create_draft(request, NONE).unwrap();
    assert_eq!(
        saved.body_text,
        "标题\n你好朋友\n3. 三个\n4. 链接 <https://example.com>\n> 引用"
    );
    assert!(saved.body_html.contains("<strong>朋友</strong>"));
    assert!(
        saved
            .body_html
            .contains("<h2 data-align=\"center\">标题</h2>")
    );
    assert!(
        saved
            .body_html
            .contains("<span data-color=\"red\"><mark data-highlight=\"yellow\">")
    );
    for forbidden in ["onclick", "style", "script", "secret", "<img", "tracker"] {
        assert!(!saved.body_html.contains(forbidden));
    }
    assert_eq!(f.core.get_draft(&saved.id).unwrap(), saved);
    assert!(
        f.core.list_drafts(None).unwrap()[0]
            .preview
            .contains("3. 三个")
    );
    let built = crate::smtp::message::build(&crate::smtp::message::OutgoingMessage {
        sender_address: "writer@example.com",
        sender_name: None,
        draft: &saved,
        message_id: "<test@example.com>",
        date: crate::smtp::message::local_now(),
        attachments: &[],
        inline_images: &[],
    })
    .unwrap();
    assert!(built.mime.is_ascii());
    let wire = String::from_utf8(built.mime.clone()).unwrap();
    assert!(wire.contains("multipart/alternative"));
    assert!(
        wire.find("text/plain; charset=utf-8").unwrap()
            < wire.find("text/html; charset=utf-8").unwrap()
    );
    let message = mail_parser::MessageParser::default()
        .parse(&built.mime)
        .unwrap();
    assert_eq!(
        message
            .body_text(0)
            .unwrap()
            .trim_end()
            .replace("\r\n", "\n"),
        saved.body_text
    );
    let html = message.body_html(0).unwrap();
    assert!(html.contains("<strong>朋友</strong>"));
    assert!(html.contains(
        "<h2 style=\"margin:0;font-size:18px;font-weight:bold;text-align:center\">标题</h2>"
    ));
    assert!(html.contains("<p style=\"margin:0;text-align:right\">"));
    assert!(html.contains(
        "<span style=\"color:#dc2626\"><mark style=\"background-color:#fef08a;color:inherit\">"
    ));
    assert!(!html.contains("data-"));
    assert!(html.contains("margin:0"));
    assert!(html.contains("<meta charset=\"utf-8\">"));
    assert!(!html.contains("<style") && !html.contains("class="));
}

#[tokio::test]
async fn localized_image_labels_follow_creation_language_and_replay_unchanged() {
    let raw = b"Content-Type: text/html; charset=utf-8\r\n\r\n<p>Original text</p><p><img alt='{{date}} &lt;b&gt;&amp;'></p>".to_vec();
    let f = fixture(&[raw]);
    for kind in [DraftKind::Reply, DraftKind::ReplyAll, DraftKind::Forward] {
        let request = CreateDraftFromRequest {
            email_id: f.emails[0].clone(),
            kind,
            client_mutation_id: id(),
        };
        let english = f
            .core
            .create_draft_from_localized(request.clone(), crate::SupportedLocale::En)
            .unwrap();
        assert!(english.body_text.contains("[Image: {{date}} <b>&]"));
        assert!(!english.body_text.contains("[图片："));
        assert!(
            english
                .body_html
                .contains("[Image: {{date}} &lt;b&gt;&amp;]")
        );
        assert!(!english.body_html.contains("<img") && !english.body_html.contains("<b>"));
        let replay = f
            .core
            .create_draft_from_localized(request, crate::SupportedLocale::ZhCn)
            .unwrap();
        assert_eq!(replay, english);
        let chinese = f.derive(0, kind).unwrap();
        assert!(chinese.body_text.contains("[图片：{{date}} <b>&]"));
    }
}

#[tokio::test]
async fn replies_and_forwards_reuse_cached_html_with_safe_links_and_simple_layout() {
    let raw = b"Content-Type: text/html; charset=utf-8\r\n\r\n<h2 align='right' style='color:red'>Heading</h2><h6>Small</h6><p style='text-align:center'><span style='color:red'><mark style='background-color:yellow'><strong>Bold</strong></mark></span> <a href='https://example.com'>Website</a></p><table><tr><td>Cell A</td><td>Cell B</td></tr></table><p><img src='https://example.com/image.png' alt='chart'></p><script>evil()</script>".to_vec();
    let f = fixture(&[raw]);
    let reply = f.derive(0, DraftKind::Reply).unwrap();
    assert!(reply.body_html.starts_with("<p></p>"));
    assert!(reply.body_html.contains("<blockquote>"));
    assert!(reply.body_html.contains("<h2>Heading</h2><h3>Small</h3>"));
    assert!(reply.body_html.contains("<strong>Bold</strong>"));
    assert!(reply.body_html.contains("href=\"https://example.com"));
    assert!(reply.body_text.contains("Website <https://example.com"));
    assert!(reply.body_text.contains("Cell A") && reply.body_text.contains("Cell B"));
    assert!(reply.body_text.contains("[图片：chart]"));
    for forbidden in [
        "<table", "<img", "<h6", "data-", "style=", "align=", "<span", "<mark", "<script", "evil()",
    ] {
        assert!(!reply.body_html.contains(forbidden), "{forbidden}");
    }
    let forward = f.derive(0, DraftKind::Forward).unwrap();
    assert!(forward.body_html.starts_with("<p></p>"));
    assert!(!forward.body_html.contains("<blockquote>"));
    assert!(forward.body_html.contains("<strong>Bold</strong>"));
    assert!(forward.body_html.contains("<h2>Heading</h2><h3>Small</h3>"));
}

fn add(f: &Fixture, draft: &Draft, file_name: &str, bytes: &[u8]) -> Result<Draft, AppError> {
    f.core.add_attachment(
        AddAttachmentRequest {
            draft_id: draft.id.clone(),
            expected_revision: draft.revision,
            file_name: file_name.to_owned(),
        },
        bytes,
    )
}

fn blob_files(directory: &std::path::Path) -> usize {
    fn count(path: &std::path::Path) -> usize {
        std::fs::read_dir(path).map_or(0, |entries| {
            entries
                .flatten()
                .map(|entry| {
                    let path = entry.path();
                    if path.is_dir() { count(&path) } else { 1 }
                })
                .sum()
        })
    }
    count(&directory.join("blobs"))
}

#[tokio::test]
async fn attachments_add_list_and_remove_with_revisions() {
    let f = fixture(&[plain("x")]);
    let draft = f.core.create_draft(f.blank(None), NONE).unwrap();
    assert!(draft.attachments.is_empty());

    let one = add(&f, &draft, "../dir/报告\n终稿.pdf", b"%PDF-1.7 hello").unwrap();
    assert_eq!(one.revision, draft.revision + 1);
    let [first] = &one.attachments[..] else {
        panic!("one attachment expected")
    };
    assert_eq!(first.file_name, "报告终稿.pdf");
    assert_eq!(first.media_type, "application/pdf");
    assert_eq!(first.size, 14);
    assert_eq!(first.origin, AttachmentOrigin::User);
    // The draft read back carries the same list.
    assert_eq!(f.core.get_draft(&draft.id).unwrap(), one);

    // A stale revision is a conflict and changes nothing.
    assert_eq!(code(add(&f, &draft, "b.txt", b"b")).0, ErrorCode::Conflict);
    let two = add(&f, &one, "unknown.zzz", &[0, 1, 2]).unwrap();
    assert_eq!(two.attachments[1].media_type, "application/octet-stream");

    let id = two.attachments[0].id.clone();
    let stale = f.core.remove_attachment(RemoveAttachmentRequest {
        draft_id: draft.id.clone(),
        attachment_id: id.clone(),
        expected_revision: one.revision,
    });
    assert_eq!(code(stale).0, ErrorCode::Conflict);
    let left = f
        .core
        .remove_attachment(RemoveAttachmentRequest {
            draft_id: draft.id.clone(),
            attachment_id: id.clone(),
            expected_revision: two.revision,
        })
        .unwrap();
    assert_eq!(left.revision, two.revision + 1);
    assert_eq!(left.attachments.len(), 1);
    let missing = f.core.remove_attachment(RemoveAttachmentRequest {
        draft_id: draft.id.clone(),
        attachment_id: id,
        expected_revision: left.revision,
    });
    assert_eq!(code(missing).0, ErrorCode::NotFound);
    assert_eq!(
        code(add(&f, &left, "..", b"x")),
        (ErrorCode::Validation, Some("fileName".to_owned()))
    );

    // Zero bytes and a repeated name are both fine; order is insertion order.
    let zero = add(&f, &left, "empty.txt", b"").unwrap();
    assert_eq!(zero.attachments.last().unwrap().size, 0);
    let repeated = add(&f, &zero, "empty.txt", b"different").unwrap();
    let names: Vec<&str> = repeated
        .attachments
        .iter()
        .map(|a| a.file_name.as_str())
        .collect();
    assert_eq!(names, ["unknown.zzz", "empty.txt", "empty.txt"]);

    // Deleting the draft takes its attachment rows along.
    f.core.delete_draft(&draft.id).unwrap();
    let rows: i64 =
        f.db.query_row("SELECT COUNT(*) FROM draft_attachments", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(rows, 0);
}

#[tokio::test]
async fn attachments_respect_the_count_and_size_limits() {
    let f = fixture(&[plain("x")]);
    let mut draft = f.core.create_draft(f.blank(None), NONE).unwrap();
    let policy = f.core.attachment_policy(&draft.account_id).unwrap();
    assert_eq!(policy.max_total_bytes, 20 * 1024 * 1024);
    assert_eq!(policy.max_count, 20);
    for index in 0..20 {
        draft = add(
            &f,
            &draft,
            &format!("{index}.txt"),
            format!("{index}").as_bytes(),
        )
        .unwrap();
    }
    assert_eq!(
        code(add(&f, &draft, "21.txt", b"x")).0,
        ErrorCode::TooManyAttachments
    );

    let big = f.core.create_draft(f.blank(None), &id()).unwrap();
    let mib = vec![7_u8; 2 * 1024 * 1024];
    let mut big = big;
    for index in 0..5 {
        let mut bytes = mib.clone();
        bytes[0] = index;
        big = add(&f, &big, "chunk.bin", &bytes).unwrap();
        big = add(&f, &big, "chunk.bin", &bytes[..bytes.len() - 1]).unwrap();
    }
    // 10 MiB + (10 MiB - 5 B) used; two more MiB tip it over.
    assert_eq!(
        code(add(&f, &big, "more.bin", &mib)).0,
        ErrorCode::AttachmentTooLarge
    );
    assert_eq!(
        code(add(&f, &big, "huge.bin", &vec![0; 21 * 1024 * 1024])).0,
        ErrorCode::AttachmentTooLarge
    );
    // Refused adds leave the draft as it was.
    assert_eq!(f.core.get_draft(&big.id).unwrap(), big);
}

#[tokio::test]
async fn a_locked_draft_refuses_attachment_changes_before_the_blob_is_written() {
    let f = fixture(&[plain("x")]);
    let mut request = f.blank(None);
    request.to = vec![who(None, "li@example.org")];
    request.subject = "queued".into();
    let draft = f.core.create_draft(request, NONE).unwrap();
    let service = f.core.service_for_tests();
    let queued = {
        let mut service = service.lock().unwrap();
        let (item, _) = service
            .queue_send(
                &crate::QueueSendRequest {
                    draft_id: draft.id.clone(),
                    expected_revision: draft.revision,
                    client_mutation_id: id(),
                },
                30,
                chrono::Utc::now(),
            )
            .unwrap();
        item
    };
    assert_eq!(
        code(add(&f, &draft, "a.txt", b"bytes")).0,
        ErrorCode::DraftLocked
    );
    let removal = f.core.remove_attachment(RemoveAttachmentRequest {
        draft_id: draft.id.clone(),
        attachment_id: id(),
        expected_revision: draft.revision,
    });
    assert_eq!(code(removal).0, ErrorCode::DraftLocked);
    service
        .lock()
        .unwrap()
        .cancel_outbox(&queued.id, chrono::Utc::now())
        .unwrap();
    let added = add(&f, &draft, "a.txt", b"bytes").unwrap();
    assert_eq!(added.attachments.len(), 1);
}

#[tokio::test]
async fn identical_content_shares_one_blob_and_cleanup_keeps_referenced_ones() {
    let f = fixture(&[plain("x")]);
    let draft = f.core.create_draft(f.blank(None), NONE).unwrap();
    let before = blob_files(f._directory.path());
    let draft = add(&f, &draft, "a.txt", b"same bytes").unwrap();
    let draft = add(&f, &draft, "b.txt", b"same bytes").unwrap();
    assert_eq!(draft.attachments.len(), 2);
    assert_eq!(blob_files(f._directory.path()), before + 1);

    let orphan = f.core.create_draft(f.blank(None), &id()).unwrap();
    let orphan = add(&f, &orphan, "o.txt", b"orphan bytes").unwrap();
    assert_eq!(blob_files(f._directory.path()), before + 2);
    f.core.delete_draft(&orphan.id).unwrap();

    // Startup cleanup frees the unreferenced blob only.
    let Fixture {
        _directory: directory,
        core,
        ..
    } = f;
    drop(core);
    let storage = crate::storage::Storage::open(directory.path()).unwrap();
    assert_eq!(blob_files(directory.path()), before + 1);
    let kept = storage.attachment_files(&draft.id).unwrap();
    assert_eq!(storage.blobs.read(&kept[0].blob).unwrap(), b"same bytes");
}

fn mixed(parts: &[(&str, &str)]) -> Vec<u8> {
    let mut raw = String::from(
        "Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nbody\r\n",
    );
    for (headers, content) in parts {
        raw.push_str(&format!("--b\r\n{headers}\r\n\r\n{content}\r\n"));
    }
    raw.push_str("--b--\r\n");
    raw.into_bytes()
}

const PDF: &str = "Content-Type: application/pdf; name=\"a.pdf\"\r\nContent-Disposition: attachment; filename=\"a.pdf\"\r\nContent-Transfer-Encoding: base64";
const PNG: &str = "Content-Type: image/png\r\nContent-ID: <logo@x>\r\nContent-Disposition: inline; filename=\"logo.png\"\r\nContent-Transfer-Encoding: base64";

#[tokio::test]
async fn forward_carries_non_inline_attachments_only_and_replays_add_nothing() {
    // "%PDF-1.7 hi" and a PNG signature, base64-encoded.
    let f = fixture(&[mixed(&[(PDF, "JVBERi0xLjcgaGk="), (PNG, "iVBORw0KGgo=")])]);
    let request = CreateDraftFromRequest {
        email_id: f.emails[0].clone(),
        kind: DraftKind::Forward,
        client_mutation_id: id(),
    };
    let draft = f.core.create_draft_from(request.clone()).unwrap();
    let [file] = &draft.attachments[..] else {
        panic!("only the PDF is carried")
    };
    assert_eq!(file.file_name, "a.pdf");
    assert_eq!(file.media_type, "application/pdf");
    assert_eq!(file.size, 11);
    assert_eq!(file.origin, AttachmentOrigin::Forwarded);

    let replay = f.core.create_draft_from(request).unwrap();
    assert_eq!(replay.id, draft.id);
    assert_eq!(replay.attachments.len(), 1);
    let rows: i64 =
        f.db.query_row("SELECT COUNT(*) FROM draft_attachments", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(rows, 1);
}

#[tokio::test]
async fn reply_does_not_carry_attachments() {
    let f = fixture(&[mixed(&[(PDF, "JVBERi0xLjcgaGk=")])]);
    let draft = f.derive(0, DraftKind::Reply).unwrap();
    assert!(draft.attachments.is_empty());
}

#[tokio::test]
async fn forward_without_attachments_has_an_empty_list() {
    let f = fixture(&[plain("x")]);
    let draft = f.derive(0, DraftKind::Forward).unwrap();
    assert!(draft.attachments.is_empty());
}

#[tokio::test]
async fn forward_over_the_policy_still_creates_the_draft_and_blocks_sending() {
    // 21 files exceed the default count limit of 20.
    let parts: Vec<(&str, &str)> = (0..21).map(|_| (PDF, "JVBERi0xLjcgaGk=")).collect();
    let f = fixture(&[mixed(&parts)]);
    let draft = f.derive(0, DraftKind::Forward).unwrap();
    assert_eq!(draft.attachments.len(), 21);
    assert!(
        draft
            .attachments
            .iter()
            .all(|file| file.origin == AttachmentOrigin::Forwarded)
    );
    // The draft survives; queueing is where the policy is enforced again.
    let request = crate::QueueSendRequest {
        draft_id: draft.id.clone(),
        expected_revision: draft.revision,
        client_mutation_id: id(),
    };
    let service = f.core.service_for_tests();
    let queued = service
        .lock()
        .unwrap()
        .prepare_send(&request, 0)
        .map(|_| ());
    assert_eq!(code(queued).0, ErrorCode::TooManyAttachments);
    assert_eq!(f.core.get_draft(&draft.id).unwrap(), draft);
}

#[tokio::test]
async fn forward_of_an_uncached_body_is_still_body_not_cached() {
    let f = fixture(&[plain("x")]);
    f.db.execute(
        "UPDATE messages SET body_state = 'missing' WHERE id = ?1",
        [&f.emails[0]],
    )
    .unwrap();
    let (error, _) = code(f.derive(0, DraftKind::Forward));
    assert_eq!(error, ErrorCode::BodyNotCached);
}

fn picture(format: image::ImageFormat, width: u32, height: u32) -> Vec<u8> {
    let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        width,
        height,
        image::Rgb([200, 30, 30]),
    ));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, format).unwrap();
    bytes.into_inner()
}

fn add_inline(f: &Fixture, draft: &Draft, name: &str, bytes: &[u8]) -> Result<Draft, AppError> {
    f.core
        .add_inline_images(
            &draft.id,
            draft.revision,
            vec![(name.to_owned(), bytes.to_vec())],
        )
        .map(|(draft, _)| draft)
}

fn inline_of(draft: &Draft) -> Vec<&crate::DraftAttachmentSummary> {
    draft.attachments.iter().filter(|a| a.inline).collect()
}

fn set_body(f: &Fixture, draft: &Draft, body_html: String) -> Draft {
    f.core
        .update_draft(UpdateDraftRequest {
            id: draft.id.clone(),
            expected_revision: draft.revision,
            account_id: None,
            to: draft.to.clone(),
            cc: vec![],
            bcc: vec![],
            subject: draft.subject.clone(),
            body_html,
        })
        .unwrap()
}

fn img(cid: &str) -> String {
    format!("<p>x<img src=\"cid:{cid}\"></p>")
}

#[tokio::test]
async fn inline_images_are_stored_with_a_content_id_and_a_sniffed_type() {
    let f = fixture(&[plain("x")]);
    let draft = f.core.create_draft(f.blank(None), NONE).unwrap();
    // The name says jpg; the bytes say PNG, and the bytes win.
    let png = picture(image::ImageFormat::Png, 4, 3);
    let draft = add_inline(&f, &draft, "photo.jpg", &png).unwrap();
    let [image] = inline_of(&draft)[..] else {
        panic!("one inline image expected");
    };
    assert_eq!(image.media_type, "image/png");
    assert_eq!(image.file_name, "photo.jpg");
    assert_eq!(image.size, png.len() as u64);
    let cid = image.content_id.clone().unwrap();
    assert_eq!(
        super::body::cid_of(&format!("cid:{cid}")),
        Some(cid.as_str())
    );
    assert!(cid.ends_with(&format!("@{}", "example.org")) || cid.contains('@'));
    assert_eq!(draft.revision, 2);

    let draft = add(&f, &draft, "a.txt", b"regular").unwrap();
    assert!(
        draft
            .attachments
            .iter()
            .any(|a| !a.inline && a.content_id.is_none())
    );
    // Pairing: inline exactly when there is a Content-ID.
    let broken: i64 =
        f.db.query_row(
            "SELECT COUNT(*) FROM draft_attachments WHERE (inline = 1) <> (content_id IS NOT NULL)",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(broken, 0);
}

#[tokio::test]
async fn inline_images_outside_the_whitelist_are_refused() {
    let f = fixture(&[plain("x")]);
    let draft = f.core.create_draft(f.blank(None), NONE).unwrap();
    let mut apng = picture(image::ImageFormat::Png, 2, 2);
    // An animation control chunk right after IHDR makes it an APNG.
    let mut chunk = 8u32.to_be_bytes().to_vec();
    chunk.extend_from_slice(b"acTL");
    chunk.extend_from_slice(&[0, 0, 0, 2, 0, 0, 0, 0]);
    chunk.extend_from_slice(&[0, 0, 0, 0]);
    apng.splice(33..33, chunk);
    let truncated = picture(image::ImageFormat::Png, 4, 4)[..40].to_vec();
    for (name, bytes) in [
        ("a.png", b"not an image".to_vec()),
        (
            "a.svg",
            b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>".to_vec(),
        ),
        ("a.png", picture(image::ImageFormat::Bmp, 2, 2)),
        ("a.png", picture(image::ImageFormat::Tiff, 2, 2)),
        ("a.png", apng),
        ("a.png", picture(image::ImageFormat::Png, 8193, 1)),
        ("a.png", truncated),
    ] {
        assert_eq!(
            code(add_inline(&f, &draft, name, &bytes)).0,
            ErrorCode::UnsupportedImage,
            "{name}"
        );
    }
    // Refused adds leave the draft untouched.
    assert_eq!(f.core.get_draft(&draft.id).unwrap(), draft);
    for format in [image::ImageFormat::Jpeg, image::ImageFormat::Gif] {
        let ok = add_inline(
            &f,
            &f.core.get_draft(&draft.id).unwrap(),
            "p",
            &picture(format, 3, 3),
        );
        assert!(ok.is_ok(), "{format:?}");
    }
}

#[tokio::test]
async fn inline_images_share_the_attachment_limits() {
    let f = fixture(&[plain("x")]);
    let mut draft = f.core.create_draft(f.blank(None), NONE).unwrap();
    let png = picture(image::ImageFormat::Png, 2, 2);
    for _ in 0..19 {
        draft = add_inline(&f, &draft, "p.png", &png).unwrap();
        // Keep each one referenced so the add-time cleanup retains it.
        let cid = inline_of(&draft)
            .last()
            .unwrap()
            .content_id
            .clone()
            .unwrap();
        let body = format!("{}{}", draft.body_html, img(&cid));
        draft = set_body(&f, &draft, body);
    }
    draft = add(&f, &draft, "a.txt", b"x").unwrap();
    assert_eq!(draft.attachments.len(), 20);
    assert_eq!(
        code(add_inline(&f, &draft, "p.png", &png)).0,
        ErrorCode::TooManyAttachments
    );
    assert_eq!(
        code(add(&f, &draft, "b.txt", b"x")).0,
        ErrorCode::TooManyAttachments
    );
}

#[tokio::test]
async fn adding_a_file_drops_only_inline_rows_the_saved_body_does_not_show() {
    let f = fixture(&[plain("x")]);
    let png = picture(image::ImageFormat::Png, 2, 2);
    let draft = f.core.create_draft(f.blank(None), NONE).unwrap();
    let draft = add_inline(&f, &draft, "kept.png", &png).unwrap();
    let kept = inline_of(&draft)[0].content_id.clone().unwrap();
    let draft = set_body(&f, &draft, img(&kept));
    let draft = add_inline(&f, &draft, "stale.png", &png).unwrap();
    assert_eq!(inline_of(&draft).len(), 2);

    // Saving never deletes, even when the body stops showing the image.
    let draft = set_body(&f, &draft, "<p>none</p>".into());
    assert_eq!(inline_of(&draft).len(), 2);
    let draft = set_body(&f, &draft, img(&kept));
    assert_eq!(inline_of(&draft).len(), 2);

    // The next add (regular file or inline image) cleans up in its transaction.
    let draft = add(&f, &draft, "a.txt", b"x").unwrap();
    let rest = inline_of(&draft);
    assert_eq!(rest.len(), 1);
    assert_eq!(rest[0].content_id.as_deref(), Some(kept.as_str()));
    assert_eq!(draft.attachments.len(), 2);

    let draft = set_body(&f, &draft, "<p>none</p>".into());
    let draft = add_inline(&f, &draft, "new.png", &png).unwrap();
    let rest = inline_of(&draft);
    assert_eq!(rest.len(), 1);
    assert_ne!(rest[0].content_id.as_deref(), Some(kept.as_str()));
}

#[tokio::test]
async fn a_locked_draft_refuses_inline_images_and_stale_revisions() {
    let f = fixture(&[plain("x")]);
    let png = picture(image::ImageFormat::Png, 2, 2);
    let mut request = f.blank(None);
    request.to = vec![who(None, "li@example.org")];
    request.subject = "queued".into();
    let draft = f.core.create_draft(request, NONE).unwrap();
    let stale = draft.clone();
    let draft = add_inline(&f, &draft, "p.png", &png).unwrap();
    assert_eq!(
        code(add_inline(&f, &stale, "q.png", &png)).0,
        ErrorCode::Conflict
    );
    f.core
        .service_for_tests()
        .lock()
        .unwrap()
        .queue_send(
            &crate::QueueSendRequest {
                draft_id: draft.id.clone(),
                expected_revision: draft.revision,
                client_mutation_id: id(),
            },
            30,
            chrono::Utc::now(),
        )
        .unwrap();
    let locked = f.core.get_draft(&draft.id).unwrap();
    assert_eq!(
        code(add_inline(&f, &locked, "q.png", &png)).0,
        ErrorCode::DraftLocked
    );
}

#[tokio::test]
async fn inline_previews_are_the_validated_transcode_never_the_raw_bytes() {
    let f = fixture(&[plain("x")]);
    let draft = f.core.create_draft(f.blank(None), NONE).unwrap();
    let gif = picture(image::ImageFormat::Gif, 5, 4);
    let draft = add_inline(&f, &draft, "anim.gif", &gif).unwrap();
    let id = inline_of(&draft)[0].id.clone();
    let (media_type, bytes) = f.core.inline_image_preview(&draft.id, &id).unwrap();
    assert_eq!(media_type, "image/png");
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_ne!(bytes, gif);

    let draft = add(&f, &draft, "a.txt", b"regular").unwrap();
    let regular = draft.attachments.iter().find(|a| !a.inline).unwrap();
    assert_eq!(
        code(f.core.inline_image_preview(&draft.id, &regular.id)).0,
        ErrorCode::NotFound
    );
}

#[tokio::test]
async fn migration_17_adds_the_partial_unique_content_id_index() {
    let f = fixture(&[plain("x")]);
    let version: i64 =
        f.db.query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
    assert!(version >= 17);
    let draft = f.core.create_draft(f.blank(None), NONE).unwrap();
    let insert = |_: &str, inline: bool, cid: Option<&str>| {
        f.db.execute(
            "INSERT INTO draft_attachments
                 (draft_id, id, file_name, media_type, size, blob_hash, inline, content_id, origin, created_at)
             VALUES (?1, ?2, 'a', 'image/png', 1, ?5, ?3, ?4, 'user', '2026-01-01T00:00:00Z')",
            params![draft.id, id(), inline, cid, "0".repeat(64)],
        )
    };
    insert("a", true, Some("c@x")).unwrap();
    assert!(insert("b", true, Some("c@x")).is_err());
    // The index is partial: regular rows and other drafts are unaffected.
    insert("c", false, Some("c@x")).unwrap();
    insert("d", false, None).unwrap();
    insert("e", false, None).unwrap();
}

fn queue_now(f: &Fixture, draft: &Draft) -> Result<(), AppError> {
    f.core
        .service_for_tests()
        .lock()
        .unwrap()
        .queue_send(
            &crate::QueueSendRequest {
                draft_id: draft.id.clone(),
                expected_revision: draft.revision,
                client_mutation_id: id(),
            },
            30,
            chrono::Utc::now(),
        )
        .map(|_| ())
}

/// A regular-looking or inline row cloned from `like`'s blob, appended at the
/// end of the table so the test controls rowid order.
fn raw_row(f: &Fixture, draft: &Draft, like: &str, inline_cid: Option<&str>) {
    f.db.execute(
        "INSERT INTO draft_attachments
             (draft_id, id, file_name, media_type, size, blob_hash, inline, content_id, origin, created_at)
         SELECT draft_id, ?3, file_name, media_type, size, blob_hash, ?4, ?5, origin, created_at
         FROM draft_attachments WHERE draft_id = ?1 AND id = ?2",
        params![draft.id, like, id(), inline_cid.is_some(), inline_cid],
    )
    .unwrap();
}

#[tokio::test]
async fn queueing_matches_frozen_ids_when_inline_rows_interleave_with_files() {
    let f = fixture(&[plain("x")]);
    let png = picture(image::ImageFormat::Png, 2, 2);
    let mut request = f.blank(None);
    request.to = vec![who(None, "li@example.org")];
    request.subject = "order".into();

    // Referenced inline image inserted before a regular attachment.
    let draft = f.core.create_draft(request.clone(), NONE).unwrap();
    let (draft, images) = f
        .core
        .add_inline_images(
            &draft.id,
            draft.revision,
            vec![("a.png".into(), png.clone())],
        )
        .unwrap();
    let draft = set_body(&f, &draft, img(&images[0].content_id));
    let draft = add(&f, &draft, "b.txt", b"x").unwrap();
    assert!(draft.attachments[0].inline && !draft.attachments[1].inline);
    queue_now(&f, &draft).unwrap();

    // An unreferenced inline row between two regular rows.
    request.client_mutation_id = id();
    let draft = f.core.create_draft(request, NONE).unwrap();
    let draft = add(&f, &draft, "a.txt", b"x").unwrap();
    let first = draft.attachments[0].id.clone();
    let (draft, _) = f
        .core
        .add_inline_images(&draft.id, draft.revision, vec![("u.png".into(), png)])
        .unwrap();
    raw_row(&f, &draft, &first, None);
    let draft = f.core.get_draft(&draft.id).unwrap();
    let order: Vec<_> = draft.attachments.iter().map(|a| a.inline).collect();
    assert_eq!(order, [false, true, false]);
    queue_now(&f, &draft).unwrap();
}

#[tokio::test]
async fn queue_time_limits_count_only_what_will_be_sent() {
    let f = fixture(&[plain("x")]);
    let png = picture(image::ImageFormat::Png, 2, 2);
    let mut request = f.blank(None);
    request.to = vec![who(None, "li@example.org")];
    request.subject = "limit".into();
    let mut draft = f.core.create_draft(request, NONE).unwrap();
    for n in 0..20 {
        draft = add(&f, &draft, &format!("{n}.txt"), b"x").unwrap();
    }
    assert_eq!(draft.attachments.len(), 20);
    // A 21st row, an unreferenced inline image, only exists past the add-time
    // limit; it must not block queueing since it is not sent.
    let first = draft.attachments[0].id.clone();
    let _ = png;
    raw_row(
        &f,
        &draft,
        &first,
        Some("0123456789abcdef0123456789abcdef@example.org"),
    );
    let draft = f.core.get_draft(&draft.id).unwrap();
    assert_eq!(draft.attachments.len(), 21);
    queue_now(&f, &draft).unwrap();
}

#[tokio::test]
async fn inline_batches_are_all_or_nothing_and_keep_their_own_members() {
    let f = fixture(&[plain("x")]);
    let png = picture(image::ImageFormat::Png, 2, 2);
    let draft = f.core.create_draft(f.blank(None), NONE).unwrap();
    let (draft, images) = f
        .core
        .add_inline_images(
            &draft.id,
            draft.revision,
            vec![
                ("one.png".into(), png.clone()),
                ("two.png".into(), png.clone()),
            ],
        )
        .unwrap();
    assert_eq!(draft.revision, 2);
    assert_eq!(images.len(), 2);
    assert_eq!(images[0].file_name, "one.png");
    assert_eq!(images[1].file_name, "two.png");
    let rows: Vec<_> = draft.attachments.iter().map(|a| a.id.clone()).collect();
    assert_eq!(
        rows,
        [
            images[0].attachment_id.clone(),
            images[1].attachment_id.clone()
        ]
    );
    assert_eq!(
        draft.attachments[1].content_id.as_deref(),
        Some(images[1].content_id.as_str())
    );

    // One bad file stores nothing, and nothing is cleaned up either.
    let bad = f.core.add_inline_images(
        &draft.id,
        draft.revision,
        vec![
            ("three.png".into(), png),
            ("x.png".into(), b"nope".to_vec()),
        ],
    );
    assert_eq!(code(bad).0, ErrorCode::UnsupportedImage);
    assert_eq!(f.core.get_draft(&draft.id).unwrap(), draft);
}
