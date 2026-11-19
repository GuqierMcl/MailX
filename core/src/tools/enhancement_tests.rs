use super::*;

#[tokio::test]
async fn search_sender_is_exact_case_insensitive_and_returns_recipients_and_hit_text() {
    let f = fx();
    f.db.execute(
        "UPDATE messages SET from_address = 'sender@example.test', to_addresses = ?1 WHERE id = ?2",
        params![
            r#"[{"address":"recipient@example.test","name":"Recipient"}]"#,
            f.emails[0]
        ],
    )
    .unwrap();
    let result = f
        .call(
            &mcp(),
            "mail_search",
            json!({"fromAddress":"SENDER@example.test", "query":"world"}),
        )
        .unwrap();
    let hits = result["emails"].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["id"], f.emails[0]);
    assert_eq!(
        hits[0]["to"]["addresses"][0]["address"],
        "recipient@example.test"
    );
    assert_eq!(hits[0]["to"]["total"], 1);
    assert!(
        hits[0]["matchSnippet"]
            .as_array()
            .unwrap()
            .iter()
            .any(|part| part["hit"] == true)
    );
    assert_eq!(
        f.call(
            &mcp(),
            "mail_search",
            json!({"fromAddress":"sender@example"})
        )
        .unwrap()["emails"],
        json!([])
    );
    for bad in ["", "\n@example.test"] {
        assert_eq!(
            code(f.call(&mcp(), "mail_search", json!({"fromAddress":bad}))),
            ToolErrorCode::InvalidInput
        );
    }
}

#[tokio::test]
async fn draft_creation_revision_can_be_used_immediately_and_pagination_is_filter_bound() {
    let f = fx();
    let other = f.add_account();
    let first = user_draft(&f, &f.account, "Match older");
    let second = user_draft(&f, &f.account, "Match newer");
    let hidden = user_draft(&f, &other, "Match hidden");
    for (id, time) in [
        (&first.id, "2026-11-01T00:00:00.000Z"),
        (&second.id, "2026-11-02T00:00:00.000Z"),
        (&hidden.id, "2026-11-03T00:00:00.000Z"),
    ] {
        f.db.execute(
            "UPDATE drafts SET updated_at = ?1 WHERE id = ?2",
            params![time, id],
        )
        .unwrap();
    }
    let scoped = with_grant(
        mcp(),
        Grant::new(
            AccountScope::Only(vec![f.account.clone()]),
            [Action::MailRead],
        ),
    );
    let page = f
        .call(
            &scoped,
            "draft_list",
            json!({"limit":1,"origin":"user","query":"Match"}),
        )
        .unwrap();
    assert_eq!(
        draft_ids(&page).as_slice(),
        std::slice::from_ref(&second.id)
    );
    assert_eq!(page["truncated"], true);
    let next = f
        .call(
            &scoped,
            "draft_list",
            json!({"limit":1,"origin":"user","query":"Match","cursor":page["nextCursor"]}),
        )
        .unwrap();
    assert_eq!(draft_ids(&next), [first.id]);
    assert_eq!(next["truncated"], false);
    assert!(next.get("nextCursor").is_none());
    assert_eq!(
        code(f.call(
            &scoped,
            "draft_list",
            json!({"origin":"ai","query":"Match","cursor":page["nextCursor"]})
        )),
        ToolErrorCode::InvalidInput
    );
    let created = f
        .call(&mcp(), "draft_create", draft_input(json!({})))
        .unwrap();
    assert!(created["revision"].as_u64().is_some());
    let updated = f.call(&mcp(), "draft_update", json!({"draftId":created["draftId"],"expectedRevision":created["revision"],"subject":"Immediate update"})).unwrap();
    assert_eq!(
        updated["revision"].as_u64(),
        created["revision"].as_u64().map(|r| r + 1)
    );
}

