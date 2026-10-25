use std::{io::Cursor, time::Duration};

use base64::{Engine, engine::general_purpose::STANDARD};
use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use rusqlite::{Connection, params};
use tempfile::TempDir;

use super::*;

fn synthetic_mail(color: [u8; 3], marker: &str, cid: &str) -> (Vec<u8>, Vec<u8>) {
    let mut image = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(RgbImage::from_pixel(1, 1, Rgb(color)))
        .write_to(&mut image, ImageFormat::Png)
        .unwrap();
    let image = image.into_inner();
    let raw = format!(
        "MIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=fixture\r\n\r\n\
         --fixture\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
         <p>{marker}</p><img src=\"cid:{cid}\"><a href=\"https://example.invalid/{marker}\">link</a>\
         <script>window.syntheticUnsafe=true</script>\r\n\
         --fixture\r\nContent-Type: image/png\r\nContent-Disposition: inline\r\n\
         Content-ID: <{cid}>\r\nContent-Transfer-Encoding: base64\r\n\r\n{}\r\n\
         --fixture--\r\n", STANDARD.encode(&image),
    ).into_bytes();
    (raw, image)
}

fn fixture(raws: &[&[u8]]) -> (TempDir, MailxCore, Vec<String>, Connection) {
    let directory = TempDir::new().unwrap();
    let (core, ids) =
        MailxCore::body_test_fixture(directory.path(), tokio::runtime::Handle::current(), raws)
            .unwrap();
    let db = Connection::open(directory.path().join("mailx.sqlite3")).unwrap();
    (directory, core, ids, db)
}

fn assert_missing<T>(result: Result<T, AppError>) {
    match result {
        Ok(_) => panic!("unavailable body resource unexpectedly succeeded"),
        Err(error) => assert_eq!(error.code, ErrorCode::NotFound),
    }
}

#[tokio::test]
async fn same_cid_stays_with_its_opaque_message_binding() {
    let (raw_a, image_a) = synthetic_mail([255, 0, 0], "first", "shared");
    let (raw_b, image_b) = synthetic_mail([0, 0, 255], "second", "shared");
    let (_directory, core, ids, _db) = fixture(&[&raw_a, &raw_b]);
    assert!(
        core.sync_states().is_empty(),
        "fixture starts no account worker"
    );
    let first = core.body_document(ids[0].clone()).await.unwrap();
    let second = core.body_document(ids[1].clone()).await.unwrap();
    assert!(first.html.contains("first") && !first.html.contains("syntheticUnsafe"));
    assert_eq!(first.binding.email_id(), ids[0]);
    assert_eq!(
        core.body_part(first.binding.clone(), "shared".into())
            .await
            .unwrap()
            .bytes,
        image_a
    );
    assert_eq!(
        core.body_part(second.binding.clone(), "shared".into())
            .await
            .unwrap()
            .bytes,
        image_b
    );
    assert_missing(
        core.body_part(first.binding.clone(), "unknown".into())
            .await,
    );
    assert_missing(
        core.body_link(ids[1].clone(), 0, Some(first.binding.clone()))
            .await,
    );
    assert_eq!(
        core.body_link(ids[0].clone(), 0, Some(first.binding.clone()))
            .await
            .unwrap(),
        "https://example.invalid/first"
    );
    assert_missing(core.body_link(ids[0].clone(), 1, None).await);
    assert_missing(core.body_link(ids[0].clone(), usize::MAX, None).await);

    // Only this private module can manufacture a mixed binding. The public API
    // gives hosts no setter for identity/raw/version; mismatched storage rejects it.
    let mut forged = first.binding;
    forged.identity = second.binding.identity;
    assert_missing(core.body_part(forged, "shared".into()).await);
}

