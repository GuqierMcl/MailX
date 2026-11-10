use super::*;
use serde_json::json;
use std::time::Duration;

/// Construct metadata without connecting to a release server or running an installer.
fn update() -> Update {
    let mut context = tauri::test::mock_context(tauri::test::noop_assets());
    context.config_mut().plugins.0.insert(
        "updater".into(),
        json!({ "pubkey": "unused-test-key", "endpoints": ["https://example.invalid/update"] }),
    );
    let app = tauri::test::mock_builder()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .build(context)
        .unwrap();
    app.updater_builder()
        .target("transition-test")
        .executable_path("runtime-path/app")
        .version_comparator(|_, _| true)
        .no_proxy()
        .configure_client(|_| panic!("transition tests must not use the network"))
        .build()
        .unwrap()
        .restore_update(json!({
            "version": "2.0.0",
            "notes": "Release notes",
            "platforms": {
                "transition-test": {
                    "url": "https://example.invalid/package",
                    "signature": "unused-test-signature"
                }
            }
        }))
        .unwrap()
        .unwrap()
}

fn available() -> Inner {
    let update = update();
    let mut inner = Inner {
        state: Some(UpdateState::Available {
            version: update.version.clone(),
            details: release_details(&update),
        }),
        update: Some(update),
        ..Default::default()
    };
    assert!(inner.apply_settings(1, true));
    inner
}

fn ready(manual: bool) -> Inner {
    let mut inner = available();
    let update = inner.update.as_ref().unwrap();
    inner.state = Some(UpdateState::Ready {
        version: update.version.clone(),
        details: release_details(update),
    });
    inner.package = Some(Arc::new(b"verified package".to_vec()));
    inner.download_manual = manual;
    inner.install_on_quit = true;
    inner
}

#[test]
fn a_switch_change_between_scheduling_and_start_prevents_an_automatic_download() {
    let mut inner = available();
    let now = Instant::now();
    assert!(inner.automatic.may_start("2.0.0", now));

    assert!(inner.apply_settings(2, false));
    assert!(inner.begin_download(false, now).is_none());
    assert!(matches!(inner.state(), UpdateState::Available { .. }));
    assert!(
        inner.state().may_check(),
        "turning off downloads must not stop checks"
    );

    assert!(inner.apply_settings(3, true));
    assert!(inner.automatic.may_start("2.0.0", now));
    inner.automatic.pause("2.0.0");
    assert!(inner.begin_download(false, now).is_none());
    assert!(
        inner.begin_download(true, now).is_some(),
        "explicit retry overrides cancellation"
    );
}

#[test]
fn switching_off_cancels_automatic_work_but_preserves_manual_downloads() {
    let now = Instant::now();
    for manual in [false, true] {
        let mut inner = available();
        let (_, generation) = inner.begin_download(manual, now).unwrap();
        assert!(inner.apply_settings(2, false));
        if manual {
            assert_eq!(inner.generation, generation);
            assert!(matches!(inner.state(), UpdateState::Downloading { .. }));
            assert!(inner.download_manual);
        } else {
            assert_ne!(
                inner.generation, generation,
                "late automatic completion is invalidated"
            );
            assert!(matches!(inner.state(), UpdateState::Available { .. }));
            assert!(!inner.install_on_quit);
            assert!(inner.begin_download(false, now).is_none());
        }
    }
}

#[test]
fn disabling_prepared_automatic_updates_keeps_the_package_without_quit_install() {
    for manual in [false, true] {
        let mut inner = ready(manual);
        let package = inner.package.clone().unwrap();
        assert!(inner.apply_settings(2, false));
        assert!(inner.state().may_restart());
        assert!(Arc::ptr_eq(inner.package.as_ref().unwrap(), &package));
        assert_eq!(inner.install_on_quit, manual);
        assert!(inner.apply_settings(3, true));
        assert!(inner.install_on_quit);
    }
}

#[test]
fn old_or_duplicate_settings_reads_cannot_reenable_downloads_or_installation() {
    let mut inner = ready(false);
    assert!(inner.apply_settings(4, false));
    for revision in [1, 3, 4] {
        assert!(!inner.apply_settings(revision, true));
        assert!(!inner.automatic.enabled);
        assert!(!inner.install_on_quit);
        assert_eq!(inner.settings_revision, Some(4));
    }
}

#[test]
fn a_due_retry_waits_for_an_inflight_check_without_reusing_an_expired_timer() {
    let mut inner = available();
    let update = inner.update.as_ref().unwrap();
    inner.state = Some(UpdateState::Failed {
        stage: FailureStage::Download,
        version: Some(update.version.clone()),
        details: Some(release_details(update)),
    });
    let now = Instant::now();
    inner.automatic.failed(now - Duration::from_secs(120), true);
    assert!(inner.retry_deadline().unwrap() < now);

    inner.checking = true;
    assert!(inner.retry_deadline().is_none());
    assert!(inner.begin_download(false, now).is_none());

    inner.checking = false;
    assert!(inner.retry_deadline().is_some());
    inner.begin_download(false, now).unwrap();
    assert!(
        inner.retry_deadline().is_none(),
        "an active download must not keep a due retry timer"
    );
    assert!(
        inner.begin_download(false, now).is_none(),
        "only one transfer may start"
    );
}