#[tokio::test]
async fn conversation_paginates_after_copy_selection_and_checks_seed_scope() {
    let f = fx();
    let conversation = Uuid::new_v4().to_string();
    for (index, id) in f.emails.iter().enumerate() {
        f.db.execute("UPDATE messages SET conversation_id=?1, message_id_header=?2, received_at=?3 WHERE id=?4",
            params![conversation, if index < 2 {"<same@example.test>"} else {"<different@example.test>"}, format!("2026-11-0{}T00:00:00.000Z", index+1), id]).unwrap();
    }
    let input = json!({"emailId":f.emails[1],"limit":1});
    let first = f.call(&mcp(), "mail_conversation", input.clone()).unwrap();
    // Prefer the seed copy over the earlier duplicate before applying LIMIT.
    assert_eq!(first["emails"][0]["id"], f.emails[1]);
    let second = f
        .call(
            &mcp(),
            "mail_conversation",
            json!({"emailId":f.emails[1],"limit":1,"cursor":first["nextCursor"]}),
        )
        .unwrap();
    assert_eq!(second["emails"][0]["id"], f.emails[2]);
    assert!(second.get("nextCursor").is_none());
    assert!(second["possiblyIncomplete"].is_boolean());
    assert_eq!(
        code(f.call(
            &mcp(),
            "mail_conversation",
            json!({"emailId":f.emails[2],"cursor":first["nextCursor"]})
        )),
        ToolErrorCode::InvalidInput
    );
    let denied = with_grant(
        mcp(),
        Grant::new(AccountScope::Only(vec![]), [Action::MailRead]),
    );
    assert_eq!(
        code(f.call(&denied, "mail_conversation", input)),
        ToolErrorCode::NotFound
    );
}

#[tokio::test]
async fn recipient_tool_enforces_scope_before_contact_lookup() {
    let f = fx();
    f.db.execute(
        "UPDATE messages SET from_address='alice@example.test', from_name='Alice'",
        [],
    )
    .unwrap();
    let result = f
        .call(&mcp(), "mail_contacts", json!({"query":"ali"}))
        .unwrap();
    assert_eq!(result["contacts"][0]["address"], "alice@example.test");
    assert_eq!(result["accountIds"], json!([f.account]));
    let denied = with_grant(
        mcp(),
        Grant::new(AccountScope::Only(vec![]), [Action::MailRead]),
    );
    let hidden = f
        .call(&denied, "mail_contacts", json!({"query":"ali"}))
        .unwrap();
    assert_eq!(hidden["contacts"], json!([]));
    assert_eq!(hidden["accountIds"], json!([]));
    assert_eq!(
        code(f.call(
            &denied,
            "mail_contacts",
            json!({"query":"ali","accountId":f.account})
        )),
        ToolErrorCode::NotFound
    );
}

#[tokio::test]
async fn conversation_never_includes_another_accounts_identical_thread_id() {
    let f = fx();
    let other = f.add_account();
    let folder = Uuid::new_v4().to_string();
    f.db.execute("INSERT INTO folders(id, account_id, mailbox_source_id, remote_name, parent_id, name, delimiter, role, selectable, attributes, unread_count, total_count, uid_next, uid_validity, synced_at)
        SELECT ?1, ?2, (SELECT mailbox_source_id FROM accounts WHERE id=?2), remote_name, NULL, name, delimiter, role, selectable, attributes, 0, 0, uid_next, uid_validity, synced_at
        FROM folders WHERE role='inbox'", params![folder, other]).unwrap();
    f.db.execute("INSERT INTO folder_sync_cursors(folder_id, uid_validity, window_start, window_capped, backfill_done, highest_uid, updated_at)
        SELECT ?1, uid_validity, window_start, window_capped, backfill_done, highest_uid, updated_at FROM folder_sync_cursors WHERE folder_id=(SELECT id FROM folders WHERE account_id=?2 AND role='inbox')",
        params![folder, f.account]).unwrap();
    let thread = Uuid::new_v4().to_string();
    f.db.execute("UPDATE messages SET conversation_id=?1", [&thread])
        .unwrap();
    f.db.execute("UPDATE messages SET account_id=?1, mailbox_source_id=(SELECT mailbox_source_id FROM accounts WHERE id=?1), folder_id=?2 WHERE id=?3", params![other, folder, f.emails[2]]).unwrap();
    let ctx = with_grant(
        mcp(),
        Grant::new(
            AccountScope::Only(vec![f.account.clone()]),
            [Action::MailRead],
        ),
    );
    let result = f
        .call(&ctx, "mail_conversation", json!({"emailId":f.emails[0]}))
        .unwrap();
    let hits = result["emails"].as_array().unwrap();
    assert_eq!(hits.len(), 2);
    assert!(hits.iter().all(|email| email["accountId"] == f.account));
    assert!(!hits.iter().any(|email| email["id"] == f.emails[2]));
}
