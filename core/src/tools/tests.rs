use std::sync::{Arc, Mutex};

use rusqlite::{Connection, params};
use serde_json::{Value, json};
use tempfile::TempDir;
use uuid::Uuid;

use super::*;
use crate::accounts::sync_plan::tests::config;
use crate::{Caller, DraftOrigin, ErrorCode, MailxCore, QueueSendRequest};

const CJK: &str = "你好😀世界👨‍👩‍👧漢字🎉";

fn plain(raw: &str) -> Vec<u8> {
    format!("Content-Type: text/plain; charset=utf-8\r\n\r\n{raw}").into_bytes()
}

#[derive(Default)]
struct Sink(Mutex<Vec<ToolCallRecord>>);

impl ToolAudit for Sink {
    fn record(&self, record: &ToolCallRecord) {
        self.0.lock().unwrap().push(record.clone());
    }
}

struct Fx {
    _dir: TempDir,
    core: MailxCore,
    db: Connection,
    emails: Vec<String>,
    account: String,
    tools: Tools,
    audit: Arc<Sink>,
    default_account: Arc<Mutex<String>>,
}

/// One account with cached messages: 0 is ASCII, 1 is CJK/emoji, 2 is long.
fn fx() -> Fx {
    let dir = TempDir::new().unwrap();
    let long = "abcdefghij".repeat(1_200);
    let raws = [plain("hello world"), plain(CJK), plain(&long)];
    let refs: Vec<&[u8]> = raws.iter().map(Vec::as_slice).collect();
    let (core, emails) = MailxCore::body_test_fixture_with_events(
        dir.path(),
        tokio::runtime::Handle::current(),
        &refs,
        Arc::new(|_: &crate::CoreEvent| {}),
    )
    .unwrap();
    let db = Connection::open(dir.path().join("mailx.sqlite3")).unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    let account: String = db
        .query_row("SELECT id FROM accounts", [], |r| r.get(0))
        .unwrap();
    let audit = Arc::new(Sink::default());
    let default_account = Arc::new(Mutex::new(String::new()));
    let default = default_account.clone();
    let tools =
        Tools::new(core.clone(), move || default.lock().unwrap().clone()).with_audit(audit.clone());
    Fx {
        _dir: dir,
        core,
        db,
        emails,
        account,
        tools,
        audit,
        default_account,
    }
}

impl Fx {
    fn add_account(&self) -> String {
        let id = Uuid::new_v4().to_string();
        self.db
            .execute(
                "INSERT INTO accounts(id, revision, display_name, email_address, mailbox_source_id, created_at, updated_at)
                 VALUES (?1, 1, 'Other', 'other@example.com', ?2, '2030-01-01T00:00:00.000Z', '2030-01-01T00:00:00.000Z')",
                params![id, Uuid::new_v4().to_string()],
            )
            .unwrap();
        id
    }

    fn call(&self, ctx: &RequestContext, tool: &str, input: Value) -> Result<Value, ToolError> {
        self.tools.call(ctx, tool, input)
    }

    fn read(&self, ctx: &RequestContext, index: usize, extra: Value) -> Result<Value, ToolError> {
        let mut input = json!({ "emailId": self.emails[index] });
        input
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        self.call(ctx, "read_email", input)
    }
}

fn mcp() -> RequestContext {
    RequestContext::new(
        Principal::LocalMcp {
            connection_id: "conn-1".into(),
            client_name: Some("Test Client".into()),
        },
        Grant::mcp_read_and_draft(),
        "req-mcp",
    )
}

fn agent() -> RequestContext {
    RequestContext::new(
        Principal::BuiltinAgent {
            run_id: "run-1".into(),
        },
        Grant::mcp_read_and_draft(),
        "req-agent",
    )
}

fn both() -> [RequestContext; 2] {
    [mcp(), agent()]
}

fn with_grant(mut ctx: RequestContext, grant: Grant) -> RequestContext {
    ctx.grant = grant;
    ctx
}

fn code(result: Result<Value, ToolError>) -> ToolErrorCode {
    result.unwrap_err().code
}

