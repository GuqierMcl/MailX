use mailx_core::{CreateDraftRequest, Draft, UpdateDraftRequest};

use super::*;

fn create(core: &MailxCore, html: &str) -> Draft {
    core.create_draft(
        CreateDraftRequest {
            account_id: None,
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: "Draft preview fixture".into(),
            body_html: html.into(),
            client_mutation_id: Uuid::new_v4().to_string(),
        },
        "",
    )
    .unwrap()
}

fn update(core: &MailxCore, draft: &Draft, html: &str) -> Draft {
    core.update_draft(UpdateDraftRequest {
        id: draft.id.clone(),
        expected_revision: draft.revision,
        account_id: None,
        to: draft.to.clone(),
        cc: draft.cc.clone(),
        bcc: draft.bcc.clone(),
        subject: draft.subject.clone(),
        body_html: html.into(),
    })
    .unwrap()
}

fn with_image(core: &MailxCore) -> Draft {
    let draft = create(core, "<p>Draft image</p>");
    let image = render::inline_image(&mime(), 2, "pixel").unwrap();
    let (draft, images) = core
        .add_inline_images(
            &draft.id,
            draft.revision,
            vec![("pixel.png".into(), image.bytes)],
        )
        .unwrap();
    update(
        core,
        &draft,
        &format!(
            "<p>Draft image</p><img src=\"cid:{}\" alt=\"Draft pixel\"><a href=\"https://example.invalid/draft\">Link</a>",
            images[0].content_id,
        ),
    )
}

async fn prepare(protocol: &BodyProtocol, core: &MailxCore, draft: &Draft) -> (String, String) {
    let token = protocol
        .prepare_draft(core, draft.id.clone(), draft.revision)
        .await
        .unwrap()
        .document_token;
    let url = format!("{BODY_ORIGIN}/document/{}?token={token}", draft.id);
    (url, token)
}

async fn html(protocol: &BodyProtocol, core: &MailxCore, url: &str) -> String {
    let (response, event) = protocol
        .serve(core, request(Method::GET, url), APP_ORIGIN)
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(event.is_none());
    assert!(
        !response
            .headers()
            .contains_key("Access-Control-Allow-Origin")
    );
    String::from_utf8(response.into_body()).unwrap()
}

