use rusqlite::Connection;
use tempfile::TempDir;

use super::*;

const APP_ORIGIN: &str = "tauri://localhost";
const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC";

fn mime() -> Vec<u8> {
    format!(
        "MIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=fixture\r\n\r\n\
         --fixture\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
         <p>Synthetic protocol fixture</p><img src=\"cid:pixel\">\
         <a href=\"https://example.invalid/full/path?x=1&amp;y=2\">link</a>\
         <script>window.untrustedExecuted=true</script>\r\n\
         --fixture\r\nContent-Type: image/png\r\nContent-Disposition: inline\r\n\
         Content-ID: <pixel>\r\nContent-Transfer-Encoding: base64\r\n\r\n{PNG}\r\n\
         --fixture--\r\n"
    )
    .into_bytes()
}

fn fixture() -> (TempDir, MailxCore, Vec<String>, Connection) {
    let directory = TempDir::new().unwrap();
    let raw = mime();
    let (core, ids) = MailxCore::body_test_fixture(
        directory.path(),
        tokio::runtime::Handle::current(),
        &[&raw, &raw],
    )
    .unwrap();
    let db = Connection::open(directory.path().join("mailx.sqlite3")).unwrap();
    (directory, core, ids, db)
}

fn request(method: Method, url: &str) -> Request<Vec<u8>> {
    Request::builder()
        .method(method)
        .uri(url)
        .body(vec![])
        .unwrap()
}

fn attribute(html: &str, name: &str) -> String {
    html.split_once(&format!("{name}=\""))
        .unwrap()
        .1
        .split('"')
        .next()
        .unwrap()
        .replace("&amp;", "&")
}

fn response_headers(
    response: &Response<Vec<u8>>,
    mime: &str,
    app: &str,
    image_scope: Option<&str>,
) {
    let headers = response.headers();
    assert_eq!(headers["Content-Type"], mime);
    assert_eq!(headers["X-Content-Type-Options"], "nosniff");
    assert_eq!(headers["Referrer-Policy"], "no-referrer");
    assert_eq!(headers["Cache-Control"], "no-store");
    let images = image_scope.map_or_else(|| "data:".to_owned(), |scope| format!("data: {scope}"));
    assert_eq!(
        headers["Content-Security-Policy"],
        format!(
            "default-src 'none'; style-src 'unsafe-inline'; img-src {images}; font-src data:; form-action 'none'; base-uri 'none'; frame-ancestors {app}"
        )
    );
    assert!(!headers.contains_key("Access-Control-Allow-Origin"));
    assert!(!headers.contains_key("Location"));
}