#[tokio::test]
async fn both_audiences_list_the_four_tools_with_schemas_and_limits() {
    let f = fx();
    for audience in [Audience::Mcp, Audience::BuiltinAgent] {
        let list = f.tools.list(audience);
        let ids: Vec<_> = list.iter().map(|t| t.id).collect();
        assert_eq!(
            ids,
            [
                "list_accounts",
                "search_emails",
                "read_email",
                "create_draft"
            ]
        );
        for tool in &list {
            assert_eq!(tool.input_schema["type"], "object", "{}", tool.id);
            assert_eq!(
                tool.input_schema["additionalProperties"], false,
                "{}",
                tool.id
            );
            assert_eq!(tool.output_schema["type"], "object", "{}", tool.id);
            assert_eq!(tool.limits.max_text_chars, 32_000);
            assert!(!tool.required_actions.is_empty());
        }
        let search = &list[1];
        assert!(search.description.contains("untrusted"));
        assert!(search.annotations.read_only);
        let draft = &list[3];
        assert!(draft.description.contains("NOT sent"));
        assert!(draft.description.contains("untrusted"));
        assert!(draft.annotations.writes_local_data && !draft.annotations.read_only);
        assert!(list[2].description.contains("untrusted"));
        assert!(list[2].input_schema["properties"]["maxChars"].is_object());
    }
    let descriptor = serde_json::to_value(&f.tools.list(Audience::Mcp)[0]).unwrap();
    assert!(descriptor["inputSchema"].is_object());
    assert!(descriptor["requiredActions"][0] == "accounts:read");
}

#[tokio::test]
async fn list_for_hides_tools_the_grant_cannot_use() {
    let f = fx();
    let ctx = with_grant(
        mcp(),
        Grant::new(AccountScope::All, [Action::AccountsRead, Action::MailRead]),
    );
    let ids: Vec<_> = f.tools.list_for(&ctx).iter().map(|t| t.id).collect();
    assert_eq!(ids, ["list_accounts", "search_emails", "read_email"]);
}

#[tokio::test]
async fn a_tool_visible_to_one_audience_is_hidden_and_uncallable_from_the_other() {
    let mut f = fx();
    #[derive(serde::Deserialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    struct Empty {}
    f.tools.register(
        ToolSpec::new::<Empty, Value>("agent_only", "Agent only", "test", Annotations::READ_ONLY)
            .requires(&[Action::MailRead])
            .visible_to(Audience::BuiltinAgent, AudiencePolicy::DEFAULT),
        |_call: &ToolCall<'_>, _input: Empty| Ok(json!({ "ok": true })),
    );
    f.tools.register(
        ToolSpec::new::<Empty, Value>("nobody", "Nobody", "test", Annotations::READ_ONLY)
            .requires(&[Action::MailRead]),
        |_call: &ToolCall<'_>, _input: Empty| Ok(json!({ "ok": true })),
    );
    assert!(
        f.tools
            .list(Audience::BuiltinAgent)
            .iter()
            .any(|t| t.id == "agent_only")
    );
    assert!(
        !f.tools
            .list(Audience::Mcp)
            .iter()
            .any(|t| t.id == "agent_only")
    );
    assert!(
        !f.tools
            .list(Audience::BuiltinAgent)
            .iter()
            .any(|t| t.id == "nobody")
    );
    assert_eq!(
        f.call(&agent(), "agent_only", json!({})).unwrap(),
        json!({ "ok": true })
    );
    // Hidden reads as unknown even with a full grant, not as "forbidden".
    assert_eq!(
        code(f.call(&mcp(), "agent_only", json!({}))),
        ToolErrorCode::UnknownTool
    );
    assert_eq!(
        code(f.call(&agent(), "nobody", json!({}))),
        ToolErrorCode::UnknownTool
    );
    assert_eq!(
        code(f.call(&mcp(), "does_not_exist", json!({}))),
        ToolErrorCode::UnknownTool
    );
}