#[tokio::test]
async fn draft_rich_body_reuses_passive_mail_rendering_and_dark_theme() {
    let (_directory, core, _, _db) = fixture();
    let draft = create(
        &core,
        "<h1 data-align=\"center\">Title</h1><p><strong>Bold</strong> <em>italic</em> <span data-color=\"red\">color</span> <mark data-highlight=\"yellow\">highlight</mark></p><ul><li>Item</li></ul><blockquote>Quote</blockquote><script>evil()</script><img src=\"https://example.invalid/tracker\"><a href=\"javascript:evil()\">Unsafe</a>",
    );
    let protocol = BodyProtocol::default();
    let (url, token) = prepare(&protocol, &core, &draft).await;
    let html = html(&protocol, &core, &format!("{url}&scheme=dark")).await;
    for expected in [
        "<h1",
        "text-align:center",
        "<strong>Bold</strong>",
        "<em>italic</em>",
        "color:#dc2626",
        "background-color:#fef08a",
        "<ul ",
        "<blockquote ",
        DARK_CSS,
    ] {
        assert!(html.contains(expected), "missing {expected}: {html}");
    }
    for forbidden in [
        "<script",
        "evil()",
        "https://example.invalid/tracker",
        "javascript:",
    ] {
        assert!(!html.contains(forbidden));
    }
    assert!(protocol.prepare(&core, draft.id.clone()).await.is_err());
    assert!(
        protocol
            .prepare_draft(&core, draft.id.clone(), draft.revision + 1)
            .await
            .is_err_and(|error| error.code == ErrorCode::Conflict)
    );
    assert!(
        protocol
            .image_status(
                &core,
                &draft.id,
                &token,
                mailx_core::RemoteImagePolicy::Automatic
            )
            .await
            .is_err()
    );
    let remote = resource_url(BODY_ORIGIN, "remote", &draft.id, "0", &token);
    let (response, _) = protocol
        .serve(&core, request(Method::GET, &remote), APP_ORIGIN)
        .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn draft_images_and_links_are_bound_to_one_source_and_current_revision() {
    let (_directory, core, email_ids, _db) = fixture();
    let draft = with_image(&core);
    let another = create(&core, "<p>Another draft</p>");
    let protocol = BodyProtocol::default();
    let (url, token) = prepare(&protocol, &core, &draft).await;
    let document = html(&protocol, &core, &url).await;
    let part = attribute(&document, "src");
    let link = attribute(&document, "href");
    let (image, _) = protocol
        .serve(&core, request(Method::GET, &part), APP_ORIGIN)
        .await;
    assert_eq!(image.status(), StatusCode::OK);
    response_headers(&image, "image/png", APP_ORIGIN, None);
    assert!(image.body().starts_with(b"\x89PNG\r\n\x1a\n"));
    for other in [&another.id, &email_ids[0]] {
        for route in [&url, &part, &link] {
            let foreign = route.replace(&draft.id, other);
            let (response, event) = protocol
                .serve(&core, request(Method::GET, &foreign), APP_ORIGIN)
                .await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
            assert!(event.is_none());
        }
    }
    let (_, event) = protocol
        .serve(&core, request(Method::GET, &link), APP_ORIGIN)
        .await;
    let event = event.unwrap();
    assert_eq!(event.url, "https://example.invalid/draft");
    let edited = update(
        &core,
        &draft,
        "<p>Saved again</p><a href=\"https://example.invalid/new\">New</a>",
    );
    for route in [&url, &part, &link] {
        let (response, event) = protocol
            .serve(&core, request(Method::GET, route), APP_ORIGIN)
            .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(event.is_none());
    }
    assert!(
        protocol
            .confirm_link(&core, draft.id.clone(), 0, token, event.request_id)
            .await
            .is_err()
    );
    let (fresh, _) = prepare(&protocol, &core, &edited).await;
    assert!(html(&protocol, &core, &fresh).await.contains("Saved again"));
}

#[tokio::test]
async fn draft_link_confirmations_are_one_shot_and_release_is_per_reading() {
    let (_directory, core, _, _db) = fixture();
    let draft = create(&core, "<a href=\"mailto:writer@example.invalid\">Write</a>");
    let protocol = BodyProtocol::default();
    let (url, token) = prepare(&protocol, &core, &draft).await;
    let (new_url, _) = prepare(&protocol, &core, &draft).await;
    let document = html(&protocol, &core, &url).await;
    let link = attribute(&document, "href");
    let (_, event) = protocol
        .serve(&core, request(Method::GET, &link), APP_ORIGIN)
        .await;
    let event = event.unwrap();
    assert_eq!(
        protocol
            .confirm_link(
                &core,
                draft.id.clone(),
                0,
                token.clone(),
                event.request_id.clone()
            )
            .await
            .unwrap(),
        "mailto:writer@example.invalid"
    );
    assert!(
        protocol
            .confirm_link(&core, draft.id.clone(), 0, token.clone(), event.request_id)
            .await
            .is_err()
    );
    protocol.release(&draft.id, &token).unwrap();
    let (response, _) = protocol
        .serve(&core, request(Method::GET, &url), APP_ORIGIN)
        .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(html(&protocol, &core, &new_url).await.contains("Write"));
    core.delete_draft(&draft.id).unwrap();
    let (response, _) = protocol
        .serve(&core, request(Method::GET, &new_url), APP_ORIGIN)
        .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn missing_draft_images_do_not_block_the_body_and_account_edits_revoke_it() {
    let (_directory, core, _, db) = fixture();
    let draft = create(
        &core,
        "<p>Still readable</p><img src=\"cid:0123456789abcdef0123456789abcdef@example.invalid\" alt=\"Missing image\">",
    );
    let protocol = BodyProtocol::default();
    let (url, _) = prepare(&protocol, &core, &draft).await;
    let document = html(&protocol, &core, &url).await;
    assert!(document.contains("Still readable") && document.contains("[Missing image]"));
    assert!(!document.contains("<img"));
    db.execute(
        "UPDATE accounts SET revision = revision + 1 WHERE id = ?1",
        [&draft.account_id],
    )
    .unwrap();
    let (response, _) = protocol
        .serve(&core, request(Method::GET, &url), APP_ORIGIN)
        .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn legacy_plain_drafts_render_escaped_lines_and_removed_accounts_revoke_resources() {
    let (_directory, core, _, db) = fixture();
    let draft = create(&core, "<p>Initial</p>");
    db.execute(
        "UPDATE drafts SET body_html = NULL, body_text = ?2 WHERE id = ?1",
        [&draft.id, "Legacy <script>text</script>\nSecond line"],
    )
    .unwrap();
    let protocol = BodyProtocol::default();
    let (url, _) = prepare(&protocol, &core, &draft).await;
    let document = html(&protocol, &core, &url).await;
    assert!(
        document.contains("Legacy &lt;script&gt;text&lt;/script&gt;")
            && document.contains("Second line")
    );
    assert!(!document.contains("<script>"));
    // The account check also rejects an orphaned draft if cascade cleanup is
    // interrupted or this external test connection has foreign keys disabled.
    db.execute("DELETE FROM accounts WHERE id = ?1", [&draft.account_id])
        .unwrap();
    let (response, _) = protocol
        .serve(&core, request(Method::GET, &url), APP_ORIGIN)
        .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[test]
fn draft_body_prepare_is_main_only_and_never_grants_composer_image_access() {
    let draft_id = Uuid::new_v4();
    let app = Url::parse("tauri://localhost/index.html").unwrap();
    assert!(crate::window_access::check("main", &app, "drafts_prepare_body", None).is_ok());
    assert!(
        crate::window_access::check(
            &crate::window_access::composer_label(&draft_id),
            &app,
            "drafts_prepare_body",
            None
        )
        .is_err()
    );
    assert!(
        crate::window_access::check(
            "main",
            &app,
            "inline_image_preview",
            Some(&draft_id.to_string())
        )
        .is_err()
    );
    assert!(
        crate::window_access::check(
            "main",
            &Url::parse("mailx-body://localhost").unwrap(),
            "drafts_prepare_body",
            None
        )
        .is_err()
    );
}

#[tokio::test]
async fn draft_documents_never_fold_a_trailing_quote() {
    let (_directory, core, _, _db) = fixture();
    let draft = create(
        &core,
        "<p>Reply</p><p>On 2026-11-03, Bob wrote:</p><blockquote><p>Quoted</p></blockquote>",
    );
    let protocol = BodyProtocol::default();
    let (url, _) = prepare(&protocol, &core, &draft).await;
    let html = html(&protocol, &core, &url).await;
    assert!(html.contains("Quoted") && html.contains("<blockquote "));
    assert!(!html.contains("<details") && !html.contains("<summary"));
}
