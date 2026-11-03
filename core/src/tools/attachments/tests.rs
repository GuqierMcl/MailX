use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use serde_json::{Value, json};
use tempfile::TempDir;

use super::*;
use crate::tools::{AccountScope, Grant, RequestContext};
use crate::{Caller, CoreEvent, CreateDraftRequest, MailAddress, MailxCore, QueueSendRequest};

struct Fixture {
    directory: TempDir,
    core: MailxCore,
    tools: Tools,
    draft: Draft,
    changes: Arc<Mutex<usize>>,
}

fn fixture() -> Fixture {
    let directory = TempDir::new().unwrap();
    let changes = Arc::new(Mutex::new(0));
    let sink = changes.clone();
    let (core, emails) = MailxCore::body_test_fixture_with_events(
        directory.path(),
        tokio::runtime::Handle::current(),
        &[b"Content-Type: text/plain\r\n\r\nSynthetic"],
        Arc::new(move |event: &CoreEvent| {
            if matches!(event, CoreEvent::DraftChanged(_)) {
                *sink.lock().unwrap() += 1;
            }
        }),
    )
    .unwrap();
    // Queue-state tests never start a send worker or contact a server.
    core.begin_shutdown();
    let account = core.get_email(&emails[0]).unwrap().summary.account_id;
    let draft = core
        .create_draft(
            CreateDraftRequest {
                account_id: Some(account),
                to: vec![MailAddress {
                    name: None,
                    address: "recipient@example.invalid".into(),
                }],
                cc: vec![],
                bcc: vec![],
                subject: "Synthetic".into(),
                body_html: "<p>Test</p>".into(),
                client_mutation_id: uuid::Uuid::new_v4().to_string(),
            },
            "",
        )
        .unwrap();
    *changes.lock().unwrap() = 0;
    let tools = Tools::new(core.clone(), String::new);
    Fixture {
        directory,
        core,
        tools,
        draft,
        changes,
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
    .map(|principal| RequestContext::new(principal, Grant::mcp_default(), "attachment-test"))
}

fn input(f: &Fixture) -> Value {
    json!({
        "draftId": f.draft.id,
        "expectedRevision": f.draft.revision,
        "filePath": f.directory.path().join("report.txt"),
    })
}

#[tokio::test]
async fn attachment_round_trip_preserves_provenance_and_source_file() {
    for ctx in contexts() {
        let f = fixture();
        let path = f.directory.path().join("report.txt");
        std::fs::write(&path, b"Report contents").unwrap();
        let result = f
            .tools
            .call(&ctx, "add_draft_attachment", input(&f))
            .unwrap();
        assert_eq!(result["revision"], f.draft.revision + 1);
        assert_eq!(result["attachment"]["name"], "report.txt");
        assert_eq!(result["attachment"]["size"], 15);
        assert_eq!(result["attachment"]["inline"], false);
        assert_eq!(
            result["attachment"]["origin"],
            match ctx.principal {
                Principal::LocalMcp { .. } => "mcp",
                Principal::BuiltinAgent { .. } => "ai",
            }
        );
        assert!(!result.to_string().contains(path.to_str().unwrap()));
        let read = f
            .tools
            .call(&ctx, "read_draft", json!({"draftId": f.draft.id}))
            .unwrap();
        assert_eq!(read["attachments"][0], result["attachment"]);
        let removed = f
            .tools
            .call(
                &ctx,
                "remove_draft_attachment",
                json!({
                    "draftId": f.draft.id,
                    "expectedRevision": result["revision"],
                    "attachmentId": result["attachment"]["id"],
                }),
            )
            .unwrap();
        assert_eq!(removed["removed"], true);
        assert_eq!(removed["revision"], f.draft.revision + 2);
        assert!(
            f.core
                .get_draft(&f.draft.id)
                .unwrap()
                .attachments
                .is_empty()
        );
        assert_eq!(std::fs::read(path).unwrap(), b"Report contents");
        assert_eq!(*f.changes.lock().unwrap(), 2);
    }
}

#[tokio::test]
async fn scope_and_grant_are_checked_before_the_file_is_opened() {
    let f = fixture();
    let mut ctx = contexts()[0].clone();
    ctx.grant = Grant::new(
        AccountScope::Only(vec![]),
        [Action::DraftWrite, Action::FilesRead],
    );
    assert_eq!(
        f.tools
            .call(&ctx, "add_draft_attachment", input(&f))
            .unwrap_err()
            .code,
        ToolErrorCode::NotFound
    );
    ctx.grant = Grant::new(AccountScope::All, [Action::MailRead]);
    assert_eq!(
        f.tools
            .call(&ctx, "add_draft_attachment", input(&f))
            .unwrap_err()
            .code,
        ToolErrorCode::NotPermitted
    );
    ctx.grant = Grant::new(AccountScope::All, [Action::DraftWrite]);
    assert_eq!(
        f.tools
            .call(&ctx, "add_draft_attachment", input(&f))
            .unwrap_err()
            .code,
        ToolErrorCode::NotPermitted
    );
    assert!(
        f.core
            .get_draft(&f.draft.id)
            .unwrap()
            .attachments
            .is_empty()
    );
    assert_eq!(*f.changes.lock().unwrap(), 0);
}

#[tokio::test]
async fn stale_open_and_queued_drafts_fail_before_file_io() {
    let f = fixture();
    let ctx = contexts()[0].clone();
    let mut stale = input(&f);
    stale["expectedRevision"] = json!(f.draft.revision + 1);
    assert_eq!(
        f.tools
            .call(&ctx, "add_draft_attachment", stale)
            .unwrap_err()
            .code,
        ToolErrorCode::Conflict
    );
    let open = Tools::new(f.core.clone(), String::new).with_draft_open_check(|_| true);
    assert_eq!(
        open.call(&ctx, "add_draft_attachment", input(&f))
            .unwrap_err()
            .code,
        ToolErrorCode::DraftOpen
    );
    f.core
        .queue_send(
            Caller::Composer,
            QueueSendRequest {
                draft_id: f.draft.id.clone(),
                expected_revision: f.draft.revision,
                client_mutation_id: uuid::Uuid::new_v4().to_string(),
            },
            30,
        )
        .unwrap();
    assert_eq!(
        f.tools
            .call(&ctx, "add_draft_attachment", input(&f))
            .unwrap_err()
            .code,
        ToolErrorCode::DraftLocked
    );
    assert!(
        f.core
            .get_draft(&f.draft.id)
            .unwrap()
            .attachments
            .is_empty()
    );
}

#[tokio::test]
async fn an_editor_opened_during_file_io_prevents_the_change() {
    let f = fixture();
    std::fs::write(f.directory.path().join("report.txt"), b"Test").unwrap();
    let checks = Arc::new(AtomicUsize::new(0));
    let seen = checks.clone();
    let tools = Tools::new(f.core.clone(), String::new)
        .with_draft_open_check(move |_| seen.fetch_add(1, Ordering::SeqCst) > 0);
    assert_eq!(
        tools
            .call(&contexts()[0], "add_draft_attachment", input(&f))
            .unwrap_err()
            .code,
        ToolErrorCode::DraftOpen
    );
    assert_eq!(checks.load(Ordering::SeqCst), 2);
    assert!(
        f.core
            .get_draft(&f.draft.id)
            .unwrap()
            .attachments
            .is_empty()
    );
    assert_eq!(*f.changes.lock().unwrap(), 0);
}

#[tokio::test]
async fn names_are_cleaned_and_callers_cannot_forge_origin() {
    let f = fixture();
    std::fs::write(f.directory.path().join("report.txt"), b"Test").unwrap();
    let ctx = &contexts()[0];
    let mut forged = input(&f);
    forged["origin"] = json!("user");
    assert_eq!(
        f.tools
            .call(ctx, "add_draft_attachment", forged)
            .unwrap_err()
            .code,
        ToolErrorCode::InvalidInput
    );
    let mut renamed = input(&f);
    renamed["fileName"] = json!("../../final.txt");
    let added = f.tools.call(ctx, "add_draft_attachment", renamed).unwrap();
    assert_eq!(added["attachment"]["name"], "final.txt");
    assert_eq!(added["attachment"]["origin"], "mcp");
    let mut removal = json!({"draftId": f.draft.id, "expectedRevision": f.draft.revision,
        "attachmentId": added["attachment"]["id"]});
    assert_eq!(
        f.tools
            .call(ctx, "remove_draft_attachment", removal.clone())
            .unwrap_err()
            .code,
        ToolErrorCode::Conflict
    );
    removal["expectedRevision"] = added["revision"].clone();
    removal["attachmentId"] = json!(uuid::Uuid::new_v4().to_string());
    assert_eq!(
        f.tools
            .call(ctx, "remove_draft_attachment", removal)
            .unwrap_err()
            .code,
        ToolErrorCode::NotFound
    );
    assert_eq!(f.core.get_draft(&f.draft.id).unwrap().attachments.len(), 1);
}

#[tokio::test]
async fn inline_images_cannot_be_removed_through_the_regular_attachment_tool() {
    let f = fixture();
    let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        1,
        1,
        image::Rgba([0, 0, 0, 255]),
    ));
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let (draft, images) = f
        .core
        .add_inline_images(
            &f.draft.id,
            f.draft.revision,
            vec![("image.png".into(), png.into_inner())],
        )
        .unwrap();
    let result = f.tools.call(&contexts()[0], "remove_draft_attachment", json!({
        "draftId": draft.id, "expectedRevision": draft.revision, "attachmentId": images[0].attachment_id,
    }));
    assert_eq!(result.unwrap_err().code, ToolErrorCode::InvalidInput);
    assert_eq!(f.core.get_draft(&draft.id).unwrap().attachments.len(), 1);
}

