use std::sync::{Arc, Mutex};

use rusqlite::{Connection, params};
use serde_json::{Value, json};
use tempfile::TempDir;

use super::*;
use crate::accounts::sync_plan::tests::config;
use crate::secrets::test_support::MemorySecrets;
use crate::secrets::{SecretStore, SecretStoreError, SecretValue};
use crate::tools::{Grant, Principal, RequestContext};
use crate::{Caller, CoreEvent, Draft, ErrorCode, MailxCore};

struct Fixture {
    _directory: TempDir,
    core: MailxCore,
    db: Connection,
    account: String,
    events: Arc<Mutex<Vec<CoreEvent>>>,
}

fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn fixture() -> Fixture {
    let directory = TempDir::new().unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let (core, emails) = MailxCore::body_test_fixture_with_events(
        directory.path(),
        tokio::runtime::Handle::current(),
        &[b"Content-Type: text/plain\r\n\r\nSynthetic"],
        Arc::new(move |event: &CoreEvent| sink.lock().unwrap().push(event.clone())),
    )
    .unwrap();
    let account = core.get_email(&emails[0]).unwrap().summary.account_id;
    let db = Connection::open(directory.path().join("mailx.sqlite3")).unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    Fixture {
        _directory: directory,
        core,
        db,
        account,
        events,
    }
}

impl Fixture {
    fn draft(&self, account: &str) -> Draft {
        self.core.create_draft(serde_json::from_value(json!({
            "accountId": account, "to": [{"address":"recipient@example.test"}],
            "cc": [], "bcc": [], "subject":"Synthetic send", "bodyHtml":"<p>Frozen body</p>",
            "clientMutationId": id()
        })).unwrap(), "").unwrap()
    }

    fn other_account(&self) -> String {
        let other = id();
        self.db.execute(
            "INSERT INTO accounts(id, revision, display_name, email_address, mailbox_source_id, created_at, updated_at)
             VALUES (?1, 1, 'Other', 'other@example.test', ?2, '2030-01-01T00:00:00.000Z', '2030-01-01T00:00:00.000Z')",
            params![other, id()],
        ).unwrap();
        other
    }

    fn tools(&self) -> Tools {
        Tools::new(self.core.clone(), String::new).with_undo_send_seconds(|| Ok(30))
    }

    fn queue(&self, tools: &Tools, ctx: &RequestContext, draft: &Draft) -> (Value, Value) {
        let input = send_input(draft);
        let result = tools.call(ctx, "send_email", input.clone()).unwrap();
        (input, result)
    }
}

fn contexts() -> [RequestContext; 2] {
    [
        Principal::LocalMcp {
            connection_id: "test-connection".into(),
            client_name: None,
        },
        Principal::BuiltinAgent {
            run_id: "test-run".into(),
        },
    ]
    .map(|principal| RequestContext::new(principal, Grant::mcp_default(), "test-request"))
}

fn send_input(draft: &Draft) -> Value {
    json!({"action":"send", "draftId":draft.id, "expectedRevision":draft.revision, "clientMutationId":id()})
}

fn error(tools: &Tools, ctx: &RequestContext, tool: &str, input: Value) -> ToolErrorCode {
    tools.call(ctx, tool, input).unwrap_err().code
}