#[tokio::test]
async fn a_missing_action_is_not_permitted_and_is_checked_on_every_call() {
    let f = fx();
    for ctx in both() {
        let accounts_only = with_grant(
            ctx.clone(),
            Grant::new(AccountScope::All, [Action::AccountsRead]),
        );
        for tool in ["search_emails", "read_email", "create_draft"] {
            assert_eq!(
                code(f.call(&accounts_only, tool, json!({}))),
                ToolErrorCode::NotPermitted,
                "{tool}"
            );
        }
        // Not cached: the same tool works again under a wider grant.
        assert!(f.call(&ctx, "search_emails", json!({})).is_ok());
        assert!(f.call(&accounts_only, "search_emails", json!({})).is_err());
    }
}

#[tokio::test]
async fn list_accounts_reports_only_scoped_accounts_without_secrets() {
    let dir = TempDir::new().unwrap();
    let core = MailxCore::open(
        dir.path(),
        Arc::new(crate::secrets::test_support::MemorySecrets::default()),
        tokio::runtime::Handle::current(),
        Arc::new(|_: &crate::CoreEvent| {}),
    )
    .unwrap();
    let mut ids = Vec::new();
    for name in ["Work", "Home"] {
        let mut config = config(json!({ "kind": "direct" }));
        config["displayName"] = json!(name);
        let request: crate::CreateAccountRequest = serde_json::from_value(
            json!({ "requestId": Uuid::new_v4().to_string(), "config": config }),
        )
        .unwrap();
        ids.push(core.create_account(request).unwrap().id);
    }
    let tools = Tools::new(core, String::new);
    for ctx in both() {
        let out = tools.call(&ctx, "list_accounts", Value::Null).unwrap();
        let accounts = out["accounts"].as_array().unwrap();
        assert_eq!(accounts.len(), 2);
        let first = accounts[0].as_object().unwrap();
        let mut keys: Vec<_> = first.keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["displayName", "emailAddress", "id", "status"]);
        assert_eq!(first["status"], "unverified");
        assert!(!out.to_string().contains("mail-secret"));
        assert!(!out.to_string().contains("imap.example.com"));

        let scoped = with_grant(
            ctx.clone(),
            Grant::new(
                AccountScope::Only(vec![ids[1].clone()]),
                [Action::AccountsRead],
            ),
        );
        let out = tools.call(&scoped, "list_accounts", json!({})).unwrap();
        assert_eq!(out["accounts"].as_array().unwrap().len(), 1);
        assert_eq!(out["accounts"][0]["id"], ids[1]);
    }
}

#[tokio::test]
async fn search_lists_mail_with_cursor_and_filters() {
    let f = fx();
    for ctx in both() {
        let out = f.call(&ctx, "search_emails", json!({})).unwrap();
        let emails = out["emails"].as_array().unwrap();
        assert_eq!(emails.len(), 3);
        let hit = emails[0].as_object().unwrap();
        let mut keys: Vec<_> = hit.keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "accountId",
                "bodyCached",
                "from",
                "hasAttachment",
                "id",
                "isRead",
                "preview",
                "receivedAt",
                "subject"
            ]
        );
        assert_eq!(hit["bodyCached"], true);
        assert_eq!(hit["accountId"], f.account);

        let page = f
            .call(&ctx, "search_emails", json!({ "limit": 2 }))
            .unwrap();
        assert_eq!(page["emails"].as_array().unwrap().len(), 2);
        let cursor = page["nextCursor"].as_str().unwrap();
        let rest = f
            .call(
                &ctx,
                "search_emails",
                json!({ "limit": 2, "cursor": cursor }),
            )
            .unwrap();
        assert_eq!(rest["emails"].as_array().unwrap().len(), 1);
        assert!(rest["nextCursor"].is_null());

        let none = f
            .call(
                &ctx,
                "search_emails",
                json!({ "receivedAfter": "2999-01-01T00:00:00Z" }),
            )
            .unwrap();
        assert!(none["emails"].as_array().unwrap().is_empty());
        let attachments = f
            .call(&ctx, "search_emails", json!({ "hasAttachment": true }))
            .unwrap();
        assert!(attachments["emails"].as_array().unwrap().is_empty());
    }
}

