use super::*;

const TARGET: &str = "zh-Hans";

async fn serve_document(
    protocol: &BodyProtocol,
    core: &MailxCore,
    id: &str,
    token: &str,
) -> (StatusCode, String) {
    let url = format!("{BODY_ORIGIN}/document/{id}?token={token}");
    let (reply, _) = protocol
        .serve(core, request(Method::GET, &url), APP_ORIGIN)
        .await;
    (
        reply.status(),
        String::from_utf8(reply.into_body()).unwrap(),
    )
}

#[tokio::test]
async fn translated_variant_serves_translated_text_with_bound_slots_and_lang() {
    let (_directory, core, ids, _db) = fixture();
    let (protocol, id) = (BodyProtocol::default(), &ids[0]);
    core.body_test_translation(id, TARGET, true).unwrap();
    let original = protocol.prepare(&core, id.clone()).await.unwrap();
    let (status, plain) = serve_document(&protocol, &core, id, &original.document_token).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!plain.contains("<html lang"));
    assert!(!plain.contains("[zh-Hans]"));

    let prepared = protocol
        .prepare_with_images(&core, id.clone(), false, false, Some((TARGET.into(), true)))
        .await
        .unwrap();
    let (status, html) = serve_document(&protocol, &core, id, &prepared.document_token).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.starts_with("<!doctype html><html lang=\"zh-Hans\"><head>"));
    assert!(html.contains("[zh-Hans] Synthetic protocol fixture"));
    assert!(!html.contains("untrustedExecuted"));
    let (src, href) = (attribute(&html, "src"), attribute(&html, "href"));
    assert!(
        src.starts_with(&format!("{BODY_ORIGIN}/part/{id}/")),
        "{src}"
    );
    assert!(
        href.starts_with(&format!("{BODY_ORIGIN}/link/{id}/")),
        "{href}"
    );
    assert!(src.ends_with(&prepared.document_token));
    let (image, _) = protocol
        .serve(&core, request(Method::GET, &src), APP_ORIGIN)
        .await;
    assert_eq!(image.status(), StatusCode::OK);
    assert_eq!(image.headers()["Content-Type"], "image/png");
    let (link, event) = protocol
        .serve(&core, request(Method::GET, &href), APP_ORIGIN)
        .await;
    assert_eq!(link.status(), StatusCode::NO_CONTENT);
    assert!(event.is_some_and(|event| event.url.starts_with("https://example.invalid/")));

    // The original document is untouched by the translated one.
    let (_, again) = serve_document(&protocol, &core, id, &original.document_token).await;
    assert_eq!(again.replace(&original.document_token, "T"), {
        plain.replace(&original.document_token, "T")
    });
}

#[tokio::test]
async fn preparing_a_variant_needs_a_current_translation() {
    let (_directory, core, ids, _db) = fixture();
    let (protocol, id) = (BodyProtocol::default(), &ids[0]);
    let variant = |target: &str, quotes| Some((target.to_owned(), quotes));
    let missing_one = |result: Result<PreparedBody, AppError>| {
        assert_eq!(
            result.err().map(|error| error.code),
            Some(ErrorCode::NotFound)
        );
    };
    missing_one(
        protocol
            .prepare_with_images(&core, id.clone(), false, false, variant(TARGET, true))
            .await,
    );
    core.body_test_translation(id, TARGET, true).unwrap();
    // Another target or quote setting is not the cached translation.
    missing_one(
        protocol
            .prepare_with_images(&core, id.clone(), false, false, variant("ja", true))
            .await,
    );
    missing_one(
        protocol
            .prepare_with_images(&core, id.clone(), false, false, variant(TARGET, false))
            .await,
    );
    // Anything outside the fixed language table is refused outright.
    assert!(
        protocol
            .prepare_with_images(&core, id.clone(), false, false, variant("\"><x", true))
            .await
            .is_err()
    );
    assert!(
        protocol
            .prepare_with_images(&core, id.clone(), false, false, variant(TARGET, true))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn a_cleared_translation_is_a_404_for_an_issued_token() {
    let (_directory, core, ids, _db) = fixture();
    let (protocol, id) = (BodyProtocol::default(), &ids[0]);
    core.body_test_translation(id, TARGET, true).unwrap();
    let prepared = protocol
        .prepare_with_images(&core, id.clone(), false, false, Some((TARGET.into(), true)))
        .await
        .unwrap();
    assert_eq!(
        serve_document(&protocol, &core, id, &prepared.document_token)
            .await
            .0,
        StatusCode::OK
    );
    core.clear_translation(id.clone()).await.unwrap();
    let (status, body) = serve_document(&protocol, &core, id, &prepared.document_token).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(!body.contains("Synthetic protocol fixture"));
}

#[tokio::test]
async fn the_variant_survives_retry_images_with_the_same_image_rules() {
    use mailx_core::RemoteImagePolicy::Ask;
    let raw = b"Content-Type: text/html\r\n\r\n<p>Hello there</p><img src=\"https://example.invalid/image\">";
    let directory = TempDir::new().unwrap();
    let (core, ids) =
        MailxCore::body_test_fixture(directory.path(), tokio::runtime::Handle::current(), &[raw])
            .unwrap();
    let (protocol, id) = (BodyProtocol::default(), &ids[0]);
    core.body_test_translation(id, TARGET, true).unwrap();
    let blocked = protocol
        .prepare_with_images(&core, id.clone(), false, false, Some((TARGET.into(), true)))
        .await
        .unwrap();
    let (_, html) = serve_document(&protocol, &core, id, &blocked.document_token).await;
    assert!(html.contains("[zh-Hans] Hello there"));
    assert!(!html.contains("/remote/"));

    let allowed = protocol
        .prepare_with_images(&core, id.clone(), true, true, Some((TARGET.into(), true)))
        .await
        .unwrap();
    let retried = protocol
        .retry_images(&core, id, &allowed.document_token, Ask)
        .await
        .unwrap();
    assert_ne!(retried.document_token, allowed.document_token);
    let (status, html) = serve_document(&protocol, &core, id, &retried.document_token).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.starts_with("<!doctype html><html lang=\"zh-Hans\">"));
    assert!(html.contains("[zh-Hans] Hello there"));
    assert!(html.contains(&format!(
        "{BODY_ORIGIN}/remote/{id}/0?token={}",
        retried.document_token
    )));
    assert!(!html.contains("https://example.invalid/image"));
    // The superseded token is gone.
    assert_eq!(
        serve_document(&protocol, &core, id, &allowed.document_token)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn drafts_never_carry_a_variant() {
    let (_directory, core, _ids, _db) = fixture();
    let protocol = BodyProtocol::default();
    let draft = core
        .create_draft(
            mailx_core::CreateDraftRequest {
                account_id: None,
                to: vec![],
                cc: vec![],
                bcc: vec![],
                subject: "s".into(),
                body_html: "<p>typed text</p>".into(),
                client_mutation_id: Uuid::new_v4().to_string(),
            },
            "",
        )
        .unwrap();
    let prepared = protocol
        .prepare_draft(&core, draft.id.clone(), draft.revision)
        .await
        .unwrap();
    let (status, html) =
        serve_document(&protocol, &core, &draft.id, &prepared.document_token).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("typed text") && !html.contains("<html lang"));
}