#[tokio::test]
async fn both_audiences_send_frozen_drafts_and_replay_without_duplicate_items() {
    for ctx in contexts() {
        let f = fixture();
        let tools = f.tools();
        let draft = f.draft(&f.account);
        f.events.lock().unwrap().clear();
        let (input, result) = f.queue(&tools, &ctx, &draft);
        assert_eq!(result["state"], "queued");
        assert_eq!(result["draftId"], draft.id);
        let queued = f.core.get_outbox(result["id"].as_str().unwrap()).unwrap();
        let not_before = chrono::DateTime::parse_from_rfc3339(&queued.not_before).unwrap();
        let created = chrono::DateTime::parse_from_rfc3339(&queued.created_at).unwrap();
        assert_eq!((not_before - created).num_seconds(), 30);
        let bytes = f.core.outbox_mime_for_tests(&queued.id);
        let mime = String::from_utf8_lossy(&bytes);
        assert!(mime.contains("Frozen body"));
        let replay = tools.call(&ctx, "send_email", input).unwrap();
        assert_eq!(replay["id"], result["id"]);
        assert_eq!(f.core.list_outbox(None, true).unwrap().len(), 1);
        let events = f.events.lock().unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, CoreEvent::Outbox(_)))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, CoreEvent::DraftChanged(_)))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn send_validates_exact_action_fields_revision_and_mutation() {
    let f = fixture();
    let tools = f.tools();
    let ctx = &contexts()[0];
    let draft = f.draft(&f.account);
    for input in [
        json!({"action":"send"}),
        json!({"action":"send","draftId":draft.id,"expectedRevision":draft.revision}),
        json!({"action":"send","draftId":draft.id,"expectedRevision":draft.revision,"clientMutationId":"bad"}),
        json!({"action":"retry"}),
        json!({"action":"retry","outboxId":id(),"draftId":draft.id}),
        json!({"action":"retry","outboxId":id(),"draftId":null}),
        json!({"action":"retry","outboxId":id(),"expectedRevision":1}),
        json!({"action":"retry","outboxId":id(),"clientMutationId":id()}),
        json!({"action":"retry","outboxId":id(),"confirmed":true}),
        json!({"action":"retry","outboxId":id(),"caller":"Composer"}),
    ] {
        assert_eq!(
            error(&tools, ctx, "send_email", input),
            ToolErrorCode::InvalidInput
        );
    }
    let mut mixed = send_input(&draft);
    mixed["outboxId"] = json!(id());
    assert_eq!(
        error(&tools, ctx, "send_email", mixed),
        ToolErrorCode::InvalidInput
    );
    let mut stale = send_input(&draft);
    stale["expectedRevision"] = json!(draft.revision + 1);
    assert_eq!(
        error(&tools, ctx, "send_email", stale),
        ToolErrorCode::Conflict
    );
    assert!(f.core.list_outbox(None, true).unwrap().is_empty());
    let (mut input, _) = f.queue(&tools, ctx, &draft);
    input["expectedRevision"] = json!(draft.revision + 1);
    assert_eq!(
        error(&tools, ctx, "send_email", input),
        ToolErrorCode::InvalidInput
    );
    assert_eq!(f.core.list_outbox(None, true).unwrap().len(), 1);
}

#[tokio::test]
async fn replay_survives_draft_deletion_and_never_leaks_another_accounts_mutation() {
    let f = fixture();
    let tools = f.tools();
    let ctx = &contexts()[0];
    let draft = f.draft(&f.account);
    let (input, result) = f.queue(&tools, ctx, &draft);
    // Simulate accepted queue storage: its draft has been consumed.
    f.db.execute(
        "UPDATE outbox SET state='accepted', accepted_at=created_at, draft_id=NULL WHERE id=?1",
        [result["id"].as_str().unwrap()],
    )
    .unwrap();
    f.db.execute("DELETE FROM drafts WHERE id=?1", [&draft.id])
        .unwrap();
    let replay = tools.call(ctx, "send_email", input.clone()).unwrap();
    assert_eq!(replay["id"], result["id"]);
    assert_eq!(replay["state"], "accepted");
    let other = f.other_account();
    let mut restricted = ctx.clone();
    restricted.grant.accounts = AccountScope::Only(vec![other.clone()]);
    assert_eq!(
        error(&tools, &restricted, "send_email", input.clone()),
        ToolErrorCode::NotFound
    );
    let mut clash = input;
    clash["draftId"] = json!(f.draft(&other).id);
    assert_eq!(
        error(&tools, &restricted, "send_email", clash),
        ToolErrorCode::NotFound
    );
    assert_eq!(f.core.list_outbox(None, true).unwrap().len(), 1);
}