#[tokio::test]
async fn search_ignores_out_of_scope_accounts_exactly_like_unknown_ones() {
    let f = fx();
    let other = f.add_account();
    for ctx in both() {
        let scoped = with_grant(
            ctx.clone(),
            Grant::new(AccountScope::Only(vec![other.clone()]), [Action::MailRead]),
        );
        let ask_in_scope_less = f.call(&scoped, "search_emails", json!({})).unwrap();
        let ask_forbidden = f
            .call(
                &scoped,
                "search_emails",
                json!({ "accountIds": [f.account] }),
            )
            .unwrap();
        let ask_unknown = f
            .call(
                &scoped,
                "search_emails",
                json!({ "accountIds": [Uuid::new_v4().to_string()] }),
            )
            .unwrap();
        assert_eq!(ask_forbidden, ask_unknown);
        assert_eq!(ask_forbidden["emails"], json!([]));
        assert_eq!(ask_in_scope_less["emails"], json!([]));
        // A full grant still sees the account's mail.
        let all = f
            .call(&ctx, "search_emails", json!({ "accountIds": [f.account] }))
            .unwrap();
        assert_eq!(all["emails"].as_array().unwrap().len(), 3);
    }
}

#[tokio::test]
async fn read_email_out_of_scope_and_nonexistent_ids_are_indistinguishable() {
    let f = fx();
    let other = f.add_account();
    for ctx in both() {
        let scoped = with_grant(
            ctx.clone(),
            Grant::new(AccountScope::Only(vec![other.clone()]), [Action::MailRead]),
        );
        let forbidden = f.read(&scoped, 0, json!({})).unwrap_err();
        let missing = f
            .call(
                &scoped,
                "read_email",
                json!({ "emailId": Uuid::new_v4().to_string() }),
            )
            .unwrap_err();
        assert_eq!(forbidden, missing);
        assert_eq!(forbidden.code, ToolErrorCode::NotFound);
        assert!(f.read(&ctx, 0, json!({})).is_ok());
    }
}

#[tokio::test]
async fn unknown_fields_and_wrong_types_are_rejected() {
    let f = fx();
    let cases = [
        ("list_accounts", json!({ "extra": 1 })),
        ("search_emails", json!({ "queryy": "x" })),
        ("search_emails", json!({ "limit": "ten" })),
        (
            "read_email",
            json!({ "emailId": f.emails[0], "role": "admin" }),
        ),
        ("read_email", json!({})),
        (
            "create_draft",
            json!({ "to": [], "subject": "s", "bodyHtml": "b", "send": true }),
        ),
        (
            "create_draft",
            json!({ "to": [{ "address": "a@b.c", "x": 1 }], "subject": "s", "bodyHtml": "b" }),
        ),
    ];
    for ctx in both() {
        for (tool, input) in &cases {
            let error = f.call(&ctx, tool, input.clone()).unwrap_err();
            assert_eq!(error.code, ToolErrorCode::InvalidInput, "{tool} {input}");
        }
    }
}