#[tokio::test]
async fn old_bindings_fail_after_every_visibility_or_account_identity_change() {
    for mutation in [
        "UPDATE accounts SET revision = revision + 1",
        "UPDATE accounts SET mailbox_source_id = 'changed-source'",
        "UPDATE folders SET selectable = 0",
        "UPDATE folder_sync_cursors SET uid_validity = 18",
        "UPDATE messages SET remote_state = 'deleted'",
        "UPDATE messages SET remote_state = 'stale'",
    ] {
        let (raw, _image) = synthetic_mail([10, 20, 30], "bound", "logo");
        let (_directory, core, ids, db) = fixture(&[&raw]);
        let document = core.body_document(ids[0].clone()).await.unwrap();
        db.execute(mutation, []).unwrap();
        assert_missing(core.body_document_bound(document.binding.clone()).await);
        assert_missing(
            core.body_links(ids[0].clone(), Some(document.binding.clone()))
                .await,
        );
        if !mutation.starts_with("UPDATE accounts SET revision") {
            assert_missing(core.body_links(ids[0].clone(), None).await);
        }
        assert_missing(
            core.body_part(document.binding.clone(), "logo".into())
                .await,
        );
        assert_missing(
            core.body_link(ids[0].clone(), 0, Some(document.binding))
                .await,
        );
    }
}

#[tokio::test]
async fn replacement_raw_and_sanitizer_invalidate_existing_bindings() {
    let (raw, _image) = synthetic_mail([1, 2, 3], "bound", "logo");
    for mutation in [
        "UPDATE message_bodies SET raw_sha256 = printf('%064d', 0)",
        "UPDATE message_bodies SET raw_size = raw_size + 1",
        "UPDATE message_bodies SET sanitizer_version = 0",
    ] {
        let (_directory, core, ids, db) = fixture(&[&raw]);
        let document = core.body_document(ids[0].clone()).await.unwrap();
        db.execute(mutation, []).unwrap();
        assert_missing(core.body_document_bound(document.binding.clone()).await);
        assert_missing(
            core.body_links(ids[0].clone(), Some(document.binding.clone()))
                .await,
        );
        assert_missing(
            core.body_part(document.binding.clone(), "logo".into())
                .await,
        );
        assert_missing(
            core.body_link(ids[0].clone(), 0, Some(document.binding))
                .await,
        );
    }
}