#[test]
fn local_file_reads_are_bounded_and_errors_do_not_expose_paths() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("secret-name.txt");
    assert_eq!(
        read_file(dir.path(), 10).unwrap_err().code,
        ToolErrorCode::InvalidInput
    );
    let missing = read_file(&path, 10).unwrap_err();
    assert_eq!(missing.code, ToolErrorCode::FileUnavailable);
    assert!(!missing.message.contains("secret-name"));
    assert!(validate_path("relative.txt").is_err());
    std::fs::write(&path, b"123456").unwrap();
    assert_eq!(
        read_file(&path, 5).unwrap_err().code,
        ToolErrorCode::AttachmentTooLarge
    );
    assert_eq!(read_file(&path, 6).unwrap(), b"123456");
}

#[cfg(unix)]
#[test]
fn unix_rejects_symlinks_devices_and_fifos_without_blocking() {
    use std::os::unix::fs::symlink;

    let dir = TempDir::new().unwrap();
    let target = dir.path().join("target.txt");
    let link = dir.path().join("link.txt");
    std::fs::write(&target, b"test").unwrap();
    symlink(&target, &link).unwrap();
    assert_eq!(
        read_file(&link, 100).unwrap_err().code,
        ToolErrorCode::InvalidInput
    );
    assert_eq!(
        read_file(Path::new("/dev/null"), 100).unwrap_err().code,
        ToolErrorCode::InvalidInput
    );
    let fifo = dir.path().join("fifo");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(
        read_file(&fifo, 100).unwrap_err().code,
        ToolErrorCode::InvalidInput
    );
    // Simulate the object seen by open after a metadata/open race. This returns
    // immediately without a writer, then the caller's handle check rejects it.
    assert!(!open_file(&fifo).unwrap().metadata().unwrap().is_file());
}

#[cfg(windows)]
#[test]
fn windows_accepts_disk_paths_but_not_networks_devices_or_streams() {
    for path in [r"C:\Users\test\report.txt", r"\\?\C:\Users\test\report.txt"] {
        assert!(validate_path(path).is_ok());
    }
    for path in [
        r"\\server\share\report.txt",
        r"\\?\UNC\server\share\report.txt",
        r"\\.\pipe\service",
        r"C:\NUL",
        r"C:\con.txt",
        r"C:\COM1",
        r"C:\report.txt:secret",
    ] {
        assert!(validate_path(path).is_err(), "{path}");
    }
}
