use std::sync::{Arc, Mutex};

use rusqlite::{Connection, params};
use serde_json::{Value, json};
use tempfile::TempDir;
use uuid::Uuid;

use super::*;
use crate::accounts::sync_plan::tests::config;
use crate::{
    Caller, CreateDraftFromRequest, CreateDraftRequest, Draft, DraftOrigin, ErrorCode, MailAddress,
    MailxCore, QueueSendRequest,
};

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
    fx_from(dir, core, emails)
}

/// A real account (routable, plannable) whose messages are all uncached,
/// for downloads through a fake IMAP server. Stored UIDVALIDITY is 7 and the
/// uid of message `i` is `i + 1`.
fn fx_real(count: usize) -> Fx {
    let dir = TempDir::new().unwrap();
    let core = MailxCore::open(
        dir.path(),
        Arc::new(crate::secrets::test_support::MemorySecrets::default()),
        tokio::runtime::Handle::current(),
        Arc::new(|_: &crate::CoreEvent| {}),
    )
    .unwrap();
    let emails = core.with_accounts_for_test(|service| {
        let account = crate::sync::bodies::tests::create_account(service, "first");
        crate::sync::bodies::tests::seed(service, &account, &vec![Some(256); count]).1
    });
    fx_from(dir, core, emails)
}

fn fx_from(dir: TempDir, core: MailxCore, emails: Vec<String>) -> Fx {
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
        Grant::mcp_default(),
        "req-mcp",
    )
}

