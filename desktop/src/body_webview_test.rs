//! Synthetic-only WebView test support. Register this module exclusively behind
//! the non-default `body-webview-test` Cargo feature. Production builds must not
//! compile this module, the native observer, the test assets or WebDriver plugin.
//!
//! The integration caller supplies URLs from the SAME body protocol handler as
//! production. Do not add a second protocol implementation to make tests pass.
use std::collections::BTreeMap;

use serde::Serialize;
use tauri::{AppHandle, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub(crate) const TITLE: &str = "MailX Body WebView Test";

const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC";

pub(crate) struct Fixture {
    pub name: &'static str,
    pub raw: Vec<u8>,
}

/// Separate startup branch. It never constructs SystemSecretStore and never
/// starts synchronization for any user account. The core helper rejects an
/// existing/nonempty profile, and its secret implementation panics on access.
pub(crate) fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    use std::sync::Arc;
    use tauri::Manager;

    let root = isolated_root()?;
    let canary = std::env::var("MAILX_BODY_CANARY_ORIGIN")?;
    let fixtures = fixtures(&canary)?;
    let raws: Vec<&[u8]> = fixtures
        .iter()
        .map(|fixture| fixture.raw.as_slice())
        .collect();
    let tauri::async_runtime::RuntimeHandle::Tokio(runtime) = tauri::async_runtime::handle();
    let (core, ids) =
        mailx_core::MailxCore::body_test_fixture(&root.join("profile"), runtime, &raws)?;
    app.manage(crate::state::DesktopState::for_body_test(core));
    let origin = if cfg!(target_os = "windows") {
        "http://mailx-body.localhost"
    } else {
        "mailx-body://localhost"
    };
    let mut documents = BTreeMap::new();
    for (fixture, id) in fixtures.iter().zip(ids) {
        if fixture.name == "policyProbe" {
            app.state::<Arc<crate::body_protocol::BodyProtocol>>()
                .configure_test_policy_probe(id.clone(), canary.clone());
        }
        documents.insert(fixture.name.to_owned(), format!("{origin}/document/{id}"));
    }
    create_window(app.handle(), documents)?;
    Ok(())
}

/// Prepare MIME bytes for the real cache/provider path. These data are entirely
/// synthetic and contain neither credentials nor a connection to an email host.
pub(crate) fn fixtures(canary_origin: &str) -> Result<Vec<Fixture>, &'static str> {
    validate_canary(canary_origin)?;
    let safe = "<h1>MAILX-SAFE-FIXTURE</h1><p>合成中文正文</p>\
        <a style=\"display:block;width:320px;height:48px;background-color:#ddffdd\" href=\"https://example.invalid/confirmed-link\">Open synthetic link</a>\
        <img src=\"cid:pixel\" width=\"96\" height=\"96\" alt=\"Synthetic red pixel\">";
    let malicious = format!(
        "<h1>MAILX-MALICIOUS-FIXTURE</h1><script>{}</script>\
         <img src=\"{canary_origin}/sanitizer-image\" onerror=\"window.__MAILX_PAGE_SCRIPT_EXECUTED=true\">\
         <img srcset=\"{canary_origin}/sanitizer-srcset 1x\">\
         <p style=\"background:url({canary_origin}/sanitizer-css)\">safe text</p>\
         <meta http-equiv=\"refresh\" content=\"0;url={canary_origin}/navigation\">\
         <a href=\"javascript:window.__MAILX_PAGE_SCRIPT_EXECUTED=true\">blocked</a>\
         <img src=\"cid:pixel\" width=\"96\" height=\"96\">",
        attack_script(canary_origin)
    );
    Ok(vec![
        Fixture {
            name: "safe",
            raw: mime(safe),
        },
        Fixture {
            name: "malicious",
            raw: mime(&malicious),
        },
        // The cache still contains a real rendered document. The ONLY test
        // bypass is replacement of the served HTML for policyProbe, below.
        Fixture {
            name: "policyProbe",
            raw: mime(safe),
        },
    ])
}