#[tokio::test]
async fn length_and_count_limits_are_enforced() {
    let f = fx();
    let addr = |i: usize| json!({ "address": format!("u{i}@example.com") });
    for ctx in both() {
        let invalid = |tool: &str, input: Value| {
            let error = f.call(&ctx, tool, input).unwrap_err();
            assert_eq!(error.code, ToolErrorCode::InvalidInput, "{tool}");
            error.field.unwrap()
        };
        assert_eq!(
            invalid("search_emails", json!({ "query": "x".repeat(257) })),
            "query"
        );
        assert!(
            f.call(&ctx, "search_emails", json!({ "query": "字".repeat(256) }))
                .is_ok()
        );
        assert_eq!(invalid("search_emails", json!({ "limit": 0 })), "limit");
        assert_eq!(invalid("search_emails", json!({ "limit": 101 })), "limit");
        assert!(
            f.call(&ctx, "search_emails", json!({ "limit": 100 }))
                .is_ok()
        );
        assert_eq!(
            invalid("search_emails", json!({ "receivedAfter": "yesterday" })),
            "receivedAfter"
        );
        assert_eq!(
            invalid("search_emails", json!({ "accountIds": vec!["a"; 21] })),
            "accountIds"
        );
        assert_eq!(
            invalid(
                "read_email",
                json!({ "emailId": f.emails[0], "maxChars": 0 })
            ),
            "maxChars"
        );
        assert_eq!(
            invalid(
                "read_email",
                json!({ "emailId": f.emails[0], "maxChars": 32_001 })
            ),
            "maxChars"
        );
        assert_eq!(
            invalid(
                "read_email",
                json!({ "emailId": f.emails[0], "offset": 99 })
            ),
            "offset"
        );
        assert_eq!(
            invalid("read_email", json!({ "emailId": "not-an-id" })),
            "emailId"
        );
        let draft = |patch: Value| {
            let mut input = json!({ "to": [addr(0)], "subject": "s", "bodyHtml": "<p>b</p>" });
            input
                .as_object_mut()
                .unwrap()
                .extend(patch.as_object().unwrap().clone());
            input
        };
        assert_eq!(
            invalid("create_draft", draft(json!({ "subject": "s".repeat(999) }))),
            "subject"
        );
        assert_eq!(
            invalid(
                "create_draft",
                draft(json!({ "bodyHtml": "x".repeat(200_001) }))
            ),
            "bodyHtml"
        );
        let many: Vec<_> = (0..51).map(addr).collect();
        assert_eq!(invalid("create_draft", draft(json!({ "to": many }))), "to");
        assert_eq!(
            invalid(
                "create_draft",
                draft(json!({ "cc": (0..51).map(addr).collect::<Vec<_>>() }))
            ),
            "cc"
        );
        assert_eq!(
            invalid("create_draft", draft(json!({ "bcc": [{ "address": "" }] }))),
            "bcc"
        );
    }
    assert!(f.core.list_drafts(None).unwrap().is_empty());
}

#[tokio::test]
async fn read_email_pages_by_unicode_chars_across_multibyte_boundaries() {
    let f = fx();
    let total = CJK.chars().count();
    assert!(CJK.len() > total, "fixture must be multi-byte");
    for ctx in both() {
        let mut offset = 0;
        let mut collected = String::new();
        let mut guard = 0;
        loop {
            let out = f
                .read(&ctx, 1, json!({ "offset": offset, "maxChars": 3 }))
                .unwrap();
            assert_eq!(out["totalChars"], total);
            let piece = out["text"].as_str().unwrap();
            assert!(piece.chars().count() <= 3);
            collected.push_str(piece);
            match out["nextOffset"].as_u64() {
                Some(next) => {
                    assert_eq!(next as usize, offset + piece.chars().count());
                    offset = next as usize;
                }
                None => break,
            }
            guard += 1;
            assert!(guard < 50);
        }
        assert_eq!(collected, CJK);
        // Exactly at the end: empty text, no next offset.
        let end = f.read(&ctx, 1, json!({ "offset": total })).unwrap();
        assert_eq!(end["text"], "");
        assert!(end["nextOffset"].is_null());
        // Whole text in one call ends with null.
        let whole = f.read(&ctx, 1, json!({})).unwrap();
        assert_eq!(whole["text"].as_str().unwrap().trim_end(), CJK);
        assert!(whole["nextOffset"].is_null());
    }
}

#[tokio::test]
async fn read_email_default_and_max_chars_follow_the_policy() {
    let f = fx();
    for ctx in both() {
        let first = f.read(&ctx, 2, json!({})).unwrap();
        assert_eq!(first["text"].as_str().unwrap().chars().count(), 8_000);
        assert_eq!(first["nextOffset"], 8_000);
        assert_eq!(first["totalChars"], 12_000);
        let rest = f
            .read(&ctx, 2, json!({ "offset": 8_000, "maxChars": 32_000 }))
            .unwrap();
        assert_eq!(rest["text"].as_str().unwrap().chars().count(), 4_000);
        assert!(rest["nextOffset"].is_null());
        assert_eq!(
            code(f.read(&ctx, 2, json!({ "maxChars": 32_001 }))),
            ToolErrorCode::InvalidInput
        );
    }
}

#[tokio::test]
async fn read_email_returns_headers_and_attachment_shape() {
    let f = fx();
    let out = f.read(&mcp(), 0, json!({})).unwrap();
    let mut keys: Vec<_> = out.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "accountId",
            "attachments",
            "bodyTruncated",
            "cc",
            "from",
            "id",
            "nextOffset",
            "receivedAt",
            "subject",
            "text",
            "to",
            "totalChars"
        ]
    );
    assert_eq!(out["id"], f.emails[0]);
    assert!(out["text"].as_str().unwrap().contains("hello world"));
    assert_eq!(out["attachments"], json!([]));
}