#[tokio::test]
async fn old_sanitizer_rebuilds_from_local_mime_without_credentials_or_workers() {
    let (raw, expected_image) = synthetic_mail([32, 64, 128], "locally-rebuilt", "logo");
    let (_directory, core, ids, db) = fixture(&[&raw]);
    let old_binding = core.body_document(ids[0].clone()).await.unwrap().binding;
    db.execute("UPDATE message_bodies SET sanitizer_version = 0, sanitized_html = '<p>OLD_UNSAFE_CACHE</p>', links = '[\"https://example.invalid/obsolete\"]'", []).unwrap();
    assert_missing(core.body_links(ids[0].clone(), Some(old_binding)).await);
    assert_eq!(
        db.query_row("SELECT sanitizer_version FROM message_bodies", [], |row| {
            row.get::<_, u32>(0)
        })
        .unwrap(),
        0,
        "a stale document binding must not rebuild and silently rebind"
    );
    let links = tokio::time::timeout(
        Duration::from_secs(5),
        core.body_links(ids[0].clone(), None),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(links.links, ["https://example.invalid/locally-rebuilt"]);
    let document = tokio::time::timeout(Duration::from_secs(5), core.body_document(ids[0].clone()))
        .await
        .unwrap()
        .unwrap();
    assert!(document.html.contains("locally-rebuilt"));
    assert!(!document.html.contains("OLD_UNSAFE_CACHE"));
    assert!(!document.html.contains("syntheticUnsafe"));
    assert_eq!(
        db.query_row("SELECT sanitizer_version FROM message_bodies", [], |row| {
            row.get::<_, u32>(0)
        })
        .unwrap(),
        SANITIZER_VERSION
    );
    assert_eq!(
        core.body_part(document.binding, "logo".into())
            .await
            .unwrap()
            .bytes,
        expected_image
    );
    assert!(core.sync_states().is_empty());
}

#[tokio::test]
async fn non_ready_document_reads_cannot_start_a_network_fetch() {
    let (raw, _image) = synthetic_mail([1, 1, 1], "cache", "logo");
    let (_directory, core, ids, db) = fixture(&[&raw]);
    let binding = core.body_document(ids[0].clone()).await.unwrap().binding;
    for state in ["missing", "fetching", "failed"] {
        db.execute("UPDATE messages SET body_state = ?1", [state])
            .unwrap();
        assert_missing(
            tokio::time::timeout(Duration::from_secs(1), core.body_document(ids[0].clone()))
                .await
                .unwrap(),
        );
        for binding in [None, Some(binding.clone())] {
            assert_missing(
                tokio::time::timeout(
                    Duration::from_secs(1),
                    core.body_links(ids[0].clone(), binding),
                )
                .await
                .unwrap(),
            );
        }
        assert_eq!(
            db.query_row("SELECT body_state FROM messages", [], |row| row
                .get::<_, String>(0))
                .unwrap(),
            state
        );
    }
    assert!(core.sync_states().is_empty());
    assert_missing(core.body_document(uuid::Uuid::new_v4().to_string()).await);
    assert_missing(
        core.body_links(uuid::Uuid::new_v4().to_string(), None)
            .await,
    );
}

#[tokio::test]
async fn numbered_links_preserve_indices_without_loading_html() {
    let (raw, _image) = synthetic_mail([3, 3, 3], "numbered", "logo");
    let (_directory, core, ids, db) = fixture(&[&raw, &raw]);
    let document = core.body_document(ids[0].clone()).await.unwrap();
    let expected = [
        "https://example.invalid/first",
        "mailto:fixture@example.invalid",
        "http://example.invalid/third",
        "https://example.invalid/first",
    ];
    db.execute(
        "UPDATE message_bodies SET sanitized_html = NULL, links = ?1 WHERE message_id = ?2",
        params![serde_json::to_string(&expected).unwrap(), ids[0]],
    )
    .unwrap();
    let links = core
        .body_links(ids[0].clone(), Some(document.binding))
        .await
        .unwrap();
    assert_eq!(links.links, expected);
    assert_eq!(links.binding.email_id(), ids[0]);
    assert_eq!(
        core.body_links(ids[0].clone(), None).await.unwrap().links,
        expected
    );
    assert_missing(core.body_document(ids[0].clone()).await);
    assert_missing(
        core.body_links(ids[1].clone(), Some(links.binding.clone()))
            .await,
    );
    for (index, url) in expected.iter().enumerate() {
        assert_eq!(
            core.body_link(ids[0].clone(), index, Some(links.binding.clone()))
                .await
                .unwrap(),
            *url
        );
    }
    db.execute(
        "UPDATE message_bodies SET links = '[]' WHERE message_id = ?1",
        [&ids[0]],
    )
    .unwrap();
    assert!(
        core.body_links(ids[0].clone(), Some(links.binding))
            .await
            .unwrap()
            .links
            .is_empty()
    );
}

#[tokio::test]
async fn numbered_links_reject_invalid_members_without_renumbering() {
    let (raw, _image) = synthetic_mail([4, 4, 4], "numbered", "logo");
    let (_directory, core, ids, db) = fixture(&[&raw]);
    let binding = core.body_document(ids[0].clone()).await.unwrap().binding;
    for blocked in [
        "javascript:alert(1)",
        "file:///private",
        "data:text/html,bad",
        "mailx-body://localhost/link/other/0",
    ] {
        db.execute(
            "UPDATE message_bodies SET links = ?1",
            [serde_json::to_string(&[
                "https://example.invalid/first",
                blocked,
                "https://example.invalid/third",
            ])
            .unwrap()],
        )
        .unwrap();
        assert_missing(core.body_links(ids[0].clone(), None).await);
        assert_missing(core.body_links(ids[0].clone(), Some(binding.clone())).await);
        // The existing single-link API still addresses the original index.
        assert_eq!(
            core.body_link(ids[0].clone(), 2, Some(binding.clone()))
                .await
                .unwrap(),
            "https://example.invalid/third"
        );
    }
}

#[tokio::test]
async fn links_recheck_current_database_value_and_allowed_scheme() {
    let (raw, _image) = synthetic_mail([2, 2, 2], "link", "logo");
    let (_directory, core, ids, db) = fixture(&[&raw]);
    let document = core.body_document(ids[0].clone()).await.unwrap();
    for blocked in [
        "javascript:alert(1)",
        "file:///private",
        "data:text/html,bad",
        "mailx-body://localhost/link/other/0",
    ] {
        db.execute(
            "UPDATE message_bodies SET links = ?1",
            [serde_json::to_string(&[blocked]).unwrap()],
        )
        .unwrap();
        assert_missing(
            core.body_link(ids[0].clone(), 0, Some(document.binding.clone()))
                .await,
        );
        assert_missing(core.body_link(ids[0].clone(), 0, None).await);
    }
    for allowed in [
        "https://example.invalid/changed",
        "http://example.invalid/changed",
        "mailto:fixture@example.invalid",
    ] {
        db.execute(
            "UPDATE message_bodies SET links = ?1",
            params![serde_json::to_string(&[allowed]).unwrap()],
        )
        .unwrap();
        assert_eq!(
            core.body_link(ids[0].clone(), 0, None).await.unwrap(),
            allowed
        );
    }
}

#[tokio::test]
async fn fixture_rejects_nonempty_profiles_before_opening_them() {
    let directory = TempDir::new().unwrap();
    std::fs::write(directory.path().join("existing-data"), b"untouched").unwrap();
    let result =
        MailxCore::body_test_fixture(directory.path(), tokio::runtime::Handle::current(), &[]);
    assert!(matches!(
        result,
        Err(AppError {
            code: ErrorCode::Validation,
            ..
        })
    ));
    assert!(!directory.path().join("mailx.sqlite3").exists());
    assert_eq!(
        std::fs::read(directory.path().join("existing-data")).unwrap(),
        b"untouched"
    );
}

#[tokio::test]
async fn remote_cache_is_reading_scoped_and_rechecks_identity() {
    let raw = b"Content-Type: text/html\r\n\r\n<img src=\"https://example.invalid/pixel\">";
    let (_directory, core, ids, db) = fixture(&[raw, raw]);
    let first = core.body_document(ids[0].clone()).await.unwrap();
    let second = core.body_document(ids[1].clone()).await.unwrap();
    let reopened = core.body_document(ids[0].clone()).await.unwrap();
    let (_, bytes) = synthetic_mail([0, 0, 255], "cache", "pixel");
    let image = render::validate_image(&bytes, "image/png").unwrap();
    let url = "https://example.invalid/pixel";
    let version = || {
        db.pragma_query_value(None, "data_version", |row| row.get::<_, i64>(0))
            .unwrap()
    };
    let before = version();
    first
        .binding
        .cache_remote_image(url.into(), &image)
        .unwrap();
    assert!(second.binding.cached_remote_image(url).unwrap().is_none());
    assert!(reopened.binding.cached_remote_image(url).unwrap().is_none());
    for _ in 0..2 {
        assert_eq!(
            core.body_remote_image(first.binding.clone(), 0)
                .await
                .unwrap()
                .bytes,
            bytes
        );
    }
    assert_eq!(
        version(),
        before,
        "reading an image must not write to SQLite"
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name='remote_images'",
            [],
            |row| row.get::<_, u32>(0)
        )
        .unwrap(),
        0
    );
    assert_missing(core.body_remote_image(first.binding.clone(), 1).await);
    db.execute(
        "UPDATE messages SET remote_state='deleted' WHERE id=?1",
        [&ids[0]],
    )
    .unwrap();
    assert_missing(core.body_remote_image(first.binding, 0).await);
}

