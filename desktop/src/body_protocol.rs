//! Isolated, read-only body resources. Protocol GET can request confirmation but
//! never opens an external application; only the trusted command can do that.
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use mailx_core::{
    AppError, BodyBinding, ErrorCode, MailxCore,
    mail::render::{self, ResourceBinding},
};
use serde::Serialize;
use tauri::{
    Url,
    http::{Method, Request, Response, StatusCode},
};
use uuid::Uuid;

const TOKEN_LIFETIME: Duration = Duration::from_secs(30 * 60);
const MAX_DOCUMENTS: usize = 128;
pub(crate) const LINK_EVENT: &str = "mailx://link-request";
/// The origin body frames load from. wry hands the handler a request rewritten
/// to `mailx-body://localhost` on every platform, so bound URLs must come from
/// here rather than from the request.
pub(crate) const BODY_ORIGIN: &str = if cfg!(target_os = "windows") {
    "http://mailx-body.localhost"
} else {
    "mailx-body://localhost"
};

pub(crate) fn register(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    use std::sync::Arc;
    use tauri::{Emitter, EventTarget, Manager};
    builder
        .manage(Arc::new(BodyProtocol::default()))
        .register_asynchronous_uri_scheme_protocol("mailx-body", |context, request, responder| {
            let app = context.app_handle().clone();
            let label = context.webview_label().to_owned();
            let protocol = app.state::<Arc<BodyProtocol>>().inner().clone();
            let origin = app
                .get_webview_window(&label)
                .and_then(|window| window.url().ok())
                .and_then(|url| app_origin(&url))
                .unwrap_or_else(|| "'none'".into());
            tauri::async_runtime::spawn(async move {
                #[cfg(feature = "body-webview-test")]
                let trace = parse_route(&request.uri().to_string()).map(|(route, _)| match route {
                    Route::Document { id, .. } => ("document", id),
                    Route::Part { id, .. } => ("part", id),
                    Route::Link { id, .. } => ("link", id),
                    Route::Remote { id, .. } => ("remote", id),
                });
                let mut core = (label == "main")
                    .then(|| app.try_state::<crate::state::DesktopState>())
                    .flatten()
                    .and_then(|state| state.core().ok());
                if matches!(
                    parse_route(&request.uri().to_string()),
                    Some((Route::Document { .. }, _))
                ) && let Some(state) = app.try_state::<crate::state::DesktopState>()
                    && let Ok(snapshot) = state.settings(|store| store.get()).await
                {
                    protocol.set_base_css(mailx_core::mail_body_css(&snapshot.settings.appearance));
                }
                if matches!(
                    parse_route(&request.uri().to_string()),
                    Some((Route::Remote { .. }, _))
                ) {
                    let allowed = if let Some(state) = app.try_state::<crate::state::DesktopState>()
                    {
                        state
                            .settings(|store| store.get())
                            .await
                            .is_ok_and(|snapshot| {
                                protocol.permits_remote_request(
                                    &request.uri().to_string(),
                                    snapshot.settings.security.remote_images,
                                )
                            })
                    } else {
                        false
                    };
                    if !allowed {
                        core = None;
                    }
                }
                let (reply, event) = match core {
                    Some(core) => protocol.serve(&core, request, &origin).await,
                    None => (
                        response(
                            StatusCode::NOT_FOUND,
                            "text/plain; charset=utf-8",
                            vec![],
                            &origin,
                            None,
                        ),
                        None,
                    ),
                };
                #[cfg(feature = "body-webview-test")]
                crate::body_webview_test::record_response(
                    &app,
                    trace.as_ref().map_or("unknown", |(route, _)| *route),
                    trace.as_ref().map(|(_, id)| id.as_str()),
                    reply.status().as_u16(),
                );
                if let Some(event) = event {
                    let _ = app.emit_to(EventTarget::webview_window("main"), LINK_EVENT, event);
                }
                responder.respond(reply);
            });
        })
}