#[tokio::test]
async fn an_uncached_body_is_body_not_cached_and_nothing_is_downloaded() {
    let f = fx();
    f.db.execute(
        "DELETE FROM message_bodies WHERE message_id = ?1",
        params![f.emails[0]],
    )
    .unwrap();
    f.db.execute(
        "UPDATE messages SET body_state = 'missing' WHERE id = ?1",
        params![f.emails[0]],
    )
    .unwrap();
    for ctx in both() {
        assert_eq!(
            code(f.read(&ctx, 0, json!({}))),
            ToolErrorCode::BodyNotCached
        );
        let hits = f.call(&ctx, "search_emails", json!({})).unwrap();
        let missing = hits["emails"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == f.emails[0])
            .unwrap();
        assert_eq!(missing["bodyCached"], false);
    }
}

#[tokio::test]
async fn reading_does_not_mark_the_message_read() {
    let f = fx();
    f.db.execute("UPDATE messages SET is_read = 0", []).unwrap();
    for ctx in both() {
        f.read(&ctx, 0, json!({})).unwrap();
        f.call(&ctx, "search_emails", json!({})).unwrap();
    }
    let unread: i64 =
        f.db.query_row("SELECT COUNT(*) FROM messages WHERE is_read = 0", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(unread, 3);
    let hits = f
        .call(&mcp(), "search_emails", json!({ "unread": true }))
        .unwrap();
    assert_eq!(hits["emails"].as_array().unwrap().len(), 3);
    assert_eq!(hits["emails"][0]["isRead"], false);
}

fn draft_input(patch: Value) -> Value {
    let mut input = json!({
        "to": [{ "name": "Ann", "address": "ann@example.com" }],
        "subject": "Hello",
        "bodyHtml": "<p>Hi</p><script>alert(1)</script>",
    });
    input
        .as_object_mut()
        .unwrap()
        .extend(patch.as_object().unwrap().clone());
    input
}

#[tokio::test]
async fn create_draft_records_the_origin_of_each_principal_and_never_sends() {
    let f = fx();
    let out = f
        .call(&mcp(), "create_draft", draft_input(json!({})))
        .unwrap();
    let draft = f.core.get_draft(out["draftId"].as_str().unwrap()).unwrap();
    assert_eq!(draft.origin, DraftOrigin::Mcp);
    assert_eq!(out["accountId"], f.account);
    assert_eq!(out["subject"], "Hello");
    assert!(!draft.body_html.contains("script"));

    let out = f
        .call(&agent(), "create_draft", draft_input(json!({})))
        .unwrap();
    let draft = f.core.get_draft(out["draftId"].as_str().unwrap()).unwrap();
    assert_eq!(draft.origin, DraftOrigin::Ai);

    assert!(f.core.list_outbox(None, true).unwrap().is_empty());
    // Same clientMutationId returns the same draft.
    let id = Uuid::new_v4().to_string();
    let a = f
        .call(
            &mcp(),
            "create_draft",
            draft_input(json!({ "clientMutationId": id })),
        )
        .unwrap();
    let b = f
        .call(
            &mcp(),
            "create_draft",
            draft_input(json!({ "clientMutationId": id })),
        )
        .unwrap();
    assert_eq!(a["draftId"], b["draftId"]);
    assert_eq!(
        code(f.call(
            &mcp(),
            "create_draft",
            draft_input(json!({ "clientMutationId": "x" }))
        )),
        ToolErrorCode::InvalidInput
    );
}

#[tokio::test]
async fn create_draft_resolves_the_account_within_the_grant() {
    let f = fx();
    let other = f.add_account();
    for ctx in both() {
        // Host default is used when no account is given.
        *f.default_account.lock().unwrap() = other.clone();
        let out = f
            .call(&ctx, "create_draft", draft_input(json!({})))
            .unwrap();
        assert_eq!(out["accountId"], other);
        // An explicit account wins.
        let out = f
            .call(
                &ctx,
                "create_draft",
                draft_input(json!({ "accountId": f.account })),
            )
            .unwrap();
        assert_eq!(out["accountId"], f.account);

        let scoped = with_grant(
            ctx.clone(),
            Grant::new(
                AccountScope::Only(vec![other.clone()]),
                [Action::DraftCreate],
            ),
        );
        let forbidden = f
            .call(
                &scoped,
                "create_draft",
                draft_input(json!({ "accountId": f.account })),
            )
            .unwrap_err();
        let missing = f
            .call(
                &scoped,
                "create_draft",
                draft_input(json!({ "accountId": Uuid::new_v4().to_string() })),
            )
            .unwrap_err();
        assert_eq!(forbidden, missing);
        assert_eq!(forbidden.code, ToolErrorCode::NotFound);

        // A default outside the grant is ignored in favour of an account in scope.
        *f.default_account.lock().unwrap() = f.account.clone();
        let out = f
            .call(&scoped, "create_draft", draft_input(json!({})))
            .unwrap();
        assert_eq!(out["accountId"], other);
    }
    assert!(f.core.list_outbox(None, true).unwrap().is_empty());
}

#[tokio::test]
async fn principals_map_to_callers_that_cannot_send() {
    assert_eq!(mcp().principal.caller(), Caller::Mcp);
    assert_eq!(agent().principal.caller(), Caller::Model);
    assert_eq!(mcp().principal.audience(), Audience::Mcp);
    assert_eq!(agent().principal.audience(), Audience::BuiltinAgent);

    let f = fx();
    let out = f
        .call(&agent(), "create_draft", draft_input(json!({})))
        .unwrap();
    let draft = f.core.get_draft(out["draftId"].as_str().unwrap()).unwrap();
    for ctx in both() {
        let error = f
            .core
            .queue_send(
                ctx.principal.caller(),
                QueueSendRequest {
                    draft_id: draft.id.clone(),
                    expected_revision: draft.revision,
                    client_mutation_id: Uuid::new_v4().to_string(),
                },
                0,
            )
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Forbidden);
    }
    assert!(f.core.list_outbox(None, true).unwrap().is_empty());
}