#[tokio::test]
async fn ending_a_reading_drops_images_and_rejects_late_cache_writes() {
    let raw = b"Content-Type: text/html\r\n\r\n<img src=\"https://example.invalid/pixel\">";
    let (_directory, core, ids, _db) = fixture(&[raw]);
    let binding = core.body_document(ids[0].clone()).await.unwrap().binding;
    let pending = binding.clone();
    let (_, bytes) = synthetic_mail([0, 0, 255], "cache", "pixel");
    let image = render::validate_image(&bytes, "image/png").unwrap();
    let url = "https://example.invalid/pixel";
    binding.cache_remote_image(url.into(), &image).unwrap();
    binding.release_remote_images();
    binding.release_remote_images();
    assert!(pending.remote_images.lock().unwrap().images.is_empty());
    assert!(matches!(
        pending.cache_remote_image(url.into(), &image),
        Err(ImageFetchError::Cancelled)
    ));
    assert_missing(core.body_remote_image(pending, 0).await);
    let reopened = core.body_document(ids[0].clone()).await.unwrap().binding;
    assert!(reopened.cached_remote_image(url).unwrap().is_none());
    reopened.cache_remote_image(url.into(), &image).unwrap();
}

#[tokio::test]
async fn downloaded_message_images_persist_across_readings_but_never_bypass_identity() {
    let raw = b"Content-Type: text/html\r\n\r\n<img src=\"https://example.invalid/pixel\">";
    let (_directory, core, ids, db) = fixture(&[raw, raw]);
    let (_, bytes) = synthetic_mail([0, 0, 255], "cache", "pixel");
    let image = render::validate_image(&bytes, "image/png").unwrap();
    // The host fetched and validated this earlier; the host is unreachable now.
    let first = core.body_document(ids[0].clone()).await.unwrap();
    core.images.put(
        &remote_cache_key(&first.binding, "https://example.invalid/pixel"),
        &image,
    );
    assert_eq!(
        core.body_remote_image(first.binding.clone(), 0)
            .await
            .unwrap()
            .bytes,
        bytes
    );
    // Ending the reading drops its in-memory copy, not the persisted one.
    first.binding.release_remote_images();
    let reopened = core.body_document(ids[0].clone()).await.unwrap();
    let other = core.body_document(ids[1].clone()).await.unwrap();
    for binding in [reopened.binding.clone(), other.binding] {
        assert_eq!(
            core.body_remote_image(binding, 0).await.unwrap().bytes,
            bytes
        );
    }

    // A cache hit still requires a current binding and a visible message.
    assert_missing(core.body_remote_image(first.binding, 0).await);
    db.execute(
        "UPDATE messages SET remote_state='deleted' WHERE id=?1",
        [&ids[0]],
    )
    .unwrap();
    assert_missing(core.body_remote_image(reopened.binding, 0).await);

    // Clearing removes what was persisted: with no network the image is gone.
    core.clear_image_cache();
    let after = core.body_document(ids[1].clone()).await.unwrap();
    assert_missing(core.body_remote_image(after.binding, 0).await);
}