async fn document(
    protocol: &BodyProtocol,
    core: &MailxCore,
    origin: &str,
    id: &str,
) -> (String, String) {
    let (response, event) = protocol
        .serve(
            core,
            request(Method::GET, &format!("{origin}/document/{id}")),
            APP_ORIGIN,
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(event.is_none());
    response_headers(
        &response,
        "text/html; charset=utf-8",
        APP_ORIGIN,
        Some(&format!("{origin}/part/{id}/")),
    );
    let html = String::from_utf8(response.into_body()).unwrap();
    assert!(html.contains("Synthetic protocol fixture"));
    assert!(!html.contains("untrustedExecuted"));
    (attribute(&html, "src"), attribute(&html, "href"))
}

#[test]
fn both_platform_forms_round_trip_encoded_cids_and_canonical_links() {
    let id = Uuid::new_v4().to_string();
    let token = Uuid::new_v4().to_string();
    for origin in ["mailx-body://localhost", "http://mailx-body.localhost"] {
        assert_eq!(
            parse_route(&format!("{origin}/document/{id}")),
            Some((Route::Document { id: id.clone() }, origin.into()))
        );
        for cid in ["pixel", "中文/section?x=#% @mail"] {
            let encoded = resource_url(origin, "part", &id, cid, &token);
            assert_eq!(
                parse_route(&encoded),
                Some((
                    Route::Part {
                        id: id.clone(),
                        cid: cid.into(),
                        token: token.clone()
                    },
                    origin.into()
                ))
            );
        }
        assert_eq!(
            parse_route(&resource_url(origin, "link", &id, "0", &token)),
            Some((
                Route::Link {
                    id: id.clone(),
                    index: 0,
                    token: token.clone()
                },
                origin.into()
            ))
        );
    }
}

#[test]
fn malformed_routes_cannot_create_an_authorized_resource() {
    let id = Uuid::new_v4().to_string();
    let token = Uuid::new_v4().to_string();
    for url in [
        format!("https://mailx-body.localhost/document/{id}"),
        format!("http://mailx-body.localhost:81/document/{id}"),
        format!("mailx-body://attacker.invalid/document/{id}"),
        format!("mailx-body://user@localhost/document/{id}"),
        format!("mailx-body://localhost/document/{id}#fragment"),
        format!("mailx-body://localhost/document/{id}?token={token}"),
        format!("mailx-body://localhost/document/{id}/extra"),
        "mailx-body://localhost/document/not-a-uuid".into(),
        format!("mailx-body://localhost/part/{id}/pixel"),
        format!("mailx-body://localhost/part/{id}/pixel?token=bad"),
        format!("mailx-body://localhost/part/{id}/pixel?token={token}&token={token}"),
        format!("mailx-body://localhost/part/{id}/pixel?other={token}"),
        format!("mailx-body://localhost/part/{id}/?token={token}"),
        format!("mailx-body://localhost/part/{id}/bad%?token={token}"),
        format!("mailx-body://localhost/part/{id}/%GG?token={token}"),
        format!("mailx-body://localhost/part/{id}/%FF?token={token}"),
        format!("mailx-body://localhost/part/{id}/%0A?token={token}"),
        format!("mailx-body://localhost/link/{id}/-1?token={token}"),
        format!("mailx-body://localhost/link/{id}/01?token={token}"),
        format!("mailx-body://localhost/link/{id}/999999999999999999999999999999?token={token}"),
        format!("mailx-body://localhost/unknown/{id}"),
    ] {
        assert!(
            parse_route(&url).is_none(),
            "accepted malformed route: {url}"
        );
    }
}

#[test]
fn frame_ancestor_is_an_explicit_application_origin() {
    for (url, expected) in [
        ("tauri://localhost/index.html", Some("tauri://localhost")),
        (
            "http://tauri.localhost/index.html",
            Some("http://tauri.localhost"),
        ),
        (
            "http://localhost:5173/index.html",
            cfg!(debug_assertions).then_some("http://localhost:5173"),
        ),
        ("https://example.invalid", None),
        ("http://localhost:1234", None),
        ("tauri://user@localhost", None),
        ("http://tauri.localhost:1234", None),
        ("mailx-body://localhost", None),
    ] {
        assert_eq!(app_origin(&Url::parse(url).unwrap()).as_deref(), expected);
    }
}

#[tokio::test]
async fn document_loads_real_png_with_current_message_only_csp_and_new_tokens() {
    let (_directory, core, ids, _db) = fixture();
    let protocol = BodyProtocol::new(false);
    for origin in ["mailx-body://localhost", "http://mailx-body.localhost"] {
        let (part_url, link_url) = document(&protocol, &core, origin, &ids[0]).await;
        let (Route::Part { token, .. }, _) = parse_route(&part_url).unwrap() else {
            panic!("part route")
        };
        assert!(Uuid::parse_str(&token).is_ok());
        assert!(link_url.contains(&format!("/link/{}/0", ids[0])));
        let (image, event) = protocol
            .serve(&core, request(Method::GET, &part_url), APP_ORIGIN)
            .await;
        assert_eq!(image.status(), StatusCode::OK);
        response_headers(&image, "image/png", APP_ORIGIN, None);
        assert!(image.body().starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(event.is_none());
        let (second_part_url, _) = document(&protocol, &core, origin, &ids[0]).await;
        assert_ne!(second_part_url, part_url);
    }
    assert!(protocol.confirmations.lock().unwrap().is_empty());
}

#[tokio::test]
async fn invalid_unready_cross_message_and_non_get_requests_fail_without_events() {
    let (_directory, core, ids, db) = fixture();
    let protocol = BodyProtocol::new(false);
    let origin = "mailx-body://localhost";
    let (part, link) = document(&protocol, &core, origin, &ids[0]).await;
    let (Route::Part { token, .. }, _) = parse_route(&part).unwrap() else {
        panic!("part route")
    };
    for bad in [
        format!("{origin}/document/{}", Uuid::new_v4()),
        resource_url(origin, "part", &ids[1], "pixel", &token),
        resource_url(origin, "link", &ids[1], "0", &token),
        resource_url(origin, "part", &ids[0], "unknown", &token),
        resource_url(
            origin,
            "part",
            &ids[0],
            "pixel",
            &Uuid::new_v4().to_string(),
        ),
        resource_url(origin, "link", &ids[0], "99", &token),
    ] {
        let (response, event) = protocol
            .serve(&core, request(Method::GET, &bad), APP_ORIGIN)
            .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        response_headers(&response, "text/plain; charset=utf-8", APP_ORIGIN, None);
        assert!(response.body().is_empty() && event.is_none());
    }
    for method in [
        Method::POST,
        Method::PUT,
        Method::DELETE,
        Method::HEAD,
        Method::OPTIONS,
    ] {
        let (response, event) = protocol
            .serve(&core, request(method, &link), APP_ORIGIN)
            .await;
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(response.headers()["Allow"], "GET");
        response_headers(&response, "text/plain; charset=utf-8", APP_ORIGIN, None);
        assert!(response.body().is_empty() && event.is_none());
    }
    for state in ["missing", "fetching", "failed"] {
        db.execute("UPDATE messages SET body_state = ?1", [state])
            .unwrap();
        let (response, event) = protocol
            .serve(
                &core,
                request(Method::GET, &format!("{origin}/document/{}", ids[0])),
                APP_ORIGIN,
            )
            .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        response_headers(&response, "text/plain; charset=utf-8", APP_ORIGIN, None);
        assert!(event.is_none());
    }
    assert!(protocol.confirmations.lock().unwrap().is_empty());
}

#[tokio::test]
async fn link_get_only_requests_confirmation_and_confirmation_is_consumed_once() {
    let (_directory, core, ids, _db) = fixture();
    let protocol = BodyProtocol::new(false);
    let (_, link) = document(&protocol, &core, "mailx-body://localhost", &ids[0]).await;
    assert_eq!(
        protocol
            .confirm_link(&core, ids[0].clone(), 0)
            .await
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    let (response, event) = protocol
        .serve(&core, request(Method::GET, &link), APP_ORIGIN)
        .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(response.body().is_empty());
    response_headers(&response, "text/plain; charset=utf-8", APP_ORIGIN, None);
    let event = event.unwrap();
    assert_eq!(event.email_id, ids[0]);
    assert_eq!(event.index, 0);
    assert_eq!(event.url, "https://example.invalid/full/path?x=1&y=2");
    assert_eq!(
        serde_json::to_value(&event).unwrap(),
        serde_json::json!({"emailId": ids[0], "index":0, "url":event.url})
    );
    assert_eq!(protocol.confirmations.lock().unwrap().len(), 1);
    // The protocol returns a target only after this explicit trusted call. No
    // opener is involved in GET or in these tests.
    assert_eq!(
        protocol
            .confirm_link(&core, ids[0].clone(), 0)
            .await
            .unwrap(),
        event.url
    );
    assert!(protocol.confirmations.lock().unwrap().is_empty());
    assert_eq!(
        protocol
            .confirm_link(&core, ids[0].clone(), 0)
            .await
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
}

#[tokio::test]
async fn edits_and_changed_link_mapping_revoke_pending_confirmations() {
    for mutation in [
        "UPDATE accounts SET revision = revision + 1",
        "UPDATE folder_sync_cursors SET uid_validity = 18",
        "UPDATE messages SET remote_state = 'deleted'",
        "UPDATE message_bodies SET links = '[\"https://example.invalid/different\"]'",
        "UPDATE message_bodies SET links = '[\"javascript:alert(1)\"]'",
    ] {
        let (_directory, core, ids, db) = fixture();
        let protocol = BodyProtocol::new(false);
        let (part, link) = document(&protocol, &core, "mailx-body://localhost", &ids[0]).await;
        let (_, event) = protocol
            .serve(&core, request(Method::GET, &link), APP_ORIGIN)
            .await;
        assert!(event.is_some());
        db.execute(mutation, []).unwrap();
        assert_eq!(
            protocol
                .confirm_link(&core, ids[0].clone(), 0)
                .await
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
        assert!(protocol.confirmations.lock().unwrap().is_empty());
        if !mutation.contains("SET links") {
            let (response, event) = protocol
                .serve(&core, request(Method::GET, &part), APP_ORIGIN)
                .await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
            assert!(event.is_none());
        }
    }
}

#[tokio::test]
async fn expired_tokens_and_pending_confirmations_are_rejected() {
    let (_directory, core, ids, _db) = fixture();
    let protocol = BodyProtocol::new(false);
    let (part, link) = document(&protocol, &core, "mailx-body://localhost", &ids[0]).await;
    let (_, event) = protocol
        .serve(&core, request(Method::GET, &link), APP_ORIGIN)
        .await;
    assert!(event.is_some());
    let expired = Instant::now() - TOKEN_LIFETIME - Duration::from_secs(1);
    for document in protocol.documents.lock().unwrap().values_mut() {
        document.created = expired;
    }
    for confirmation in protocol.confirmations.lock().unwrap().values_mut() {
        confirmation.document.created = expired;
    }
    for url in [&part, &link] {
        let (response, event) = protocol
            .serve(&core, request(Method::GET, url), APP_ORIGIN)
            .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(event.is_none());
    }
    assert_eq!(
        protocol
            .confirm_link(&core, ids[0].clone(), 0)
            .await
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
}

async fn numbered_document(protocol: &BodyProtocol, core: &MailxCore, id: &str) -> String {
    let (response, event) = protocol
        .serve(
            core,
            request(
                Method::GET,
                &format!("mailx-body://localhost/document/{id}"),
            ),
            APP_ORIGIN,
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(event.is_none());
    let html = String::from_utf8(response.into_body()).unwrap();
    assert!(!html.contains("href="));
    assert!(html.contains("[1]"));
    html
}

#[test]
fn only_macos_defaults_to_the_verified_numbered_link_fallback() {
    assert_eq!(
        BodyProtocol::default().numbered_links,
        cfg!(target_os = "macos")
    );
}

#[tokio::test]
async fn numbered_links_require_a_loaded_bound_document_and_never_navigate() {
    let (_directory, core, ids, _db) = fixture();
    let protocol = BodyProtocol::new(true);
    assert!(protocol.list_links(&core, ids[0].clone()).await.is_err());
    let html = numbered_document(&protocol, &core, &ids[0]).await;
    let listing = protocol.list_links(&core, ids[0].clone()).await.unwrap();
    let token = listing.document_token.clone().unwrap();
    assert_eq!(listing.links.len(), 1);
    assert_eq!(listing.links[0].index, 0);
    assert_eq!(
        listing.links[0].url,
        "https://example.invalid/full/path?x=1&y=2"
    );
    assert!(
        protocol
            .confirm_link(&core, ids[0].clone(), 0)
            .await
            .is_err()
    );
    let metadata = serde_json::to_value(&listing).unwrap();
    assert_eq!(metadata.as_object().unwrap().len(), 2);
    assert!(metadata.get("html").is_none());
    let (image, _) = protocol
        .serve(
            &core,
            request(Method::GET, &attribute(&html, "src")),
            APP_ORIGIN,
        )
        .await;
    assert_eq!(image.status(), StatusCode::OK);
    let direct = resource_url("mailx-body://localhost", "link", &ids[0], "0", &token);
    let (response, event) = protocol
        .serve(&core, request(Method::GET, &direct), APP_ORIGIN)
        .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(event.is_none());
    for (id, index, bad_token) in [
        (ids[1].clone(), 0, token.clone()),
        (ids[0].clone(), 1, token.clone()),
        (ids[0].clone(), 0, Uuid::new_v4().to_string()),
    ] {
        assert!(
            protocol
                .request_link(&core, id, index, bad_token)
                .await
                .is_err()
        );
    }
    let event = protocol
        .request_link(&core, ids[0].clone(), 0, token)
        .await
        .unwrap();
    assert_eq!(event.url, listing.links[0].url);
    assert_eq!(
        protocol
            .confirm_link(&core, ids[0].clone(), 0)
            .await
            .unwrap(),
        event.url
    );
    assert!(
        protocol
            .confirm_link(&core, ids[0].clone(), 0)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn numbered_link_lists_and_confirmations_expire_when_document_is_replaced() {
    let (_directory, core, ids, _db) = fixture();
    let protocol = BodyProtocol::new(true);
    numbered_document(&protocol, &core, &ids[0]).await;
    let token = protocol
        .list_links(&core, ids[0].clone())
        .await
        .unwrap()
        .document_token
        .unwrap();
    protocol
        .request_link(&core, ids[0].clone(), 0, token.clone())
        .await
        .unwrap();
    numbered_document(&protocol, &core, &ids[0]).await;
    assert!(
        protocol
            .request_link(&core, ids[0].clone(), 0, token)
            .await
            .is_err()
    );
    assert!(
        protocol
            .confirm_link(&core, ids[0].clone(), 0)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn numbered_links_recheck_identity_mapping_and_binding_before_confirmation() {
    for mutation in [
        "UPDATE accounts SET revision = revision + 1",
        "UPDATE folder_sync_cursors SET uid_validity = 18",
        "UPDATE messages SET remote_state = 'deleted'",
        "UPDATE message_bodies SET raw_sha256 = printf('%064d', 0)",
        "UPDATE message_bodies SET sanitizer_version = 0",
        "UPDATE message_bodies SET links = '[\"https://example.invalid/changed\"]'",
        "UPDATE message_bodies SET links = '[\"javascript:alert(1)\"]'",
    ] {
        let (_directory, core, ids, db) = fixture();
        let protocol = BodyProtocol::new(true);
        numbered_document(&protocol, &core, &ids[0]).await;
        let token = protocol
            .list_links(&core, ids[0].clone())
            .await
            .unwrap()
            .document_token
            .unwrap();
        protocol
            .request_link(&core, ids[0].clone(), 0, token.clone())
            .await
            .unwrap();
        db.execute(mutation, []).unwrap();
        assert!(
            protocol
                .confirm_link(&core, ids[0].clone(), 0)
                .await
                .is_err()
        );
        if !mutation.contains("SET links") {
            assert!(protocol.list_links(&core, ids[0].clone()).await.is_err());
            assert!(
                protocol
                    .request_link(&core, ids[0].clone(), 0, token)
                    .await
                    .is_err()
            );
        }
    }
}