#[tokio::test]
async fn every_call_is_audited_without_content() {
    let f = fx();
    f.call(&mcp(), "read_email", json!({ "emailId": f.emails[1] }))
        .unwrap();
    f.call(
        &agent(),
        "create_draft",
        draft_input(json!({ "subject": "SECRET-SUBJECT" })),
    )
    .unwrap();
    let _ = f.call(&mcp(), "nope", json!({}));
    let _ = f.call(
        &with_grant(mcp(), Grant::new(AccountScope::All, [])),
        "search_emails",
        json!({}),
    );
    let records = f.audit.0.lock().unwrap().clone();
    assert_eq!(records.len(), 4);
    assert_eq!(records[0].principal_kind, "localMcp");
    assert_eq!(records[0].principal_id, "conn-1");
    assert_eq!(records[0].client_name.as_deref(), Some("Test Client"));
    assert_eq!(records[0].request_id, "req-mcp");
    assert_eq!(records[0].tool_id, "read_email");
    assert_eq!(records[0].resource_ids, [f.emails[1].clone()]);
    assert_eq!(records[0].error, None);
    assert_eq!(records[1].principal_kind, "builtinAgent");
    assert!(records[1].resource_ids.contains(&f.account));
    assert_eq!(records[2].error, Some(ToolErrorCode::UnknownTool));
    assert_eq!(records[3].error, Some(ToolErrorCode::NotPermitted));
    let dump = format!("{records:?}");
    assert!(!dump.contains("SECRET-SUBJECT") && !dump.contains("hello world"));
}