#[tokio::test]
async fn image_diagnostics_are_bound_and_never_include_remote_details() {
    let raw = b"Content-Type: text/html\r\n\r\n<img src=\"https://127.0.0.1/private?secret=private-marker\">";
    let (_directory, core, ids, db) = fixture(&[raw]);
    let binding = core.body_document(ids[0].clone()).await.unwrap().binding;
    assert!(
        core.body_image_status(binding.clone())
            .await
            .unwrap()
            .is_empty()
    );
    let (first, duplicate) = tokio::join!(
        core.body_remote_image_detailed(binding.clone(), 0),
        core.body_remote_image_detailed(binding.clone(), 0),
    );
    // This cached-only fixture deliberately has no connection profile, so a
    // cache miss fails before any network plan can be opened.
    assert!(matches!(first, Err(ImageFetchError::Cancelled)));
    assert!(matches!(duplicate, Err(ImageFetchError::Cancelled)));
    let status = core.body_image_status(binding.clone()).await.unwrap();
    assert_eq!(
        status,
        vec![RemoteImageStatus {
            index: 0,
            loading: false,
            error: Some(ImageFetchError::Cancelled)
        }]
    );
    let serialized = serde_json::to_string(&status).unwrap();
    assert!(!serialized.contains("private-marker"));
    assert!(!serialized.contains("127.0.0.1"));
    assert!(
        core.body_remote_image_detailed(binding.clone(), usize::MAX)
            .await
            .is_err()
    );
    assert_eq!(
        core.body_image_status(binding.clone()).await.unwrap(),
        status
    );
    db.execute("UPDATE accounts SET revision=revision+1", [])
        .unwrap();
    assert_missing(core.body_image_status(binding.clone()).await);
    binding.release_remote_images();
    assert!(binding.remote_images.lock().unwrap().statuses.is_empty());
}

#[tokio::test]
async fn remote_cache_does_not_cross_account_source_or_route_revisions() {
    let raw = b"Content-Type: text/html\r\n\r\n<img src=\"https://127.0.0.1/pixel\">";
    let (_directory, core, ids, db) = fixture(&[raw]);
    let binding = core.body_document(ids[0].clone()).await.unwrap().binding;
    let (_, bytes) = synthetic_mail([0, 0, 255], "cache", "pixel");
    let image = render::validate_image(&bytes, "image/png").unwrap();
    let url = "https://127.0.0.1/pixel";
    let key = remote_cache_key(&binding, url);
    core.images.put(&key, &image);
    assert!(core.body_remote_image(binding.clone(), 0).await.is_ok());
    let mut other = binding.clone();
    other.identity.account_id.push_str("-other");
    assert_ne!(key, remote_cache_key(&other, url));
    other = binding.clone();
    other.identity.mailbox_source_id.push_str("-other");
    assert_ne!(key, remote_cache_key(&other, url));
    db.execute("UPDATE accounts SET revision=revision+1", [])
        .unwrap();
    let current = core.body_document(ids[0].clone()).await.unwrap().binding;
    assert_ne!(key, remote_cache_key(&current, url));
    core.images.put(&format!("remote:{url}"), &image);
    assert!(matches!(
        core.body_remote_image_detailed(current, 0).await,
        Err(ImageFetchError::Cancelled)
    ));
}