#[tokio::test]
async fn sending_checks_account_scope_open_editors_and_current_undo_setting() {
    let f = fixture();
    let ctx = &contexts()[0];
    let draft = f.draft(&f.account);
    let mut outside = ctx.clone();
    outside.grant.accounts = AccountScope::Only(vec![f.other_account()]);
    assert_eq!(
        error(&f.tools(), &outside, "send_email", send_input(&draft)),
        ToolErrorCode::NotFound
    );
    let open = f.tools().with_draft_open_check(|_| true);
    assert_eq!(
        error(&open, ctx, "send_email", send_input(&draft)),
        ToolErrorCode::DraftOpen
    );
    let broken = f
        .tools()
        .with_undo_send_seconds(|| Err(ToolError::new(ToolErrorCode::Unavailable)));
    assert_eq!(
        error(&broken, ctx, "send_email", send_input(&draft)),
        ToolErrorCode::Unavailable
    );
    assert!(f.core.list_outbox(None, true).unwrap().is_empty());
    let seconds = Arc::new(Mutex::new(5));
    let read = seconds.clone();
    let tools = f
        .tools()
        .with_undo_send_seconds(move || Ok(*read.lock().unwrap()));
    for value in [5, 20] {
        *seconds.lock().unwrap() = value;
        let (_, item) = f.queue(&tools, ctx, &f.draft(&f.account));
        let from =
            chrono::DateTime::parse_from_rfc3339(item["createdAt"].as_str().unwrap()).unwrap();
        let until =
            chrono::DateTime::parse_from_rfc3339(item["notBefore"].as_str().unwrap()).unwrap();
        assert_eq!((until - from).num_seconds(), i64::from(value));
    }
}

#[tokio::test]
async fn retry_and_cancel_obey_queue_state_scope_and_emit_changes() {
    let f = fixture();
    let tools = f.tools();
    let ctx = &contexts()[0];
    let draft = f.draft(&f.account);
    let (_, item) = f.queue(&tools, ctx, &draft);
    let id = item["id"].as_str().unwrap();
    f.db.execute(
        "UPDATE outbox SET next_attempt_at='2099-01-01T00:00:00.000Z' WHERE id=?1",
        [id],
    )
    .unwrap();
    f.events.lock().unwrap().clear();
    let retry = tools
        .call(ctx, "send_email", json!({"action":"retry","outboxId":id}))
        .unwrap();
    assert_eq!(retry["id"], id);
    assert_eq!(retry["notBefore"], item["notBefore"]);
    assert!(retry.get("nextAttemptAt").is_none());
    assert!(
        f.events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, CoreEvent::Outbox(_)))
    );
    let mut outside = ctx.clone();
    outside.grant.accounts = AccountScope::Only(vec![f.other_account()]);
    for (tool, input) in [
        ("get_outbox", json!({"outboxId":id})),
        ("cancel_outbox", json!({"outboxId":id})),
        ("send_email", json!({"action":"retry","outboxId":id})),
    ] {
        assert_eq!(
            error(&tools, &outside, tool, input),
            ToolErrorCode::NotFound
        );
    }
    for state in ["failed", "unknown", "accepted", "sending"] {
        f.db.execute("UPDATE outbox SET state=?2, accepted_at=CASE WHEN ?2='accepted' THEN created_at ELSE NULL END WHERE id=?1", params![id, state])
            .unwrap();
        let expected = if state == "sending" {
            ToolErrorCode::AlreadySending
        } else {
            ToolErrorCode::Conflict
        };
        assert_eq!(
            error(
                &tools,
                ctx,
                "send_email",
                json!({"action":"retry","outboxId":id})
            ),
            expected
        );
        assert_eq!(
            error(&tools, ctx, "cancel_outbox", json!({"outboxId":id})),
            expected
        );
    }
    f.db.execute("UPDATE outbox SET state='queued' WHERE id=?1", [id])
        .unwrap();
    let canceled = tools
        .call(ctx, "cancel_outbox", json!({"outboxId":id}))
        .unwrap();
    assert_eq!(canceled["state"], "canceled");
    assert!(!f.core.draft_locked(&draft.id).unwrap());
    assert_eq!(f.core.get_draft(&draft.id).unwrap().id, draft.id);
}