/// Cancel before WebKit commits a custom-scheme subframe navigation. WebView2
/// can also reach this hook; its subframe protocol fallback remains a 204.
pub(crate) fn navigation(app: &tauri::AppHandle, url: &Url) -> bool {
    use tauri::{Emitter, EventTarget, Manager};
    let Some((Route::Link { id, index, token }, _)) = parse_route(url.as_str()) else {
        return true;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let protocol = app.state::<std::sync::Arc<BodyProtocol>>();
        let Some(core) = app
            .try_state::<crate::state::DesktopState>()
            .and_then(|s| s.core().ok())
        else {
            return;
        };
        if let Ok(document) = protocol.resolve(&id, &token)
            && let Ok(event) = protocol.prepare_link(&core, document, id, index).await
        {
            let _ = app.emit_to(EventTarget::webview_window("main"), LINK_EVENT, event);
        }
    });
    false
}

#[derive(Clone)]
struct Document {
    token: String,
    binding: BodyBinding,
    expires_at: Instant,
    remote_images: bool,
    explicit_images: bool,
}

#[derive(Clone)]
struct Confirmation {
    document: Document,
    index: usize,
    url: String,
}

pub(crate) struct BodyProtocol {
    numbered_links: bool,
    base_css: Mutex<String>,
    documents: Mutex<HashMap<String, Document>>,
    confirmations: Mutex<HashMap<String, Confirmation>>,
    #[cfg(feature = "body-webview-test")]
    test_policy_probe: Mutex<Option<(String, String)>>,
}