#[tokio::test]
async fn releasing_a_reading_cancels_a_queued_image_before_its_request_slot_opens() {
    let url = "https://images.example.invalid/queued";
    let raw = format!("Content-Type: text/html\r\n\r\n<img src=\"{url}\">");
    let (_directory, core, ids, _db) = fixture(&[raw.as_bytes()]);
    let binding = core.body_document(ids[0].clone()).await.unwrap().binding;
    let request = Arc::new(tokio::sync::Mutex::new(()));
    binding
        .remote_images
        .lock()
        .unwrap()
        .requests
        .insert(url.into(), request.clone());
    let slot = request.lock_owned().await;
    let pending_core = core.clone();
    let pending_binding = binding.clone();
    let pending = tokio::spawn(async move {
        pending_core
            .body_remote_image_detailed(pending_binding, 0)
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if binding
                .remote_images
                .lock()
                .unwrap()
                .statuses
                .get(&0)
                .is_some_and(|status| status.loading)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    binding.release_remote_images();
    // Still hold the request slot: the whole loader, not merely its eventual
    // cache write, must stop immediately when the capability is revoked.
    let result = tokio::time::timeout(Duration::from_secs(2), pending)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result, Err(ImageFetchError::Cancelled)));
    drop(slot);
    let cache = binding.remote_images.lock().unwrap();
    assert!(cache.requests.is_empty() && cache.statuses.is_empty());
    assert!(cache.images.is_empty() && cache.failures.is_empty());
}

#[tokio::test]
async fn a_released_binding_cannot_start_a_later_image_request_or_restore_status() {
    let raw = b"Content-Type: text/html\r\n\r\n<img src=\"https://images.example.invalid/late\">";
    let (_directory, core, ids, _db) = fixture(&[raw]);
    let binding = core.body_document(ids[0].clone()).await.unwrap().binding;
    let late = binding.clone();
    binding.release_remote_images();
    assert!(matches!(
        core.body_remote_image_detailed(late.clone(), 0).await,
        Err(ImageFetchError::Cancelled)
    ));
    assert!(
        late.image_status(RemoteImageStatus {
            index: 0,
            loading: true,
            error: None
        })
        .is_err()
    );
    assert!(late.remote_images.lock().unwrap().statuses.is_empty());
    let fresh = core.body_document(ids[0].clone()).await.unwrap().binding;
    assert!(!*fresh.remote_cancelled.borrow());
}

#[tokio::test]
async fn retry_keeps_successes_without_disk_and_reopens_only_failed_resources() {
    let good_url = "https://images.example.invalid/good";
    let failed_url = "https://images.example.invalid/failed";
    let raw = format!(
        "Content-Type: text/html\r\n\r\n<img src=\"{good_url}\"><img src=\"{failed_url}\">"
    );
    let (_directory, core, ids, _db) = fixture(&[raw.as_bytes()]);
    let binding = core.body_document(ids[0].clone()).await.unwrap().binding;
    let (_, bytes) = synthetic_mail([10, 20, 30], "retained", "pixel");
    let image = render::validate_image(&bytes, "image/png").unwrap();
    binding.cache_remote_image(good_url.into(), &image).unwrap();
    assert!(core.body_remote_image(binding.clone(), 0).await.is_ok());
    // Represent a previously attempted download. The next read observes the
    // remembered error instead of opening another network request.
    binding
        .remote_images
        .lock()
        .unwrap()
        .failures
        .insert(failed_url.into(), ImageFetchError::Connect);
    assert!(matches!(
        core.body_remote_image_detailed(binding.clone(), 1).await,
        Err(ImageFetchError::Connect)
    ));
    core.clear_image_cache();
    let retry = core.body_retry_images(binding.clone()).await.unwrap();
    assert!(core.body_retry_images(binding.clone()).await.is_err());
    assert!(core.body_retry_images(retry.clone()).await.is_err());
    binding.release_remote_images();
    // A new document has independent cancellation and carries the successful
    // bytes itself. Without that memory, this credential-free fixture fails.
    assert_eq!(
        core.body_remote_image(retry.clone(), 0)
            .await
            .unwrap()
            .bytes,
        bytes
    );
    assert!(!*retry.remote_cancelled.borrow());
    assert!(retry.remote_images.lock().unwrap().failures.is_empty());
    // A recovered source can now be read rather than reusing its old failure.
    core.images
        .put(&remote_cache_key(&retry, failed_url), &image);
    assert_eq!(
        core.body_remote_image(retry.clone(), 1)
            .await
            .unwrap()
            .bytes,
        bytes
    );
    assert!(
        core.body_image_status(retry)
            .await
            .unwrap()
            .iter()
            .all(|status| status.error.is_none() && !status.loading)
    );
}