#[tokio::test]
async fn app_errors_map_to_stable_safe_codes() {
    use crate::AppError;
    let map = |code| ToolError::from(AppError::new(code)).code;
    assert_eq!(map(ErrorCode::NotFound), ToolErrorCode::NotFound);
    assert_eq!(map(ErrorCode::Validation), ToolErrorCode::InvalidInput);
    assert_eq!(map(ErrorCode::BodyNotCached), ToolErrorCode::BodyNotCached);
    assert_eq!(map(ErrorCode::Forbidden), ToolErrorCode::NotPermitted);
    assert_eq!(
        map(ErrorCode::StorageUnavailable),
        ToolErrorCode::Unavailable
    );
    assert_eq!(map(ErrorCode::SecretMissing), ToolErrorCode::Internal);
    let mut error = AppError::new(ErrorCode::Validation);
    error.message = "/Users/x/secret.db".into();
    error.field = Some("subject".into());
    let tool = ToolError::from(error);
    assert_eq!(tool.field.as_deref(), Some("subject"));
    assert!(!tool.message.contains("/Users"));
    let json = serde_json::to_value(ToolError::new(ToolErrorCode::BodyNotCached)).unwrap();
    assert_eq!(json["code"], "bodyNotCached");
}

/// Moves message 1 into a trash folder and message 2 into a junk folder;
/// message 0 stays in the inbox.
fn file_into_trash_and_junk(f: &Fx) {
    for (role, index) in [("trash", 1), ("junk", 2)] {
        let folder = Uuid::new_v4().to_string();
        f.db.execute(
            "INSERT INTO folders(id, account_id, mailbox_source_id, remote_name, parent_id, name, delimiter, role, selectable, attributes, unread_count, total_count, uid_next, uid_validity, synced_at)
             SELECT ?1, account_id, mailbox_source_id, ?2, NULL, ?2, delimiter, ?2, 1, attributes, 0, 0, uid_next, uid_validity, synced_at
             FROM folders WHERE role = 'inbox'",
            params![folder, role],
        )
        .unwrap();
        f.db.execute(
            "INSERT INTO folder_sync_cursors(folder_id, uid_validity, window_start, window_capped, backfill_done, highest_uid, updated_at)
             SELECT ?1, uid_validity, window_start, window_capped, backfill_done, highest_uid, updated_at
             FROM folder_sync_cursors WHERE folder_id = (SELECT id FROM folders WHERE role = 'inbox')",
            params![folder],
        )
        .unwrap();
        f.db.execute(
            "UPDATE messages SET folder_id = ?1 WHERE id = ?2",
            params![folder, f.emails[index]],
        )
        .unwrap();
    }
}

fn hit_ids(out: &Value) -> Vec<String> {
    let mut ids: Vec<String> = out["emails"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_owned())
        .collect();
    ids.sort();
    ids
}

#[tokio::test]
async fn search_excludes_trash_and_junk_unless_a_folder_role_is_given() {
    let f = fx();
    file_into_trash_and_junk(&f);
    let sorted = |ids: &[&String]| {
        let mut ids: Vec<String> = ids.iter().map(|id| (*id).clone()).collect();
        ids.sort();
        ids
    };
    for ctx in both() {
        let search = |input: Value| hit_ids(&f.call(&ctx, "search_emails", input).unwrap());
        assert_eq!(search(json!({})), sorted(&[&f.emails[0]]));
        assert_eq!(search(json!({ "query": "hello" })), sorted(&[&f.emails[0]]));
        assert!(search(json!({ "query": "abcdefghij" })).is_empty());
        assert_eq!(
            search(json!({ "folderRole": "trash" })),
            sorted(&[&f.emails[1]])
        );
        assert_eq!(
            search(json!({ "folderRole": "junk" })),
            sorted(&[&f.emails[2]])
        );
        assert_eq!(
            search(json!({ "folderRole": "trash", "query": "abcdefghij" })),
            Vec::<String>::new()
        );
        assert_eq!(
            search(json!({ "folderRole": "junk", "query": "abcdefghij" })),
            sorted(&[&f.emails[2]])
        );
        assert_eq!(
            search(json!({ "folderRole": "inbox" })),
            sorted(&[&f.emails[0]])
        );
        assert!(search(json!({ "folderRole": "sent" })).is_empty());
    }
}

#[tokio::test]
async fn search_rejects_unknown_folder_roles() {
    let f = fx();
    for ctx in both() {
        for role in [json!("Trash"), json!("spam"), json!(""), json!(3)] {
            assert_eq!(
                code(f.call(&ctx, "search_emails", json!({ "folderRole": role }))),
                ToolErrorCode::InvalidInput
            );
        }
    }
}
