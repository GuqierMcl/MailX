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
    assert_missing(pending.cache_remote_image(url.into(), &image));
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
    core.images
        .put("remote:https://example.invalid/pixel", &image);

    let first = core.body_document(ids[0].clone()).await.unwrap();
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