#[tokio::test]
async fn image_retry_rejects_pending_released_and_stale_readings() {
    let raw = b"Content-Type: text/html\r\n\r\n<img src=\"https://images.example.invalid/pixel\">";
    let (_directory, core, ids, db) = fixture(&[raw]);
    let binding = core.body_document(ids[0].clone()).await.unwrap().binding;
    binding
        .image_status(RemoteImageStatus {
            index: 0,
            loading: true,
            error: None,
        })
        .unwrap();
    assert!(core.body_retry_images(binding.clone()).await.is_err());
    binding.release_remote_images();
    assert_missing(core.body_retry_images(binding).await);
    let stale = core.body_document(ids[0].clone()).await.unwrap().binding;
    db.execute("UPDATE accounts SET revision=revision+1", [])
        .unwrap();
    assert_missing(core.body_retry_images(stale).await);
}

#[tokio::test]
async fn document_image_budget_rejects_successes_it_cannot_retain_for_retry() {
    let raw = b"Content-Type: text/html\r\n\r\n<p>budget fixture</p>";
    let (_directory, core, ids, _db) = fixture(&[raw]);
    let binding = core.body_document(ids[0].clone()).await.unwrap().binding;
    // Only the retaining budget is under test; network decoding has separate
    // per-image validation. Production cannot hand this helper unvalidated data.
    let image = InlineImage {
        bytes: vec![0; MAX_REMOTE_CACHE_BYTES],
        mime_type: "image/png".into(),
    };
    binding.cache_remote_image("full".into(), &image).unwrap();
    let extra = InlineImage {
        bytes: vec![0],
        mime_type: "image/png".into(),
    };
    assert!(matches!(
        binding.cache_remote_image("overflow".into(), &extra),
        Err(ImageFetchError::TooLarge)
    ));
    let cache = binding.remote_images.lock().unwrap();
    assert_eq!(cache.images.len(), 1);
    assert_eq!(
        cache.failures.get("overflow"),
        Some(&ImageFetchError::TooLarge)
    );
}

fn mail_with_attachments() -> Vec<u8> {
    let encoded = STANDARD.encode(b"%PDF-1.7 synthetic attachment bytes");
    format!(
        "MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=outer\r\n\r\n\
         --outer\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>hello</p>\r\n\
         --outer\r\nContent-Type: application/pdf; name=\"..\\\\..\\\\report.pdf\"\r\n\
         Content-Disposition: attachment; filename=\"..\\\\..\\\\report.pdf\"\r\n\
         Content-Transfer-Encoding: base64\r\n\r\n{encoded}\r\n\
         --outer\r\nContent-Type: image/png\r\nContent-Disposition: inline\r\n\
         Content-ID: <logo>\r\nContent-Transfer-Encoding: base64\r\n\r\n{}\r\n\
         --outer\r\nContent-Type: text/plain; charset=gbk\r\n\
         Content-Disposition: attachment; filename=\"note.txt\"\r\n\
         Content-Transfer-Encoding: base64\r\n\r\n{}\r\n\
         --outer--\r\n",
        STANDARD.encode(b"not displayed here"),
        STANDARD.encode([0xd5, 0xfd, 0xce, 0xc4]),
    )
    .into_bytes()
}