fn agent() -> RequestContext {
    RequestContext::new(
        Principal::BuiltinAgent {
            run_id: "run-1".into(),
        },
        Grant::mcp_default(),
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
async fn both_audiences_list_their_tools_with_schemas_and_limits() {
    let f = fx();
    for audience in [Audience::Mcp, Audience::BuiltinAgent] {
        let list = f.tools.list(audience);
        let ids: Vec<_> = list.iter().map(|t| t.id).collect();
        let mut expected = vec![
            "list_accounts",
            "search_emails",
            "read_email",
            "list_folders",
            "create_draft",
            "create_reply_draft",
            "list_drafts",
            "read_draft",
            "update_draft",
            "delete_draft",
            "set_email_flags",
            "archive_emails",
            "unarchive_emails",
            "refresh_sync",
            "send_email",
            "list_outbox",
            "get_outbox",
            "cancel_outbox",
            "add_draft_attachment",
            "remove_draft_attachment",
        ];
        if audience == Audience::Mcp {
            expected.push("show_email_in_app");
            expected.push("show_draft_in_app");
        }
        assert_eq!(ids.len(), if audience == Audience::Mcp { 22 } else { 20 });
        assert_eq!(ids, expected);
        let delete = list.iter().find(|t| t.id == "delete_draft").unwrap();
        assert!(delete.annotations.destructive && !delete.annotations.read_only);
        let update = list.iter().find(|t| t.id == "update_draft").unwrap();
        assert!(!update.annotations.destructive && !update.annotations.read_only);
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
        let draft = &list[4];
        assert!(draft.description.contains("without sending"));
        assert!(draft.description.contains("untrusted"));
        assert!(draft.annotations.writes_local_data && !draft.annotations.read_only);
        assert!(list[2].description.contains("untrusted"));
        assert!(list[2].input_schema["properties"]["maxChars"].is_object());
        assert_eq!(
            list[2].input_schema["properties"]["download"]["type"],
            "boolean"
        );
        assert!(
            list[2].description.contains("never marks") || list[2].description.contains("never")
        );
        assert!(search.input_schema["properties"]["folderId"].is_object());
        let folders = &list[3];
        assert!(folders.annotations.read_only);
        assert_eq!(folders.required_actions, [Action::MailRead]);
        let reply = &list[5];
        assert!(reply.description.contains("without sending"));
        assert!(reply.description.contains("download: true"));
        assert!(reply.annotations.writes_local_data && !reply.annotations.read_only);
        assert_eq!(
            reply.required_actions,
            [Action::DraftCreate, Action::MailRead]
        );
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
    assert_eq!(
        ids,
        [
            "list_accounts",
            "search_emails",
            "read_email",
            "list_folders",
            "list_drafts",
            "read_draft",
            "show_email_in_app",
            "show_draft_in_app"
        ]
    );
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
        assert_eq!(keys, ["displayName", "emailAddress", "id", "syncStatus"]);
        // The fixture's workers may or may not have started; the value is
        // always one of the coarse categories, never a server message.
        assert!(
            ["unknown", "syncing", "connectionError"]
                .contains(&first["syncStatus"].as_str().unwrap()),
            "{first:?}"
        );
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
                "archived",
                "bodyCached",
                "folderId",
                "from",
                "hasAttachment",
                "id",
                "isRead",
                "isStarred",
                "localOnly",
                "pendingChanges",
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
        assert!(rest.get("nextCursor").is_none());

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
        assert!(end.get("nextOffset").is_none());
        // Whole text in one call ends without a next offset.
        let whole = f.read(&ctx, 1, json!({})).unwrap();
        assert_eq!(whole["text"].as_str().unwrap().trim_end(), CJK);
        assert!(whole.get("nextOffset").is_none());
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
        assert!(rest.get("nextOffset").is_none());
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
            "archived",
            "attachments",
            "bodyTruncated",
            "cc",
            "folderId",
            "from",
            "id",
            "isRead",
            "isStarred",
            "localOnly",
            "pendingChanges",
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
        // A stale default (a removed account) falls back like the composer
        // does instead of failing with notFound.
        *f.default_account.lock().unwrap() = Uuid::new_v4().to_string();
        let out = f
            .call(&ctx, "create_draft", draft_input(json!({})))
            .unwrap();
        assert!(out["accountId"] == f.account || out["accountId"] == other);
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
async fn principals_keep_their_identity_and_need_a_send_grant() {
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
        let scoped = with_grant(ctx, Grant::new(AccountScope::All, [Action::DraftCreate]));
        assert_eq!(
            code(f.call(
                &scoped,
                "send_email",
                json!({
                    "action": "send", "draftId": draft.id,
                    "expectedRevision": draft.revision,
                    "clientMutationId": Uuid::new_v4().to_string(),
                }),
            )),
            ToolErrorCode::NotPermitted
        );
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

#[tokio::test]
async fn tool_schemas_are_portable_and_still_resolve() {
    fn walk(value: &Value, path: &str, refs: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    let here = format!("{path}/{key}");
                    match key.as_str() {
                        "type" => assert!(child.is_string(), "{here}: {child}"),
                        "enum" => assert!(!child.to_string().contains("null"), "{here}"),
                        "anyOf" | "oneOf" => {
                            assert!(!child.to_string().contains(r#""type":"null""#), "{here}")
                        }
                        "$ref" => refs.push(child.as_str().unwrap().to_owned()),
                        _ => {}
                    }
                    walk(child, &here, refs);
                }
            }
            Value::Array(items) => items
                .iter()
                .enumerate()
                .for_each(|(i, child)| walk(child, &format!("{path}/{i}"), refs)),
            _ => {}
        }
    }

    let f = fx();
    for audience in [Audience::Mcp, Audience::BuiltinAgent] {
        for tool in f.tools.list(audience) {
            for (kind, schema) in [
                ("input", &tool.input_schema),
                ("output", &tool.output_schema),
            ] {
                let mut refs = Vec::new();
                walk(schema, &format!("{}.{kind}", tool.id), &mut refs);
                assert!(schema["$schema"].is_string(), "{} {kind}", tool.id);
                for target in refs {
                    let pointer = target.strip_prefix('#').expect("local $ref");
                    assert!(
                        schema.pointer(pointer).is_some(),
                        "{} {kind}: unresolved {target}",
                        tool.id
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn optional_fields_are_omittable_not_nullable() {
    let f = fx();
    let list = f.tools.list(Audience::Mcp);
    let search = list.iter().find(|t| t.id == "search_emails").unwrap();
    let props = &search.input_schema["properties"];
    assert_eq!(props["query"]["type"], "string");
    assert!(props["folderRole"].is_object());
    assert!(
        search
            .input_schema
            .get("required")
            .is_none_or(|r| r.as_array().unwrap().is_empty())
    );
    let out = &search.output_schema;
    assert_eq!(out["properties"]["nextCursor"]["type"], "string");
    assert_eq!(out["required"], json!(["emails", "coverage"]));
}

// ---- download, reply drafts, folders ----

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::sync::bodies::tests::session_for;

const RAW: &[u8] = b"From: Sender <sender@example.test>\r\nTo: me@example.test\r\n\
Subject: Remote\r\nMessage-ID: <remote@example.test>\r\n\
Content-Type: text/plain; charset=utf-8\r\n\r\nDownloaded body text\r\n";

/// Drops the stored body of message `index` of the bare `fx` core.
fn uncache_body(f: &Fx, index: usize) {
    f.db.execute(
        "DELETE FROM message_bodies WHERE message_id = ?1",
        params![f.emails[index]],
    )
    .unwrap();
    f.db.execute(
        "UPDATE messages SET body_state = 'missing' WHERE id = ?1",
        params![f.emails[index]],
    )
    .unwrap();
}

/// Message `index` of an `fx_real` core: (uid, UIDVALIDITY), unread.
fn uncached(f: &Fx, index: usize) -> (u32, u32) {
    f.db.execute(
        "UPDATE messages SET is_read = 0 WHERE id = ?1",
        params![f.emails[index]],
    )
    .unwrap();
    f.db.query_row(
        "SELECT uid, uid_validity FROM messages WHERE id = ?1",
        params![f.emails[index]],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .unwrap()
}

/// A fake IMAP server behind the body manager's test connector. Returns the
/// number of connections it was asked for. With `respond` false the server
/// accepts the command but never answers it.
fn serve(f: &Fx, uid: u32, validity: u32, respond: bool) -> Arc<AtomicUsize> {
    let (connector, mut requests) = tokio::sync::mpsc::unbounded_channel();
    f.core.body_manager().set_test_connector(connector);
    let connections = Arc::new(AtomicUsize::new(0));
    let counter = connections.clone();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Some(reply) = requests.recv().await {
            counter.fetch_add(1, Ordering::SeqCst);
            let (session, release, task) = session_for(uid, validity, validity, RAW).await;
            // A held `release` keeps the server silent; dropping it would
            // make the server give up, which is a different failure.
            let release = if respond {
                let _ = release.send(());
                None
            } else {
                Some(release)
            };
            held.push((release, task));
            let _ = reply.send(Ok(session));
        }
    });
    connections
}

impl Fx {
    /// Calls from a thread that may block, as the hosts' `spawn_blocking` does.
    fn call_blocking(
        &self,
        ctx: &RequestContext,
        tool: &str,
        input: Value,
    ) -> Result<Value, ToolError> {
        tokio::task::block_in_place(|| self.call(ctx, tool, input))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn download_true_on_a_cached_body_changes_nothing_and_contacts_no_server() {
    let f = fx();
    let (uid, validity) = {
        let i = 1;
        f.db.query_row(
            "SELECT uid, uid_validity FROM messages WHERE id = ?1",
            params![f.emails[i]],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
    };
    let connections = serve(&f, uid, validity, true);
    for ctx in both() {
        let plain = f.read(&ctx, 0, json!({})).unwrap();
        let with = f.read(&ctx, 0, json!({ "download": true })).unwrap();
        assert_eq!(plain, with);
    }
    assert_eq!(connections.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn download_false_or_omitted_on_an_uncached_body_stays_body_not_cached_offline() {
    let f = fx_real(1);
    let (uid, validity) = uncached(&f, 0);
    let connections = serve(&f, uid, validity, true);
    for ctx in both() {
        for patch in [json!({}), json!({ "download": false })] {
            assert_eq!(code(f.read(&ctx, 0, patch)), ToolErrorCode::BodyNotCached);
        }
    }
    assert_eq!(connections.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn download_true_fetches_an_uncached_body_without_marking_it_read() {
    for ctx in both() {
        let f = fx_real(1);
        let (uid, validity) = uncached(&f, 0);
        // The fake server only answers the exact `UID FETCH n (UID BODY.PEEK[])`
        // after a read-only `EXAMINE`, so a plain BODY[] fetch would fail it.
        let connections = serve(&f, uid, validity, true);
        let out = f
            .call_blocking(
                &ctx,
                "read_email",
                json!({ "emailId": f.emails[0], "download": true }),
            )
            .unwrap();
        assert!(
            out["text"]
                .as_str()
                .unwrap()
                .contains("Downloaded body text")
        );
        assert_eq!(connections.load(Ordering::SeqCst), 1);
        let is_read: i64 =
            f.db.query_row(
                "SELECT is_read FROM messages WHERE id = ?1",
                params![f.emails[0]],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(is_read, 0);
        // Now cached: the next plain read needs no server.
        let again = f.read(&ctx, 0, json!({})).unwrap();
        assert_eq!(again["text"], out["text"]);
        assert_eq!(connections.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn download_for_an_out_of_scope_or_unknown_id_makes_no_network_attempt() {
    let f = fx_real(1);
    let other = f.add_account();
    let (uid, validity) = uncached(&f, 0);
    let connections = serve(&f, uid, validity, true);
    for ctx in both() {
        let scoped = with_grant(
            ctx.clone(),
            Grant::new(AccountScope::Only(vec![other.clone()]), [Action::MailRead]),
        );
        let forbidden = f
            .call_blocking(
                &scoped,
                "read_email",
                json!({ "emailId": f.emails[0], "download": true }),
            )
            .unwrap_err();
        let missing = f
            .call_blocking(
                &scoped,
                "read_email",
                json!({ "emailId": Uuid::new_v4().to_string(), "download": true }),
            )
            .unwrap_err();
        assert_eq!(forbidden, missing);
        assert_eq!(forbidden.code, ToolErrorCode::NotFound);
    }
    assert_eq!(connections.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stalled_download_ends_in_download_timeout() {
    let f = fx_real(1);
    let (uid, validity) = uncached(&f, 0);
    serve(&f, uid, validity, false);
    super::mail::DOWNLOAD_TIMEOUT_OVERRIDE.with(|t| t.set(Some(Duration::from_millis(300))));
    let result = f.call_blocking(
        &mcp(),
        "read_email",
        json!({ "emailId": f.emails[0], "download": true }),
    );
    super::mail::DOWNLOAD_TIMEOUT_OVERRIDE.with(|t| t.set(None));
    assert_eq!(code(result), ToolErrorCode::DownloadTimeout);
}

#[tokio::test]
async fn a_download_without_a_usable_runtime_is_unavailable_not_a_panic() {
    // A current-thread runtime cannot run the download while the caller
    // blocks it, so the tool refuses instead of deadlocking.
    let f = fx_real(1);
    let (uid, validity) = uncached(&f, 0);
    let connections = serve(&f, uid, validity, true);
    let result = f.read(&mcp(), 0, json!({ "download": true }));
    assert_eq!(code(result), ToolErrorCode::Unavailable);
    assert_eq!(connections.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_download_maps_to_a_stable_code_without_server_text() {
    let f = fx_real(1);
    let (uid, validity) = uncached(&f, 0);
    let (connector, mut requests) = tokio::sync::mpsc::unbounded_channel();
    f.core.body_manager().set_test_connector(connector);
    tokio::spawn(async move {
        while let Some(reply) = requests.recv().await {
            let _ = reply.send(Err(crate::imap::ImapError::Protocol(
                "secret server text".into(),
            )));
        }
    });
    let _ = (uid, validity);
    let error = f
        .call_blocking(
            &mcp(),
            "read_email",
            json!({ "emailId": f.emails[0], "download": true }),
        )
        .unwrap_err();
    assert_eq!(error.code, ToolErrorCode::DownloadFailed);
    assert!(!format!("{error:?}").contains("secret"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_message_gone_from_the_server_becomes_not_found() {
    let f = fx_real(1);
    let (uid, validity) = uncached(&f, 0);
    // The server reports another UIDVALIDITY: the stored message is stale.
    let (connector, mut requests) = tokio::sync::mpsc::unbounded_channel();
    f.core.body_manager().set_test_connector(connector);
    tokio::spawn(async move {
        while let Some(reply) = requests.recv().await {
            let (session, release, _task) = session_for(uid, validity + 1, validity, RAW).await;
            let _ = release.send(());
            let _ = reply.send(Ok(session));
        }
    });
    let error = f
        .call_blocking(
            &mcp(),
            "read_email",
            json!({ "emailId": f.emails[0], "download": true }),
        )
        .unwrap_err();
    assert_eq!(error.code, ToolErrorCode::NotFound);
}

fn reply_input(f: &Fx, mode: &str, patch: Value) -> Value {
    let mut input = json!({ "emailId": f.emails[0], "mode": mode });
    input
        .as_object_mut()
        .unwrap()
        .extend(patch.as_object().unwrap().clone());
    input
}

#[tokio::test]
async fn reply_drafts_follow_the_composer_rules_for_each_mode() {
    let f = fx();
    f.db.execute(
        "UPDATE messages SET from_address = 'sender@example.test'",
        [],
    )
    .unwrap();
    for (mode, prefix) in [
        ("reply", "Re: "),
        ("replyAll", "Re: "),
        ("forward", "Fwd: "),
    ] {
        let out = f
            .call(
                &mcp(),
                "create_reply_draft",
                reply_input(&f, mode, json!({})),
            )
            .unwrap();
        assert_eq!(out["mode"], mode);
        assert_eq!(out["accountId"], f.account);
        assert!(
            out["subject"].as_str().unwrap().starts_with(prefix),
            "{out}"
        );
        let draft = f.core.get_draft(out["draftId"].as_str().unwrap()).unwrap();
        assert_eq!(draft.origin, DraftOrigin::Mcp);
        assert_eq!(draft.subject, out["subject"]);
        if mode == "forward" {
            assert!(draft.to.is_empty());
        } else {
            assert!(!draft.to.is_empty());
        }
        assert!(!draft.body_html.is_empty());
    }
    assert!(f.core.list_outbox(None, true).unwrap().is_empty());
}

#[tokio::test]
async fn reply_draft_origin_follows_the_principal_and_the_text_goes_above_the_quote() {
    let f = fx();
    let quoted = f
        .call(
            &mcp(),
            "create_reply_draft",
            reply_input(&f, "reply", json!({})),
        )
        .unwrap();
    let plain = f
        .core
        .get_draft(quoted["draftId"].as_str().unwrap())
        .unwrap();
    for (ctx, origin) in [(mcp(), DraftOrigin::Mcp), (agent(), DraftOrigin::Ai)] {
        let out = f
            .call(
                &ctx,
                "create_reply_draft",
                reply_input(
                    &f,
                    "reply",
                    json!({ "bodyHtml": "<p>Agent says hi</p><script>alert(1)</script>" }),
                ),
            )
            .unwrap();
        let draft = f.core.get_draft(out["draftId"].as_str().unwrap()).unwrap();
        assert_eq!(draft.origin, origin);
        assert!(!draft.body_html.contains("script"));
        assert!(draft.body_html.starts_with("<p>Agent says hi</p>"));
        assert!(draft.body_html.ends_with(&plain.body_html));
    }
    // The UI path is unchanged.
    let by_user = f
        .core
        .create_draft_from_localized(
            CreateDraftFromRequest {
                email_id: f.emails[0].clone(),
                kind: crate::DraftKind::Reply,
                client_mutation_id: Uuid::new_v4().to_string(),
            },
            crate::SupportedLocale::default(),
        )
        .unwrap();
    assert_eq!(by_user.origin, DraftOrigin::User);
}

#[tokio::test]
async fn reply_draft_errors_are_stable_and_nothing_is_created() {
    let f = fx();
    let other = f.add_account();
    let drafts = |f: &Fx| f.core.list_drafts(None).map(|d| d.len()).unwrap_or(0);
    let before = drafts(&f);
    // Uncached original: no implicit download.
    uncache_body(&f, 0);
    for ctx in both() {
        assert_eq!(
            code(f.call(
                &ctx,
                "create_reply_draft",
                reply_input(&f, "reply", json!({}))
            )),
            ToolErrorCode::BodyNotCached
        );
        let scoped = with_grant(
            ctx.clone(),
            Grant::new(
                AccountScope::Only(vec![other.clone()]),
                [Action::MailRead, Action::DraftCreate],
            ),
        );
        let forbidden = f
            .call(
                &scoped,
                "create_reply_draft",
                reply_input(&f, "reply", json!({})),
            )
            .unwrap_err();
        let missing = f
            .call(
                &scoped,
                "create_reply_draft",
                json!({ "emailId": Uuid::new_v4().to_string(), "mode": "reply" }),
            )
            .unwrap_err();
        assert_eq!(forbidden, missing);
        assert_eq!(forbidden.code, ToolErrorCode::NotFound);
        for actions in [vec![Action::DraftCreate], vec![Action::MailRead]] {
            let weak = with_grant(ctx.clone(), Grant::new(AccountScope::All, actions));
            assert_eq!(
                code(f.call(
                    &weak,
                    "create_reply_draft",
                    reply_input(&f, "reply", json!({}))
                )),
                ToolErrorCode::NotPermitted
            );
        }
        assert_eq!(
            code(f.call(
                &ctx,
                "create_reply_draft",
                reply_input(&f, "bogus", json!({}))
            )),
            ToolErrorCode::InvalidInput
        );
    }
    assert_eq!(drafts(&f), before);
}

fn inbox_id(f: &Fx) -> String {
    f.db.query_row("SELECT id FROM folders WHERE role = 'inbox'", [], |r| {
        r.get(0)
    })
    .unwrap()
}

#[tokio::test]
async fn list_folders_without_an_account_lists_only_permitted_accounts() {
    let dir = TempDir::new().unwrap();
    let core = MailxCore::open(
        dir.path(),
        Arc::new(crate::secrets::test_support::MemorySecrets::default()),
        tokio::runtime::Handle::current(),
        Arc::new(|_: &crate::CoreEvent| {}),
    )
    .unwrap();
    let mut config = config(json!({ "kind": "direct" }));
    config["displayName"] = json!("Work");
    let request: crate::CreateAccountRequest = serde_json::from_value(
        json!({ "requestId": Uuid::new_v4().to_string(), "config": config }),
    )
    .unwrap();
    let id = core.create_account(request).unwrap().id;
    let tools = Tools::new(core, String::new);
    for ctx in both() {
        // A new account has no synced folders yet.
        let all = tools.call(&ctx, "list_folders", json!({})).unwrap();
        assert_eq!(all["folders"], json!([]));
        let own = tools
            .call(&ctx, "list_folders", json!({ "accountId": id }))
            .unwrap();
        assert_eq!(own["folders"], json!([]));
        let scoped = with_grant(
            ctx.clone(),
            Grant::new(
                AccountScope::Only(vec![Uuid::new_v4().to_string()]),
                [Action::MailRead],
            ),
        );
        assert_eq!(
            tools.call(&scoped, "list_folders", json!({})).unwrap()["folders"],
            json!([])
        );
        assert_eq!(
            code(tools.call(&scoped, "list_folders", json!({ "accountId": id }))),
            ToolErrorCode::NotFound
        );
    }
}

#[tokio::test]
async fn list_folders_is_scoped_and_reports_roles() {
    let f = fx_real(3);
    file_into_trash_and_junk(&f);
    let other = Uuid::new_v4().to_string();
    for ctx in both() {
        let all = f
            .call(&ctx, "list_folders", json!({ "accountId": f.account }))
            .unwrap();
        let folders = all["folders"].as_array().unwrap();
        let roles: Vec<_> = folders.iter().filter_map(|x| x["role"].as_str()).collect();
        for role in ["inbox", "trash", "junk"] {
            assert!(roles.contains(&role), "{roles:?}");
        }
        let one = f
            .call(&ctx, "list_folders", json!({ "accountId": f.account }))
            .unwrap();
        assert!(
            one["folders"]
                .as_array()
                .unwrap()
                .iter()
                .all(|x| x["accountId"] == f.account)
        );
        let scoped = with_grant(
            ctx.clone(),
            Grant::new(AccountScope::Only(vec![other.clone()]), [Action::MailRead]),
        );
        let forbidden = f
            .call(&scoped, "list_folders", json!({ "accountId": f.account }))
            .unwrap_err();
        let missing = f
            .call(
                &scoped,
                "list_folders",
                json!({ "accountId": Uuid::new_v4().to_string() }),
            )
            .unwrap_err();
        assert_eq!(forbidden, missing);
        assert_eq!(forbidden.code, ToolErrorCode::NotFound);
        let weak = with_grant(
            ctx.clone(),
            Grant::new(AccountScope::All, [Action::AccountsRead]),
        );
        assert_eq!(
            code(f.call(&weak, "list_folders", json!({}))),
            ToolErrorCode::NotPermitted
        );
    }
}

#[tokio::test]
async fn search_by_folder_id_includes_trash_and_validates_the_filter() {
    let f = fx();
    file_into_trash_and_junk(&f);
    let other = f.add_account();
    let folder_of = |role: &str| -> String {
        f.db.query_row(
            "SELECT id FROM folders WHERE role = ?1 AND account_id = ?2",
            params![role, f.account],
            |r| r.get(0),
        )
        .unwrap()
    };
    for ctx in both() {
        let search = |input: Value| f.call(&ctx, "search_emails", input);
        assert_eq!(
            hit_ids(&search(json!({ "folderId": folder_of("trash") })).unwrap()),
            vec![f.emails[1].clone()]
        );
        assert_eq!(
            hit_ids(&search(json!({ "folderId": inbox_id(&f) })).unwrap()),
            vec![f.emails[0].clone()]
        );
        assert_eq!(
            code(search(
                json!({ "folderId": inbox_id(&f), "folderRole": "inbox" })
            )),
            ToolErrorCode::InvalidInput
        );
        // Unknown and out-of-scope folders answer identically: no results.
        let scoped = with_grant(
            ctx.clone(),
            Grant::new(AccountScope::Only(vec![other.clone()]), [Action::MailRead]),
        );
        let forbidden = f
            .call(
                &scoped,
                "search_emails",
                json!({ "folderId": folder_of("trash") }),
            )
            .unwrap();
        let unknown = f
            .call(
                &scoped,
                "search_emails",
                json!({ "folderId": Uuid::new_v4().to_string() }),
            )
            .unwrap();
        assert_eq!(forbidden, unknown);
        assert_eq!(forbidden["emails"], json!([]));
    }
}

#[tokio::test]
async fn reply_draft_text_follows_the_host_locale_read_per_call() {
    let f = fx();
    let locale = Arc::new(Mutex::new(crate::SupportedLocale::ZhCn));
    let shared = locale.clone();
    let tools =
        Tools::new(f.core.clone(), String::new).with_locale(move || *shared.lock().unwrap());
    let forward = |tools: &Tools| {
        let out = tools
            .call(
                &mcp(),
                "create_reply_draft",
                reply_input(&f, "forward", json!({})),
            )
            .unwrap();
        f.core
            .get_draft(out["draftId"].as_str().unwrap())
            .unwrap()
            .body_html
    };
    assert!(forward(&tools).contains("转发的邮件"));
    *locale.lock().unwrap() = crate::SupportedLocale::En;
    let english = forward(&tools);
    assert!(!english.contains("转发的邮件"));
    assert!(english.contains("Forwarded message"), "{english}");
}

fn user_draft(f: &Fx, account: &str, subject: &str) -> Draft {
    f.core
        .create_draft(
            CreateDraftRequest {
                account_id: Some(account.to_string()),
                to: vec![MailAddress {
                    name: None,
                    address: "ann@example.com".into(),
                }],
                cc: Vec::new(),
                bcc: Vec::new(),
                subject: subject.into(),
                body_html: "<p>mine</p>".into(),
                client_mutation_id: Uuid::new_v4().to_string(),
            },
            account,
        )
        .unwrap()
}

fn draft_ids(out: &Value) -> Vec<String> {
    out["drafts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn list_and_read_drafts_cover_all_origins_and_respect_scope() {
    let f = fx();
    let other = f.add_account();
    let mine = user_draft(&f, &f.account, "From user");
    let by_mcp = f
        .call(&mcp(), "create_draft", draft_input(json!({})))
        .unwrap();
    let by_ai = f
        .call(&agent(), "create_draft", draft_input(json!({})))
        .unwrap();
    let elsewhere = f
        .call(
            &mcp(),
            "create_draft",
            draft_input(json!({ "accountId": other })),
        )
        .unwrap();
    for ctx in both() {
        let out = f.call(&ctx, "list_drafts", json!({})).unwrap();
        let ids = draft_ids(&out);
        assert_eq!(ids.len(), 4);
        assert_eq!(out["truncated"], false);
        let origins: Vec<&str> = out["drafts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["origin"].as_str().unwrap())
            .collect();
        for expected in ["user", "mcp", "ai"] {
            assert!(origins.contains(&expected), "{origins:?}");
        }
        let item = &out["drafts"][0];
        assert!(item.get("bodyHtml").is_none());
        assert_eq!(item["locked"], false);
        assert_eq!(item["open"], false);

        let scoped = with_grant(
            ctx.clone(),
            Grant::new(
                AccountScope::Only(vec![f.account.clone()]),
                [Action::MailRead],
            ),
        );
        let out = f.call(&scoped, "list_drafts", json!({})).unwrap();
        assert_eq!(draft_ids(&out).len(), 3);
        assert!(!draft_ids(&out).contains(&elsewhere["draftId"].as_str().unwrap().to_string()));
        assert_eq!(
            code(f.call(&scoped, "list_drafts", json!({ "accountId": other }))),
            ToolErrorCode::NotFound
        );
        let filtered = f
            .call(&ctx, "list_drafts", json!({ "accountId": other }))
            .unwrap();
        assert_eq!(draft_ids(&filtered).len(), 1);

        for id in [
            mine.id.as_str(),
            by_mcp["draftId"].as_str().unwrap(),
            by_ai["draftId"].as_str().unwrap(),
        ] {
            let read = f
                .call(&ctx, "read_draft", json!({ "draftId": id }))
                .unwrap();
            assert_eq!(read["id"], id);
            assert!(read["bodyHtml"].as_str().unwrap().contains("<p>"));
            assert!(read["revision"].as_u64().is_some());
            assert_eq!(read["attachments"], json!([]));
        }
        let hidden = code(f.call(
            &scoped,
            "read_draft",
            json!({ "draftId": elsewhere["draftId"] }),
        ));
        let missing = code(f.call(
            &scoped,
            "read_draft",
            json!({ "draftId": Uuid::new_v4().to_string() }),
        ));
        let malformed = code(f.call(&scoped, "read_draft", json!({ "draftId": "x" })));
        assert_eq!(hidden, ToolErrorCode::NotFound);
        assert_eq!(missing, hidden);
        assert_eq!(malformed, hidden);
    }
}

#[tokio::test]
async fn update_draft_changes_only_given_fields_keeps_origin_and_sanitizes() {
    let f = fx();
    let mine = user_draft(&f, &f.account, "Original");
    for ctx in both() {
        let current = f.core.get_draft(&mine.id).unwrap();
        let out = f
            .call(
                &ctx,
                "update_draft",
                json!({
                    "draftId": mine.id,
                    "expectedRevision": current.revision,
                    "subject": "Changed",
                    "bodyHtml": "<p>new</p><script>alert(1)</script>"
                }),
            )
            .unwrap();
        let after = f.core.get_draft(&mine.id).unwrap();
        assert_eq!(out["revision"], after.revision);
        assert!(after.revision > current.revision);
        assert_eq!(out["subject"], "Changed");
        assert_eq!(after.subject, "Changed");
        assert!(after.body_html.contains("new") && !after.body_html.contains("script"));
        assert_eq!(after.to, current.to);
        assert_eq!(after.origin, DraftOrigin::User);

        let only_to = f
            .call(
                &ctx,
                "update_draft",
                json!({
                    "draftId": mine.id,
                    "expectedRevision": after.revision,
                    "to": [{ "address": "bob@example.com" }]
                }),
            )
            .unwrap();
        let last = f.core.get_draft(&mine.id).unwrap();
        assert_eq!(only_to["revision"], last.revision);
        assert_eq!(last.to[0].address, "bob@example.com");
        assert_eq!(last.subject, "Changed");
        assert_eq!(last.body_html, after.body_html);
    }
}

#[tokio::test]
async fn update_draft_rejects_stale_revisions_bad_input_and_missing_grant() {
    let f = fx();
    let other = f.add_account();
    let mine = user_draft(&f, &f.account, "Original");
    let elsewhere = user_draft(&f, &other, "Elsewhere");
    for ctx in both() {
        let current = f.core.get_draft(&mine.id).unwrap();
        let stale = code(f.call(
            &ctx,
            "update_draft",
            json!({ "draftId": mine.id, "expectedRevision": current.revision + 7, "subject": "x" }),
        ));
        assert_eq!(stale, ToolErrorCode::Conflict);
        assert_eq!(f.core.get_draft(&mine.id).unwrap().subject, "Original");
        assert_eq!(
            code(f.call(
                &ctx,
                "update_draft",
                json!({ "draftId": mine.id, "subject": "x" }),
            )),
            ToolErrorCode::InvalidInput
        );
        assert_eq!(
            code(f.call(
                &ctx,
                "update_draft",
                json!({ "draftId": mine.id, "expectedRevision": current.revision, "subject": "s".repeat(999) }),
            )),
            ToolErrorCode::InvalidInput
        );
        let read_only = with_grant(
            ctx.clone(),
            Grant::new(AccountScope::All, [Action::MailRead, Action::DraftCreate]),
        );
        for (tool, input) in [
            (
                "update_draft",
                json!({ "draftId": mine.id, "expectedRevision": current.revision, "subject": "x" }),
            ),
            (
                "delete_draft",
                json!({ "draftId": mine.id, "expectedRevision": current.revision }),
            ),
        ] {
            assert_eq!(
                code(f.call(&read_only, tool, input)),
                ToolErrorCode::NotPermitted
            );
        }
        let scoped = with_grant(
            ctx.clone(),
            Grant::new(
                AccountScope::Only(vec![f.account.clone()]),
                [Action::MailRead, Action::DraftWrite],
            ),
        );
        for tool in ["update_draft", "delete_draft"] {
            assert_eq!(
                code(f.call(
                    &scoped,
                    tool,
                    json!({ "draftId": elsewhere.id, "expectedRevision": elsewhere.revision }),
                )),
                ToolErrorCode::NotFound
            );
        }
    }
    assert!(f.core.get_draft(&elsewhere.id).is_ok());
}

#[tokio::test]
async fn locked_drafts_refuse_update_and_delete_and_show_locked() {
    let f = fx();
    let draft = user_draft(&f, &f.account, "Queued");
    f.core
        .queue_send(
            Caller::Composer,
            QueueSendRequest {
                draft_id: draft.id.clone(),
                expected_revision: draft.revision,
                client_mutation_id: Uuid::new_v4().to_string(),
            },
            30,
        )
        .unwrap();
    for ctx in both() {
        let read = f
            .call(&ctx, "read_draft", json!({ "draftId": draft.id }))
            .unwrap();
        assert_eq!(read["locked"], true);
        let list = f.call(&ctx, "list_drafts", json!({})).unwrap();
        assert_eq!(list["drafts"][0]["locked"], true);
        for (tool, input) in [
            (
                "update_draft",
                json!({ "draftId": draft.id, "expectedRevision": read["revision"], "subject": "x" }),
            ),
            (
                "delete_draft",
                json!({ "draftId": draft.id, "expectedRevision": read["revision"] }),
            ),
        ] {
            assert_eq!(code(f.call(&ctx, tool, input)), ToolErrorCode::DraftLocked);
        }
    }
    assert!(f.core.get_draft(&draft.id).is_ok());
}

#[tokio::test]
async fn open_drafts_are_reported_and_refuse_update_and_delete() {
    let f = fx();
    let draft = user_draft(&f, &f.account, "Open");
    let open_id = Arc::new(Mutex::new(Some(draft.id.clone())));
    let shared = open_id.clone();
    let tools = Tools::new(f.core.clone(), String::new)
        .with_draft_open_check(move |id| shared.lock().unwrap().as_deref() == Some(id));
    for ctx in both() {
        let read = tools
            .call(&ctx, "read_draft", json!({ "draftId": draft.id }))
            .unwrap();
        assert_eq!(read["open"], true);
        let list = tools.call(&ctx, "list_drafts", json!({})).unwrap();
        assert_eq!(list["drafts"][0]["open"], true);
        for (tool, input) in [
            (
                "update_draft",
                json!({ "draftId": draft.id, "expectedRevision": draft.revision, "subject": "x" }),
            ),
            (
                "delete_draft",
                json!({ "draftId": draft.id, "expectedRevision": draft.revision }),
            ),
        ] {
            let error = tools.call(&ctx, tool, input).unwrap_err();
            assert_eq!(error.code, ToolErrorCode::DraftOpen);
        }
    }
    assert_eq!(f.core.get_draft(&draft.id).unwrap().subject, "Open");
    // The check is read at call time: once the window closes, edits go through.
    *open_id.lock().unwrap() = None;
    tools
        .call(
            &agent(),
            "update_draft",
            json!({ "draftId": draft.id, "expectedRevision": draft.revision, "subject": "Closed" }),
        )
        .unwrap();
}

#[tokio::test]
async fn delete_draft_needs_the_current_revision() {
    let f = fx();
    for ctx in both() {
        let draft = user_draft(&f, &f.account, "Doomed");
        assert_eq!(
            code(f.call(
                &ctx,
                "delete_draft",
                json!({ "draftId": draft.id, "expectedRevision": draft.revision + 1 }),
            )),
            ToolErrorCode::Conflict
        );
        assert!(f.core.get_draft(&draft.id).is_ok());
        let out = f
            .call(
                &ctx,
                "delete_draft",
                json!({ "draftId": draft.id, "expectedRevision": draft.revision }),
            )
            .unwrap();
        assert_eq!(out["deleted"], true);
        assert!(f.core.get_draft(&draft.id).is_err());
        assert_eq!(
            code(f.call(
                &ctx,
                "delete_draft",
                json!({ "draftId": draft.id, "expectedRevision": draft.revision }),
            )),
            ToolErrorCode::NotFound
        );
    }
}

#[tokio::test]
async fn show_email_in_app_is_mcp_only_scoped_and_calls_the_host_hook() {
    let f = fx();
    let calls = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
    let sink = calls.clone();
    let tools = Tools::new(f.core.clone(), String::new).with_ui_opener(move |account, email| {
        sink.lock().unwrap().push((account.into(), email.into()));
        Ok(())
    });
    let email = f.emails[1].clone();
    let input = json!({ "emailId": email });

    assert_eq!(
        tools
            .call(&agent(), "show_email_in_app", input.clone())
            .unwrap_err()
            .code,
        ToolErrorCode::UnknownTool
    );
    assert!(calls.lock().unwrap().is_empty());

    let other = f.add_account();
    let scoped = with_grant(
        mcp(),
        Grant::new(AccountScope::Only(vec![other]), [Action::MailRead]),
    );
    let hidden = tools
        .call(&scoped, "show_email_in_app", input.clone())
        .unwrap_err()
        .code;
    let missing = tools
        .call(
            &scoped,
            "show_email_in_app",
            json!({ "emailId": Uuid::new_v4().to_string() }),
        )
        .unwrap_err()
        .code;
    assert_eq!(hidden, ToolErrorCode::NotFound);
    assert_eq!(missing, hidden);
    assert!(calls.lock().unwrap().is_empty());

    let no_read = with_grant(mcp(), Grant::new(AccountScope::All, [Action::AccountsRead]));
    assert_eq!(
        tools
            .call(&no_read, "show_email_in_app", input.clone())
            .unwrap_err()
            .code,
        ToolErrorCode::NotPermitted
    );

    let out = tools
        .call(&mcp(), "show_email_in_app", input.clone())
        .unwrap();
    assert_eq!(out, json!({ "opened": true }));
    assert_eq!(
        *calls.lock().unwrap(),
        vec![(f.account.clone(), email.clone())]
    );

    // Without a host the default hook says so.
    assert_eq!(
        f.call(&mcp(), "show_email_in_app", input).unwrap_err().code,
        ToolErrorCode::Unavailable
    );
}

#[tokio::test]
async fn organization_reports_each_field_without_leaking_or_modifying_out_of_scope_mail() {
    for ctx in both() {
        let f = fx();
        f.db.execute("UPDATE messages SET is_read = 0, is_flagged = 0", [])
            .unwrap();
        f.db.execute("UPDATE folders SET can_flag = 0", []).unwrap();
        let missing = Uuid::new_v4().to_string();
        let input =
            json!({ "emailIds": [f.emails[0], missing], "isRead": true, "isStarred": true });
        let denied = with_grant(
            ctx.clone(),
            Grant::new(
                AccountScope::Only(vec![f.add_account()]),
                [Action::MailOrganize],
            ),
        );
        let hidden = f.call(&denied, "set_email_flags", input.clone()).unwrap();
        for item in hidden["results"].as_array().unwrap() {
            assert_eq!(
                item["isRead"],
                json!({"status":"failed","errorCode":"notFound"})
            );
            assert_eq!(item["isStarred"], item["isRead"]);
        }
        assert!(!f.core.get_email(&f.emails[0]).unwrap().summary.is_read);
        let output = f.call(&ctx, "set_email_flags", input.clone()).unwrap();
        assert_eq!(output["results"][0]["isRead"], json!({"status":"queued"}));
        assert_eq!(
            output["results"][0]["isStarred"],
            json!({"status":"localOnly"})
        );
        assert_eq!(
            output["results"][1]["isRead"],
            json!({"status":"failed","errorCode":"notFound"})
        );
        let repeat = f.call(&ctx, "set_email_flags", input).unwrap();
        assert_eq!(repeat["results"][0]["isRead"]["status"], "unchanged");
        assert_eq!(repeat["results"][0]["isStarred"]["status"], "unchanged");
        let read = f.read(&ctx, 0, json!({})).unwrap();
        assert_eq!(read["isRead"], true);
        assert_eq!(read["isStarred"], true);
        assert_eq!(read["pendingChanges"], true);
        assert_eq!(read["localOnly"], false);
    }
}

#[tokio::test]
async fn organization_validates_batch_before_any_change_and_audits_all_fifty_targets() {
    let f = fx();
    f.db.execute("UPDATE messages SET is_read = 0", []).unwrap();
    for input in [
        json!({"emailIds": [], "isRead":true}),
        json!({"emailIds": [f.emails[0], f.emails[0]], "isRead":true}),
        json!({"emailIds": [f.emails[0]]}),
        json!({"emailIds": [f.emails[0]], "isRead":null}),
        json!({"emailIds": [f.emails[0]], "isRead":true, "delete":true}),
        json!({"emailIds": (0..51).map(|_| Uuid::new_v4().to_string()).collect::<Vec<_>>(), "isRead":true}),
    ] {
        assert_eq!(
            code(f.call(&mcp(), "set_email_flags", input)),
            ToolErrorCode::InvalidInput
        );
    }
    assert!(!f.core.get_email(&f.emails[0]).unwrap().summary.is_read);
    let ids: Vec<_> = (0..50).map(|_| Uuid::new_v4().to_string()).collect();
    let out = f
        .call(
            &mcp(),
            "set_email_flags",
            json!({"emailIds":ids,"isRead":true}),
        )
        .unwrap();
    assert_eq!(out["results"].as_array().unwrap().len(), 50);
    assert!(out["results"][0].get("isStarred").is_none());
    let records = f.audit.0.lock().unwrap();
    assert_eq!(records.last().unwrap().resource_ids, ids);
}

#[tokio::test]
async fn organization_actions_are_independently_required() {
    let f = fx();
    for (tool, input) in [
        (
            "set_email_flags",
            json!({"emailIds":[f.emails[0]],"isRead":true}),
        ),
        ("archive_emails", json!({"emailIds":[f.emails[0]]})),
        ("unarchive_emails", json!({"emailIds":[f.emails[0]]})),
        ("refresh_sync", json!({"accountId":f.account})),
    ] {
        let reader = with_grant(mcp(), Grant::new(AccountScope::All, [Action::MailRead]));
        assert_eq!(
            code(f.call(&reader, tool, input)),
            ToolErrorCode::NotPermitted
        );
        assert!(!f.tools.list_for(&reader).iter().any(|item| item.id == tool));
    }
    let descriptors = f.tools.list(Audience::Mcp);
    let flags = descriptors
        .iter()
        .find(|item| item.id == "set_email_flags")
        .unwrap();
    assert!(
        flags.annotations.writes_server_data
            && flags.annotations.open_world
            && flags.annotations.idempotent
    );
    let archive = descriptors
        .iter()
        .find(|item| item.id == "archive_emails")
        .unwrap();
    assert!(
        !archive.annotations.writes_server_data
            && !archive.annotations.open_world
            && archive.annotations.idempotent
    );
}

#[tokio::test]
async fn archive_tools_preserve_atomic_scope_checks_and_search_restore_round_trip() {
    for ctx in both() {
        let f = fx();
        let missing = Uuid::new_v4().to_string();
        assert_eq!(
            code(f.call(
                &ctx,
                "archive_emails",
                json!({"emailIds":[f.emails[0],missing]})
            )),
            ToolErrorCode::NotFound
        );
        assert!(!f.core.get_email(&f.emails[0]).unwrap().summary.archived);
        let denied = with_grant(
            ctx.clone(),
            Grant::new(
                AccountScope::Only(vec![f.add_account()]),
                [Action::MailArchive],
            ),
        );
        assert_eq!(
            code(f.call(&denied, "archive_emails", json!({"emailIds":[f.emails[0]]}))),
            ToolErrorCode::NotFound
        );
        assert!(!f.core.get_email(&f.emails[0]).unwrap().summary.archived);
        let out = f
            .call(&ctx, "archive_emails", json!({"emailIds":[f.emails[0]]}))
            .unwrap();
        assert_eq!(out["processedCount"], 1);
        assert_eq!(out["changedCount"], 1);
        assert!(out.get("undoToken").is_none());
        let repeat = f
            .call(&ctx, "archive_emails", json!({"emailIds":[f.emails[0]]}))
            .unwrap();
        assert_eq!(repeat["changedCount"], 0);
        let archived = f
            .call(&ctx, "search_emails", json!({"archive":"only"}))
            .unwrap();
        assert_eq!(archived["emails"].as_array().unwrap().len(), 1);
        assert_eq!(archived["emails"][0]["id"], f.emails[0]);
        assert_eq!(archived["emails"][0]["archived"], true);
        assert!(archived["emails"][0]["folderId"].is_string());
        assert!(archived["coverage"]["headersComplete"].is_boolean());
        assert!(archived["coverage"]["indexedBodies"].is_number());
        assert_eq!(f.read(&ctx, 0, json!({})).unwrap()["archived"], true);
        let restored = f
            .call(&ctx, "unarchive_emails", json!({"emailIds":[f.emails[0]]}))
            .unwrap();
        assert_eq!(restored["changedCount"], 1);
        assert!(!f.core.get_email(&f.emails[0]).unwrap().summary.archived);
    }
}

#[tokio::test]
async fn search_accepts_starred_and_archive_filters_and_scope_coverage_is_safe() {
    let f = fx();
    f.db.execute(
        "UPDATE messages SET is_flagged = CASE WHEN id = ?1 THEN 1 ELSE 0 END",
        [&f.emails[1]],
    )
    .unwrap();
    for ctx in both() {
        let hits = f
            .call(
                &ctx,
                "search_emails",
                json!({"starred":true,"archive":"include"}),
            )
            .unwrap();
        assert_eq!(hits["emails"].as_array().unwrap().len(), 1);
        assert_eq!(hits["emails"][0]["id"], f.emails[1]);
        assert_eq!(hits["emails"][0]["isStarred"], true);
        assert_eq!(
            code(f.call(&ctx, "search_emails", json!({"archive":true}))),
            ToolErrorCode::InvalidInput
        );
        assert_eq!(
            code(f.call(&ctx, "search_emails", json!({"archive":"invalid"}))),
            ToolErrorCode::InvalidInput
        );
        let empty = with_grant(
            ctx,
            Grant::new(AccountScope::Only(vec![]), [Action::MailRead]),
        );
        let output = f.call(&empty, "search_emails", json!({})).unwrap();
        assert_eq!(output["emails"], json!([]));
        assert_eq!(output["coverage"]["indexedBodies"], 0);
        assert!(output["coverage"].get("syncedAfter").is_none());
    }
}

#[tokio::test]
async fn show_draft_is_scoped_mcp_only_and_uses_existing_draft_hook() {
    let f = fx();
    let draft = user_draft(&f, &f.account, "Show me");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let sink = calls.clone();
    let tools = Tools::new(f.core.clone(), String::new).with_draft_opener(move |account, id| {
        sink.lock()
            .unwrap()
            .push((account.to_owned(), id.to_owned()));
        Ok(())
    });
    let input = json!({"draftId":draft.id});
    assert_eq!(
        tools
            .call(&agent(), "show_draft_in_app", input.clone())
            .unwrap_err()
            .code,
        ToolErrorCode::UnknownTool
    );
    let denied = with_grant(
        mcp(),
        Grant::new(
            AccountScope::Only(vec![f.add_account()]),
            [Action::MailRead],
        ),
    );
    assert_eq!(
        tools
            .call(&denied, "show_draft_in_app", input.clone())
            .unwrap_err()
            .code,
        ToolErrorCode::NotFound
    );
    assert!(calls.lock().unwrap().is_empty());
    assert_eq!(
        tools
            .call(&mcp(), "show_draft_in_app", input.clone())
            .unwrap(),
        json!({"opened":true})
    );
    assert_eq!(
        *calls.lock().unwrap(),
        vec![(f.account.clone(), draft.id.clone())]
    );
    assert_eq!(f.core.get_draft(&draft.id).unwrap(), draft);
    assert_eq!(
        code(f.call(&mcp(), "show_draft_in_app", input)),
        ToolErrorCode::Unavailable
    );
}

#[tokio::test]
async fn refresh_sync_is_scoped_and_reports_an_idle_worker_without_claiming_completion() {
    let f = fx_real(1);
    for ctx in both() {
        let denied = with_grant(
            ctx.clone(),
            Grant::new(
                AccountScope::Only(vec![f.add_account()]),
                [Action::MailSync],
            ),
        );
        assert_eq!(
            code(f.call(&denied, "refresh_sync", json!({"accountId":f.account}))),
            ToolErrorCode::NotFound
        );
        assert_eq!(
            code(f.call(
                &ctx,
                "refresh_sync",
                json!({"accountId":Uuid::new_v4().to_string()})
            )),
            ToolErrorCode::NotFound
        );
        let output = f
            .call(&ctx, "refresh_sync", json!({"accountId":f.account}))
            .unwrap();
        assert_eq!(output["accountId"], f.account);
        assert_eq!(output["accepted"], false);
        assert_eq!(output["syncStatus"], "unknown");
        assert_eq!(output["userActionRequired"], false);
    }
}