#[tokio::test]
async fn outbox_pages_filter_scope_and_state_before_limit_and_bind_cursors() {
    let f = fixture();
    let tools = f.tools();
    let ctx = &contexts()[0];
    let other = f.other_account();
    let mut restricted = ctx.clone();
    restricted.grant.accounts = AccountScope::Only(vec![f.account.clone()]);
    let mut expected = Vec::new();
    for (index, account) in [&f.account, &other, &f.account, &other, &f.account, &other]
        .into_iter()
        .enumerate()
    {
        let (_, result) = f.queue(&tools, ctx, &f.draft(account));
        let id = result["id"].as_str().unwrap().to_owned();
        f.db.execute(
            "UPDATE outbox SET created_at=?2 WHERE id=?1",
            params![id, format!("2030-01-01T00:00:{index:02}.000Z")],
        )
        .unwrap();
        if account == &f.account {
            expected.push(id);
        }
    }
    expected.reverse();
    let first = tools
        .call(&restricted, "list_outbox", json!({"limit":2}))
        .unwrap();
    let got: Vec<_> = first["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect();
    assert_eq!(got, expected[..2]);
    let cursor = first["nextCursor"].as_str().unwrap();
    let next = tools
        .call(
            &restricted,
            "list_outbox",
            json!({"limit":2,"cursor":cursor}),
        )
        .unwrap();
    assert_eq!(next["items"][0]["id"], expected[2]);
    assert!(next.get("nextCursor").is_none());
    assert_eq!(
        error(
            &tools,
            ctx,
            "list_outbox",
            json!({"limit":2,"cursor":cursor})
        ),
        ToolErrorCode::InvalidInput
    );
    assert_eq!(
        error(
            &tools,
            &restricted,
            "list_outbox",
            json!({"states":["accepted"],"cursor":cursor})
        ),
        ToolErrorCode::InvalidInput
    );
    f.db.execute(
        "UPDATE outbox SET state='accepted', accepted_at=created_at WHERE id=?1",
        [&expected[0]],
    )
    .unwrap();
    let accepted = tools
        .call(
            &restricted,
            "list_outbox",
            json!({"states":["accepted"],"limit":1}),
        )
        .unwrap();
    assert_eq!(accepted["items"][0]["id"], expected[0]);
    let active = tools.call(&restricted, "list_outbox", json!({})).unwrap();
    assert_eq!(active["items"].as_array().unwrap().len(), 2);
    for bad in [
        json!({"limit":0}),
        json!({"limit":101}),
        json!({"states":[]}),
        json!({"states":["invalid"]}),
        json!({"cursor":"bad"}),
    ] {
        assert_eq!(
            error(&tools, ctx, "list_outbox", bad),
            ToolErrorCode::InvalidInput
        );
    }
}

#[tokio::test]
async fn account_outbox_queries_work_when_the_secret_store_is_unavailable() {
    #[derive(Default)]
    struct SwitchableSecrets {
        values: MemorySecrets,
        failure: Mutex<Option<SecretStoreError>>,
    }

    impl SecretStore for SwitchableSecrets {
        fn get(&self, key: &str) -> Result<Option<SecretValue>, SecretStoreError> {
            if let Some(error) = *self.failure.lock().unwrap() {
                return Err(error);
            }
            self.values.get(key)
        }

        fn set(&self, key: &str, value: &SecretValue) -> Result<(), SecretStoreError> {
            self.values.set(key, value)
        }

        fn delete(&self, key: &str) -> Result<(), SecretStoreError> {
            self.values.delete(key)
        }
    }

    let directory = TempDir::new().unwrap();
    let secrets = Arc::new(SwitchableSecrets::default());
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let core = MailxCore::open(
        directory.path(),
        secrets.clone(),
        tokio::runtime::Handle::current(),
        Arc::new(move |event: &CoreEvent| sink.lock().unwrap().push(event.clone())),
    )
    .unwrap();
    core.begin_shutdown();
    // Seed the service directly so no sync or SMTP worker starts for this account.
    let account = core.with_accounts_for_test(|service| {
        service
            .create(
                serde_json::from_value(json!({
                    "requestId": id(), "config": config(json!({"kind":"direct"}))
                }))
                .unwrap(),
            )
            .unwrap()
            .id
    });
    let db = Connection::open(directory.path().join("mailx.sqlite3")).unwrap();
    let f = Fixture {
        _directory: directory,
        core,
        db,
        account,
        events,
    };
    let tools = f.tools();
    let ctx = &contexts()[0];
    let (_, item) = f.queue(&tools, ctx, &f.draft(&f.account));
    let input = json!({"accountId":f.account});
    assert_eq!(
        tools.call(ctx, "list_outbox", input.clone()).unwrap()["items"][0]["id"],
        item["id"]
    );
    f.db.execute(
        "UPDATE outbox SET state='failed',last_error=?2 WHERE id=?1",
        params![
            item["id"].as_str().unwrap(),
            json!({"reason":"secretStore","rejected":[]}).to_string()
        ],
    )
    .unwrap();
    for (failure, code) in [
        (
            SecretStoreError::Unavailable,
            ErrorCode::SecretStoreUnavailable,
        ),
        (SecretStoreError::Locked, ErrorCode::SecretStoreLocked),
    ] {
        *secrets.failure.lock().unwrap() = Some(failure);
        assert_eq!(f.core.get_account(&f.account).unwrap_err().code, code);
        let result = tools.call(ctx, "list_outbox", input.clone()).unwrap();
        assert_eq!(result["items"].as_array().unwrap().len(), 1);
        assert_eq!(result["items"][0]["id"], item["id"]);
        assert_eq!(result["items"][0]["error"]["reason"], "secretStore");
        assert_eq!(
            error(&tools, ctx, "list_outbox", json!({"accountId":id()})),
            ToolErrorCode::NotFound
        );
        let mut restricted = ctx.clone();
        restricted.grant.accounts = AccountScope::Only(vec![]);
        assert_eq!(
            error(&tools, &restricted, "list_outbox", input.clone()),
            ToolErrorCode::NotFound
        );
    }
}

#[tokio::test]
async fn outbox_results_hide_server_text_and_require_their_own_capabilities() {
    let f = fixture();
    let tools = f.tools();
    let ctx = &contexts()[0];
    let (_, item) = f.queue(&tools, ctx, &f.draft(&f.account));
    let id = item["id"].as_str().unwrap();
    f.db.execute("UPDATE outbox SET last_error=?2,server_reply=?3 WHERE id=?1",params![id,
        json!({"reason":"authRejected","smtpCode":535,"text":"SECRET","rejected":[{"address":"a@x.test","code":550,"text":"SECRET"}]}).to_string(),
        json!({"code":250,"text":"SECRET"}).to_string()]).unwrap();
    let result = tools
        .call(ctx, "get_outbox", json!({"outboxId":id}))
        .unwrap();
    assert_eq!(
        result["error"],
        json!({"reason":"authRejected","smtpCode":535})
    );
    assert!(!result.to_string().contains("SECRET"));
    assert!(result.get("serverReply").is_none());
    for (tool, input, action) in [
        (
            "send_email",
            json!({"action":"retry","outboxId":id}),
            Action::MailSend,
        ),
        ("get_outbox", json!({"outboxId":id}), Action::OutboxRead),
        ("list_outbox", json!({}), Action::OutboxRead),
        ("cancel_outbox", json!({"outboxId":id}), Action::OutboxWrite),
    ] {
        let mut denied = ctx.clone();
        denied.grant.actions.remove(&action);
        assert_eq!(
            error(&tools, &denied, tool, input),
            ToolErrorCode::NotPermitted
        );
    }
}

#[tokio::test]
async fn parallel_sends_with_the_same_mutation_commit_once() {
    let f = fixture();
    let draft = f.draft(&f.account);
    let request: QueueSendRequest = serde_json::from_value(
        json!({"draftId":draft.id,"expectedRevision":draft.revision,"clientMutationId":id()}),
    )
    .unwrap();
    // Force both snapshots to be prepared before either is committed.
    let (first, second) = f.core.with_accounts_for_test(|service| {
        let prepare =
            |service: &crate::AccountService| match service.prepare_send(&request, 30).unwrap() {
                crate::accounts::outbox::SendPlan::Build(build) => build,
                _ => panic!("unexpected replay"),
            };
        (prepare(service), prepare(service))
    });
    let first_bytes = first.freeze().unwrap();
    let second_bytes = second.freeze().unwrap();
    let first_item = f.core.with_accounts_for_test(|service| {
        service
            .commit_send(&request, 30, &first, first_bytes, chrono::Utc::now())
            .unwrap()
            .0
    });
    let (second_item, event) = f.core.with_accounts_for_test(|service| {
        service
            .commit_send(&request, 30, &second, second_bytes, chrono::Utc::now())
            .unwrap()
    });
    assert_eq!(first_item.id, second_item.id);
    assert!(event.is_none());
    assert_eq!(f.core.list_outbox(None, true).unwrap().len(), 1);
    assert_eq!(
        f.core
            .queue_send(Caller::MainWindow, request, 30)
            .unwrap_err()
            .code,
        ErrorCode::Forbidden
    );
}