#[tokio::test]
async fn attachments_are_read_from_the_local_copy_with_a_safe_name() {
    let raw = mail_with_attachments();
    let (_directory, core, ids, _db) = fixture(&[&raw, &raw]);
    // The detail lists only real attachments, with ids this API accepts.
    let detail = core.get_email(&ids[0]).unwrap();
    let listed: Vec<_> = detail.attachments.iter().map(|a| a.id.clone()).collect();
    assert_eq!(listed.len(), 2, "{listed:?}");
    assert!(
        listed
            .iter()
            .all(|id| id.starts_with(&format!("{}:", ids[0])))
    );

    let pdf = core
        .attachment_content(ids[0].clone(), listed[0].clone())
        .await
        .unwrap();
    assert_eq!(pdf.bytes, b"%PDF-1.7 synthetic attachment bytes");
    assert_eq!(pdf.info.file_name, "report.pdf");
    assert_eq!(pdf.info.mime_type, "application/pdf");
    let info = core
        .attachment_info(ids[0].clone(), listed[0].clone())
        .await
        .unwrap();
    assert_eq!(info, pdf.info);

    // Text comes back as UTF-8, whatever charset the sender declared.
    let note = core
        .attachment_content(ids[0].clone(), listed[1].clone())
        .await
        .unwrap();
    assert_eq!(String::from_utf8(note.bytes).unwrap(), "正文");
    assert_eq!(note.info.file_name, "note.txt");
}

#[tokio::test]
async fn only_listed_attachments_of_the_named_message_can_be_read() {
    let raw = mail_with_attachments();
    let (_directory, core, ids, _db) = fixture(&[&raw, &raw]);
    let listed = core.get_email(&ids[0]).unwrap().attachments;
    let id = listed[0].id.clone();
    let other = listed[0].id.replace(&ids[0], &ids[1]);
    let index: usize = id.rsplit(':').next().unwrap().parse().unwrap();
    for bad in [
        // Another message's id with this message's attachment.
        (ids[1].clone(), id.clone()),
        // The inline image and the body part are not attachments.
        (ids[0].clone(), format!("{}:{}", ids[0], index + 1)),
        (ids[0].clone(), format!("{}:1", ids[0])),
        // Malformed or out-of-range ids.
        (ids[0].clone(), format!("{}:{}", ids[0], 999)),
        (ids[0].clone(), format!("{}:-1", ids[0])),
        (ids[0].clone(), format!("{}:", ids[0])),
        (ids[0].clone(), format!("{}: 1", ids[0])),
        (ids[0].clone(), format!("{}:01x", ids[0])),
        (ids[0].clone(), ids[0].clone()),
        (ids[0].clone(), String::new()),
        (ids[0].clone(), "../../x:1".into()),
        ("not-a-uuid".into(), "not-a-uuid:1".into()),
    ] {
        let label = format!("{bad:?}");
        assert_missing_or_invalid(
            core.attachment_content(bad.0.clone(), bad.1.clone()).await,
            &label,
        );
        assert_missing_or_invalid(core.attachment_info(bad.0, bad.1).await.map(|_| ()), &label);
    }
    // Same bytes, but each message answers only for itself.
    assert!(core.attachment_content(ids[1].clone(), other).await.is_ok());
}

fn assert_missing_or_invalid<T>(result: Result<T, AppError>, label: &str) {
    match result {
        Ok(_) => panic!("unexpectedly readable: {label}"),
        Err(error) => assert!(
            matches!(error.code, ErrorCode::NotFound | ErrorCode::Validation),
            "{label}: {:?}",
            error.code
        ),
    }
}

#[tokio::test]
async fn attachments_are_not_handed_out_once_the_message_is_gone() {
    let raw = mail_with_attachments();
    let (_directory, core, ids, db) = fixture(&[&raw]);
    let id = core.get_email(&ids[0]).unwrap().attachments[0].id.clone();
    assert!(
        core.attachment_content(ids[0].clone(), id.clone())
            .await
            .is_ok()
    );
    db.execute(
        "UPDATE messages SET remote_state='deleted' WHERE id=?1",
        [&ids[0]],
    )
    .unwrap();
    assert_missing(core.attachment_content(ids[0].clone(), id.clone()).await);
    assert_missing(core.attachment_info(ids[0].clone(), id).await);
}