#[test]
fn a_cancelled_generation_cannot_be_reused_and_manual_retry_gets_a_new_generation() {
    let mut inner = available();
    let now = Instant::now();
    let (_, cancelled_generation) = inner.begin_download(false, now).unwrap();
    inner.cancel_download(true);
    assert_ne!(inner.generation, cancelled_generation);
    assert!(inner.automatic.is_paused("2.0.0"));
    assert!(!inner.install_on_quit);
    assert!(
        inner.apply_settings(2, true),
        "an unrelated settings revision arrives"
    );
    assert!(
        inner
            .begin_download(false, now + next_wait(false))
            .is_none()
    );

    let (_, retry_generation) = inner.begin_download(true, now).unwrap();
    assert!(retry_generation > cancelled_generation);
    assert!(!inner.automatic.is_paused("2.0.0"));
    assert!(inner.download_manual);
}

#[test]
fn failed_install_targets_require_explicit_retry_even_after_a_check_changes_the_visible_state() {
    for failed_state in [false, true] {
        let mut inner = available();
        inner.install_failure = Some("2.0.0".into());
        if failed_state {
            let update = inner.update.as_ref().unwrap();
            inner.state = Some(UpdateState::Failed {
                stage: FailureStage::Install,
                version: Some(update.version.clone()),
                details: Some(release_details(update)),
            });
        }
        let now = Instant::now();
        assert!(inner.automatic.may_start("2.0.0", now));
        assert!(inner.begin_download(false, now).is_none());
        assert!(inner.begin_download(true, now).is_some());
        assert!(inner.download_manual);
    }
    let mut different_release = available();
    different_release.install_failure = Some("1.5.0".into());
    assert!(
        different_release
            .begin_download(false, Instant::now())
            .is_some()
    );
}

#[test]
fn explicitly_reenabling_updates_resumes_cancelled_or_failed_manual_downloads() {
    for cancelled in [false, true] {
        let mut inner = available();
        let now = Instant::now();
        inner.begin_download(true, now).unwrap();
        if cancelled {
            inner.cancel_download(true);
            assert!(inner.automatic.is_paused("2.0.0"));
            assert!(!inner.download_manual);
            assert!(!inner.pending_manual);
        } else {
            let update = inner.update.as_ref().unwrap();
            inner.state = Some(UpdateState::Failed {
                stage: FailureStage::Download,
                version: Some(update.version.clone()),
                details: Some(release_details(update)),
            });
            assert!(inner.download_manual);
        }
        assert!(
            inner
                .begin_download(false, now + next_wait(false))
                .is_none()
        );
        assert!(inner.apply_settings(2, false));
        assert!(
            inner
                .begin_download(false, now + next_wait(false))
                .is_none()
        );
        assert!(inner.apply_settings(3, true));
        assert!(!inner.automatic.is_paused("2.0.0"));
        inner.begin_download(false, now).unwrap();
        assert!(!inner.download_manual);
        assert!(matches!(inner.state(), UpdateState::Downloading { .. }));
    }
}

#[test]
fn explicit_download_waits_for_the_check_and_cannot_be_consumed_as_automatic_work() {
    let mut inner = available();
    let now = Instant::now();
    inner.checking = true;
    assert!(inner.begin_download(true, now).is_none());
    assert!(inner.pending_manual);
    assert!(matches!(inner.state(), UpdateState::Checking));

    assert!(inner.apply_settings(2, false));
    assert!(
        inner.pending_manual,
        "turning off automatic work preserves the explicit request"
    );
    inner.checking = false;
    let update = inner.update.as_ref().unwrap();
    inner.state = Some(UpdateState::Available {
        version: update.version.clone(),
        details: release_details(update),
    });
    assert!(inner.apply_settings(3, true));
    assert!(inner.automatic.may_start("2.0.0", now));
    assert!(inner.begin_download(false, now).is_none());
    assert!(inner.pending_manual);

    inner.begin_download(true, now).unwrap();
    assert!(inner.download_manual);
    assert!(!inner.pending_manual);
    assert!(matches!(inner.state(), UpdateState::Downloading { .. }));
}

#[test]
fn an_explicit_download_promotes_an_existing_automatic_transfer_without_restarting_it() {
    let mut inner = available();
    let now = Instant::now();
    let (_, generation) = inner.begin_download(false, now).unwrap();
    assert!(!inner.download_manual);
    assert!(
        inner.begin_download(true, now).is_none(),
        "the existing transfer is reused"
    );
    assert_eq!(inner.generation, generation);
    assert!(inner.download_manual);
    assert!(!inner.pending_manual);

    assert!(inner.apply_settings(2, false));
    assert_eq!(inner.generation, generation);
    assert!(matches!(inner.state(), UpdateState::Downloading { .. }));
    assert!(inner.download_manual);
}

#[test]
fn restart_preflight_cannot_arm_a_replaced_or_withdrawn_package() {
    let mut replaced = ready(false);
    let preflight_generation = replaced.generation;
    replaced.generation += 1;
    assert!(!replaced.arm_restart(preflight_generation));
    assert!(!replaced.restart_after_install);

    let mut withdrawn = ready(false);
    let preflight_generation = withdrawn.generation;
    withdrawn.state = Some(UpdateState::Idle);
    assert!(!withdrawn.arm_restart(preflight_generation));
    assert!(!withdrawn.restart_after_install);

    let mut current = ready(false);
    assert!(current.arm_restart(current.generation));
    assert!(current.restart_after_install);
}

#[test]
fn only_plain_https_links_open() {
    assert!(https_link("https://github.com/GuqierMcl/MailX/releases").is_some());
    for bad in [
        "http://example.com",
        "javascript:alert(1)",
        "data:text/html,x",
        "file:///etc/passwd",
        "https://user:pw@example.com",
        "mailto:a@b.c",
        "not a url",
    ] {
        assert!(https_link(bad).is_none(), "{bad}");
    }
}