/// Deliberately bypass the sanitizer in the test build to challenge the browser
/// security boundary independently. The caller MUST use production response
/// headers and URL binding, and must never expose this branch without the feature.
/// `part_url` is a bound protocol URL supplied by the production handler.
pub(crate) fn policy_probe_html(
    canary_origin: &str,
    part_url: &str,
    link_url: &str,
) -> Result<String, &'static str> {
    validate_canary(canary_origin)?;
    if !part_url.starts_with("mailx-body://localhost/part/")
        && !part_url.starts_with("http://mailx-body.localhost/part/")
    {
        return Err("invalid test image URL");
    }
    let part_url = part_url
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;");
    let link_url = link_url
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;");
    Ok(format!(
        "<!doctype html><html><body><h1>MAILX-POLICY-PROBE</h1>\
         <script>{}</script><img src=\"{canary_origin}/policy-image\" onerror=\"window.__MAILX_PAGE_SCRIPT_EXECUTED=true\">\
         <style>@import url({canary_origin}/policy-import); p{{background:url({canary_origin}/policy-css)}}</style>\
         <p>Native sandbox and CSP probe</p>\
         <img src=\"{link_url}\" alt=\"A link route must not be usable as an image\">\
         <img data-test-cid src=\"{part_url}\" width=\"96\" height=\"96\">\
         </body></html>",
        attack_script(canary_origin)
    ))
}

fn attack_script(canary: &str) -> String {
    format!(
        "window.__MAILX_PAGE_SCRIPT_EXECUTED=true;\
         document.documentElement.dataset.mailxPageScriptExecuted='true';\
         document.querySelector('h1').textContent='MAILX-PAGE-SCRIPT-EXECUTED';\
         try{{parent.__MAILX_TEST_SECRET='compromised'}}catch(e){{}}\
         try{{window.__TAURI_INTERNALS__.invoke('app_get_startup_status',{{}})}}catch(e){{}}\
         try{{parent.__TAURI_INTERNALS__.invoke('accounts_list',{{}})}}catch(e){{}}\
         parent.postMessage({{kind:'mailx://link-request',emailId:'forged',index:0,url:'{canary}/forged'}},'*');\
         fetch('{canary}/script-ran');"
    )
}

fn mime(html: &str) -> Vec<u8> {
    format!(
        "MIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=mailx-test\r\n\r\n\
         --mailx-test\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{html}\r\n\
         --mailx-test\r\nContent-Type: image/png\r\nContent-ID: <pixel>\r\n\
         Content-Disposition: inline; filename=pixel.png\r\nContent-Transfer-Encoding: base64\r\n\r\n\
         {PNG}\r\n--mailx-test--\r\n"
    ).into_bytes()
}

fn validate_canary(origin: &str) -> Result<(), &'static str> {
    let port = origin
        .strip_prefix("http://127.0.0.1:")
        .ok_or("test canary must be loopback")?;
    if port.parse::<u16>().is_ok_and(|port| port != 0) {
        Ok(())
    } else {
        Err("invalid test canary port")
    }
}

/// Requires an isolated directory created by scripts/body-webview.mjs. The app
/// must select it before initializing any store, synchronization or SecretStore.
pub(crate) fn isolated_root() -> Result<std::path::PathBuf, &'static str> {
    let root = std::env::var_os("MAILX_BODY_WEBVIEW_ROOT")
        .map(std::path::PathBuf::from)
        .ok_or("test data root is required")?;
    if !root.is_absolute() || !root.join(".mailx-synthetic-webview-test").is_file() {
        return Err("test data root marker is missing");
    }
    Ok(root)
}

pub(crate) fn create_window(
    app: &AppHandle,
    documents: BTreeMap<String, String>,
) -> Result<WebviewWindow, Box<dyn std::error::Error>> {
    let nonce = std::env::var("MAILX_BODY_WEBVIEW_NONCE")?;
    if nonce.len() != 64 || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("invalid test nonce".into());
    }
    let config = serde_json::json!({ "nonce": nonce, "documents": documents, "numberedLinks": cfg!(not(target_os = "windows")) });
    let bootstrap = format!("window.__MAILX_BODY_TEST_CONFIG={config};");
    Ok(
        WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
            .title(TITLE)
            .inner_size(900.0, 700.0)
            .position(100.0, 100.0)
            .decorations(false)
            .resizable(false)
            .initialization_script(bootstrap)
            .build()?,
    )
}

/// Call after the production handler has formed its response. Paths/tokens and
/// raw email content are intentionally excluded, even for synthetic messages.
pub(crate) fn record_response(app: &AppHandle, route: &str, email_id: Option<&str>, status: u16) {
    use tauri::Manager;
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Record<'a> {
        route: &'a str,
        email_id: Option<&'a str>,
        status: u16,
    }
    let Ok(record) = serde_json::to_string(&Record {
        route,
        email_id,
        status,
    }) else {
        return;
    };
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.eval(format!(
            "window.__MAILX_BODY_TEST_RECORD_REQUEST?.({record});"
        ));
    }
}