impl Default for BodyProtocol {
    fn default() -> Self {
        // Native navigation cancellation keeps WKWebView/WebKitGTK frames intact.
        // WebView2 also retains its document for the protocol's 204 fallback.
        Self::new(false)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BodyLinks {
    pub document_token: String,
    pub links: Vec<ListedLink>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreparedBody {
    pub document_token: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListedLink {
    pub email_id: String,
    pub index: usize,
    pub url: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LinkRequest {
    pub email_id: String,
    pub index: usize,
    pub url: String,
    pub document_token: String,
    pub request_id: String,
}

/// Inverts the whole page, then inverts images back so photos keep their colors.
/// The canvas color is the inverse of the app's near-black background.
const DARK_CSS: &str = "html{background:#e8e8e8;filter:invert(1) hue-rotate(180deg)}img{filter:invert(1) hue-rotate(180deg)}";

#[derive(Debug, PartialEq, Eq)]
enum Route {
    Document {
        id: String,
        token: String,
        dark: bool,
    },
    Remote {
        id: String,
        index: usize,
        token: String,
    },
    Part {
        id: String,
        cid: String,
        token: String,
    },
    Link {
        id: String,
        index: usize,
        token: String,
    },
}

fn missing() -> AppError {
    AppError::new(ErrorCode::NotFound)
}

fn decode_segment(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    for (i, byte) in bytes.iter().enumerate() {
        if *byte == b'%'
            && (i + 2 >= bytes.len()
                || !bytes[i + 1].is_ascii_hexdigit()
                || !bytes[i + 2].is_ascii_hexdigit())
        {
            return None;
        }
    }
    let decoded = percent_encoding::percent_decode_str(value)
        .decode_utf8()
        .ok()?
        .into_owned();
    (!decoded.chars().any(char::is_control)).then_some(decoded)
}

fn parse_route(value: &str) -> Option<(Route, String)> {
    let url = Url::parse(value).ok()?;
    if !matches!(
        (url.scheme(), url.host_str(), url.port()),
        ("mailx-body", Some("localhost"), None) | ("http", Some("mailx-body.localhost"), None)
    ) {
        return None;
    }
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return None;
    }
    let segments = url
        .path()
        .strip_prefix('/')?
        .split('/')
        .map(decode_segment)
        .collect::<Option<Vec<_>>>()?;
    let id = segments.get(1)?;
    if !crate::window_access::valid_email_id(id) {
        return None;
    }
    let pairs: Vec<_> = url.query_pairs().collect();
    // Only a document may carry a presentation hint; it grants nothing.
    let dark = match pairs.get(1) {
        None => false,
        Some((key, value))
            if segments.first().map(String::as_str) == Some("document")
                && key == "scheme"
                && value == "dark" =>
        {
            true
        }
        Some(_) => return None,
    };
    if pairs.len() > 2 || pairs.first().is_none_or(|pair| pair.0 != "token") {
        return None;
    }
    let token = pairs[0].1.to_string();
    if Uuid::parse_str(&token).ok()?.to_string() != token {
        return None;
    }
    let route = match segments.first()?.as_str() {
        "remote" if segments.len() == 3 => {
            let index = segments[2].parse::<usize>().ok()?;
            if index.to_string() != segments[2] {
                return None;
            }
            Route::Remote {
                id: id.clone(),
                index,
                token,
            }
        }
        "document" if segments.len() == 2 => Route::Document {
            id: id.clone(),
            token,
            dark,
        },
        kind @ ("part" | "link") if segments.len() == 3 => {
            if kind == "part" {
                if segments[2].is_empty() {
                    return None;
                }
                Route::Part {
                    id: id.clone(),
                    cid: segments[2].clone(),
                    token,
                }
            } else {
                let index = segments[2].parse::<usize>().ok()?;
                if index.to_string() != segments[2] {
                    return None;
                }
                Route::Link {
                    id: id.clone(),
                    index,
                    token,
                }
            }
        }
        _ => return None,
    };
    Some((route, BODY_ORIGIN.into()))
}

fn resource_url(origin: &str, route: &str, id: &str, tail: &str, token: &str) -> String {
    let mut url = Url::parse(origin).expect("fixed body origin");
    url.path_segments_mut()
        .expect("body URL supports paths")
        .extend([route, id, tail]);
    url.query_pairs_mut().append_pair("token", token);
    url.to_string()
}

fn response(
    status: StatusCode,
    mime: &str,
    bytes: Vec<u8>,
    app_origin: &str,
    image_scope: Option<&str>,
) -> Response<Vec<u8>> {
    let images = image_scope.map_or_else(|| "data:".into(), |scope| format!("data: {scope}"));
    let csp = format!(
        "default-src 'none'; style-src 'unsafe-inline'; img-src {images}; font-src data:; form-action 'none'; base-uri 'none'; frame-ancestors {app_origin}"
    );
    let mut builder = Response::builder()
        .status(status)
        .header("Content-Type", mime)
        .header("Content-Security-Policy", csp)
        .header("X-Content-Type-Options", "nosniff")
        .header("Referrer-Policy", "no-referrer")
        .header("Cache-Control", "no-store");
    if status == StatusCode::METHOD_NOT_ALLOWED {
        builder = builder.header("Allow", "GET");
    }
    builder.body(bytes).expect("fixed response headers")
}

impl BodyProtocol {
    fn new(numbered_links: bool) -> Self {
        Self {
            numbered_links,
            base_css: Mutex::new(mailx_core::mail_body_css(&Default::default())),
            documents: Mutex::new(HashMap::new()),
            confirmations: Mutex::new(HashMap::new()),
            #[cfg(feature = "body-webview-test")]
            test_policy_probe: Mutex::new(None),
        }
    }
    #[cfg(feature = "body-webview-test")]
    pub(crate) fn configure_test_policy_probe(&self, id: String, canary_origin: String) {
        *self
            .test_policy_probe
            .lock()
            .expect("test policy probe lock") = Some((id, canary_origin));
    }
    fn issue(&self, binding: BodyBinding) -> Result<String, AppError> {
        let mut documents = self.documents.lock().map_err(|_| missing())?;
        documents.retain(|_, document| {
            if Instant::now() < document.expires_at {
                true
            } else {
                document.binding.release_remote_images();
                false
            }
        });
        if documents.len() >= MAX_DOCUMENTS
            && let Some(oldest) = documents
                .iter()
                .min_by_key(|(_, document)| document.expires_at)
                .map(|(token, _)| token.clone())
            && let Some(document) = documents.remove(&oldest)
        {
            document.binding.release_remote_images();
        }
        let token = Uuid::new_v4().to_string();
        documents.insert(
            token.clone(),
            Document {
                token: token.clone(),
                binding,
                expires_at: Instant::now() + TOKEN_LIFETIME,
                remote_images: false,
                explicit_images: false,
            },
        );
        Ok(token)
    }

    /// Each preparation creates an independent capability. There is deliberately
    /// no email-to-current-token pointer: a late prepare/GET for A cannot replace
    /// the token selected by a newer A -> B -> A view. The trusted UI accepts
    /// results/events only for its current view generation and exact token.
    /// The UI releases unselected tokens; TTL/eviction is a fallback.
    pub(crate) async fn prepare(
        &self,
        core: &MailxCore,
        id: String,
    ) -> Result<PreparedBody, AppError> {
        let links = core.body_links(id, None).await?;
        Ok(PreparedBody {
            document_token: self.issue(links.binding)?,
        })
    }

    pub(crate) async fn prepare_with_images(
        &self,
        core: &MailxCore,
        id: String,
        allow: bool,
        explicit: bool,
    ) -> Result<PreparedBody, AppError> {
        let prepared = self.prepare(core, id).await?;
        if allow {
            let mut documents = self.documents.lock().map_err(|_| missing())?;
            let document = documents
                .get_mut(&prepared.document_token)
                .ok_or_else(missing)?;
            document.remote_images = true;
            document.explicit_images = explicit;
        }
        Ok(prepared)
    }

    fn set_base_css(&self, css: String) {
        if let Ok(mut current) = self.base_css.lock() {
            *current = css;
        }
    }

    fn permits_remote_request(&self, value: &str, policy: mailx_core::RemoteImagePolicy) -> bool {
        let Some((Route::Remote { id, token, .. }, _)) = parse_route(value) else {
            return false;
        };
        self.resolve(&id, &token).is_ok_and(|document| {
            document.remote_images
                && match policy {
                    mailx_core::RemoteImagePolicy::Never => false,
                    mailx_core::RemoteImagePolicy::Ask => document.explicit_images,
                    mailx_core::RemoteImagePolicy::Automatic => true,
                }
        })
    }

    pub(crate) fn revoke_remote_images(&self) {
        if let Ok(mut documents) = self.documents.lock() {
            for document in documents.values_mut() {
                document.remote_images = false;
                document.binding.release_remote_images();
            }
        }
    }

    /// A saved connection replaces the route and invalidates its open readers.
    pub(crate) fn revoke_account(&self, account_id: &str) {
        if let Ok(mut documents) = self.documents.lock() {
            documents.retain(|_, document| {
                if document.binding.account_id() != account_id {
                    return true;
                }
                document.binding.release_remote_images();
                false
            });
        }
        if let Ok(mut confirmations) = self.confirmations.lock() {
            confirmations
                .retain(|_, confirmation| confirmation.document.binding.account_id() != account_id);
        }
    }

    pub(crate) async fn image_status(
        &self,
        core: &MailxCore,
        id: &str,
        token: &str,
        policy: mailx_core::RemoteImagePolicy,
    ) -> Result<Vec<mailx_core::RemoteImageStatus>, AppError> {
        let allowed = || {
            let document = self.resolve(id, token)?;
            if !document.remote_images
                || matches!(policy, mailx_core::RemoteImagePolicy::Never)
                || (matches!(policy, mailx_core::RemoteImagePolicy::Ask)
                    && !document.explicit_images)
            {
                return Err(missing());
            }
            Ok(document)
        };
        let document = allowed()?;
        let statuses = core.body_image_status(document.binding).await?;
        allowed()?;
        Ok(statuses)
    }

    pub(crate) async fn retry_images(
        &self,
        core: &MailxCore,
        id: &str,
        token: &str,
        policy: mailx_core::RemoteImagePolicy,
    ) -> Result<PreparedBody, AppError> {
        self.image_status(core, id, token, policy).await?;
        let previous = self.resolve(id, token)?;
        let binding = core.body_retry_images(previous.binding).await?;
        let next = Uuid::new_v4().to_string();
        {
            let mut documents = self.documents.lock().map_err(|_| missing())?;
            let current = documents
                .get(token)
                .filter(|document| {
                    document.binding.email_id() == id
                        && document.remote_images
                        && Instant::now() < document.expires_at
                })
                .cloned()
                .ok_or_else(missing)?;
            // Rotate under the same lock used by revocation: a stale retry
            // cannot recreate a grant after the user tightened image policy.
            documents.insert(
                next.clone(),
                Document {
                    token: next.clone(),
                    binding,
                    expires_at: Instant::now() + TOKEN_LIFETIME,
                    remote_images: true,
                    explicit_images: current.explicit_images,
                },
            );
            documents.remove(token);
            current.binding.release_remote_images();
        }
        // Removes old pending link confirmations; the old binding is gone.
        self.release(id, token)?;
        Ok(PreparedBody {
            document_token: next,
        })
    }

    /// End one reading without affecting a newer opening of the same email.
    pub(crate) fn release(&self, id: &str, token: &str) -> Result<(), AppError> {
        {
            let mut documents = self.documents.lock().map_err(|_| missing())?;
            if let Some(document) = documents.get(token) {
                if document.binding.email_id() != id {
                    return Err(missing());
                }
                document.binding.release_remote_images();
                documents.remove(token);
            }
        }
        self.confirmations
            .lock()
            .map_err(|_| missing())?
            .retain(|_, confirmation| confirmation.document.token != token);
        Ok(())
    }

    fn resolve(&self, id: &str, token: &str) -> Result<Document, AppError> {
        self.documents
            .lock()
            .map_err(|_| missing())?
            .get(token)
            .filter(|document| {
                document.binding.email_id() == id && Instant::now() < document.expires_at
            })
            .cloned()
            .ok_or_else(missing)
    }

    pub(crate) async fn serve(
        &self,
        core: &MailxCore,
        request: Request<Vec<u8>>,
        app_origin: &str,
    ) -> (Response<Vec<u8>>, Option<LinkRequest>) {
        if request.method() != Method::GET {
            return (
                response(
                    StatusCode::METHOD_NOT_ALLOWED,
                    "text/plain; charset=utf-8",
                    vec![],
                    app_origin,
                    None,
                ),
                None,
            );
        }
        let Some((route, origin)) = parse_route(&request.uri().to_string()) else {
            return (
                response(
                    StatusCode::NOT_FOUND,
                    "text/plain; charset=utf-8",
                    vec![],
                    app_origin,
                    None,
                ),
                None,
            );
        };
        match self.resource(core, route, &origin, app_origin).await {
            Ok(result) => result,
            Err(_) => (
                response(
                    StatusCode::NOT_FOUND,
                    "text/plain; charset=utf-8",
                    vec![],
                    app_origin,
                    None,
                ),
                None,
            ),
        }
    }

    async fn resource(
        &self,
        core: &MailxCore,
        route: Route,
        origin: &str,
        app_origin: &str,
    ) -> Result<(Response<Vec<u8>>, Option<LinkRequest>), AppError> {
        match route {
            Route::Document { id, token, dark } => {
                let document = self.resolve(&id, &token)?;
                let allow_remote = document.remote_images;
                let document = core.body_document_bound(document.binding).await?;
                self.resolve(&id, &token)?;
                let resolve = |binding: ResourceBinding<'_>| {
                    Some(match binding {
                        ResourceBinding::Remote { index } => {
                            if !allow_remote {
                                return None;
                            }
                            resource_url(origin, "remote", &id, &index.to_string(), &token)
                        }
                        ResourceBinding::Part { content_id } => {
                            resource_url(origin, "part", &id, content_id, &token)
                        }
                        ResourceBinding::Link { index } => {
                            resource_url(origin, "link", &id, &index.to_string(), &token)
                        }
                    })
                };
                let html = if self.numbered_links {
                    render::bind_numbered_resources(&document.html, resolve)
                } else {
                    render::bind_resources(&document.html, resolve)
                };
                let html = format!(
                    "<!doctype html><html><head><meta charset=\"utf-8\"><style>{base_css}{dark_css}</style></head><body><div class=\"mailx-page\">{html}</div></body></html>",
                    base_css = self.base_css.lock().map_err(|_| missing())?,
                    dark_css = if dark { DARK_CSS } else { "" }
                );
                #[cfg(feature = "body-webview-test")]
                let html = {
                    let probe = self.test_policy_probe.lock().map_err(|_| missing())?;
                    if let Some((probe_id, canary)) =
                        probe.as_ref().filter(|(probe_id, _)| probe_id == &id)
                    {
                        crate::body_webview_test::policy_probe_html(
                            canary,
                            &resource_url(origin, "part", probe_id, "pixel", &token),
                            &resource_url(origin, "link", probe_id, "0", &token),
                        )
                        .map_err(|_| missing())?
                    } else {
                        html
                    }
                };
                let scope = format!("{origin}/part/{id}/ {origin}/remote/{id}/");
                Ok((
                    response(
                        StatusCode::OK,
                        "text/html; charset=utf-8",
                        html.into_bytes(),
                        app_origin,
                        Some(&scope),
                    ),
                    None,
                ))
            }
            Route::Remote { id, index, token } => {
                let document = self.resolve(&id, &token)?;
                if !document.remote_images {
                    return Err(missing());
                }
                let image = core.body_remote_image(document.binding, index).await?;
                if !self.resolve(&id, &token)?.remote_images {
                    return Err(missing());
                }
                Ok((
                    response(
                        StatusCode::OK,
                        &image.mime_type,
                        image.bytes,
                        app_origin,
                        None,
                    ),
                    None,
                ))
            }
            Route::Part { id, cid, token } => {
                let document = self.resolve(&id, &token)?;
                let image = core.body_part(document.binding, cid).await?;
                Ok((
                    response(
                        StatusCode::OK,
                        &image.mime_type,
                        image.bytes,
                        app_origin,
                        None,
                    ),
                    None,
                ))
            }
            Route::Link { id, index, token } => {
                if self.numbered_links {
                    return Err(missing());
                }
                let document = self.resolve(&id, &token)?;
                let event = self.prepare_link(core, document, id, index).await?;
                Ok((
                    response(
                        StatusCode::NO_CONTENT,
                        "text/plain; charset=utf-8",
                        vec![],
                        app_origin,
                        None,
                    ),
                    Some(event),
                ))
            }
        }
    }

    pub(crate) async fn list_links(
        &self,
        core: &MailxCore,
        id: String,
        token: String,
    ) -> Result<BodyLinks, AppError> {
        let document = self.resolve(&id, &token)?;
        let links = core.body_links(id.clone(), Some(document.binding)).await?;
        self.resolve(&id, &token)?;
        Ok(BodyLinks {
            document_token: token,
            links: if self.numbered_links {
                links
                    .links
                    .into_iter()
                    .enumerate()
                    .map(|(index, url)| ListedLink {
                        email_id: id.clone(),
                        index,
                        url,
                    })
                    .collect()
            } else {
                vec![]
            },
        })
    }

    pub(crate) async fn request_link(
        &self,
        core: &MailxCore,
        id: String,
        index: usize,
        token: String,
    ) -> Result<LinkRequest, AppError> {
        if !self.numbered_links {
            return Err(missing());
        }
        let document = self.resolve(&id, &token)?;
        self.prepare_link(core, document, id, index).await
    }

    async fn prepare_link(
        &self,
        core: &MailxCore,
        document: Document,
        id: String,
        index: usize,
    ) -> Result<LinkRequest, AppError> {
        let url = core
            .body_link(id.clone(), index, Some(document.binding.clone()))
            .await?;
        self.resolve(&id, &document.token)?;
        let request_id = Uuid::new_v4().to_string();
        let document_token = document.token.clone();
        let mut confirmations = self.confirmations.lock().map_err(|_| missing())?;
        confirmations.retain(|_, confirmation| Instant::now() < confirmation.document.expires_at);
        if confirmations.len() >= MAX_DOCUMENTS {
            confirmations.clear();
        }
        confirmations.insert(
            request_id.clone(),
            Confirmation {
                document,
                index,
                url: url.clone(),
            },
        );
        Ok(LinkRequest {
            email_id: id,
            index,
            url,
            document_token,
            request_id,
        })
    }

    /// Called only after a trusted main-window confirmation. Consuming the
    /// pending request prevents replay, and its document identity prevents an
    /// account edit or changed link mapping from opening a different target.
    pub(crate) async fn confirm_link(
        &self,
        core: &MailxCore,
        id: String,
        index: usize,
        token: String,
        request_id: String,
    ) -> Result<String, AppError> {
        let confirmation = {
            let mut confirmations = self.confirmations.lock().map_err(|_| missing())?;
            // A mismatched request must not consume a different dialog's grant.
            let pending = confirmations.get(&request_id).ok_or_else(missing)?;
            if pending.document.binding.email_id() != id
                || pending.index != index
                || pending.document.token != token
            {
                return Err(missing());
            }
            confirmations.remove(&request_id).ok_or_else(missing)?
        };
        self.resolve(&id, &token)?;
        let url = core
            .body_link(id.clone(), index, Some(confirmation.document.binding))
            .await?;
        self.resolve(&id, &token)?;
        if url != confirmation.url {
            return Err(missing());
        }
        Ok(url)
    }
}

/// Only the real app origin is allowed as frame ancestor; URL text from an
/// email, protocol request or IPC argument never contributes to this header.
pub(crate) fn app_origin(url: &Url) -> Option<String> {
    if !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    match (url.scheme(), url.host_str(), url.port()) {
        ("tauri", Some("localhost"), None) => Some("tauri://localhost".into()),
        ("http", Some("tauri.localhost"), None) => Some("http://tauri.localhost".into()),
        ("http", Some("localhost"), Some(5173)) if cfg!(debug_assertions) => {
            Some("http://localhost:5173".into())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests;
