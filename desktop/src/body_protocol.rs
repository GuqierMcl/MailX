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
                    Route::Document { id } => ("document", id),
                    Route::Part { id, .. } => ("part", id),
                    Route::Link { id, .. } => ("link", id),
                });
                let core = (label == "main")
                    .then(|| app.try_state::<crate::state::DesktopState>())
                    .flatten()
                    .and_then(|state| state.core().ok());
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

#[derive(Clone)]
struct Document {
    token: String,
    binding: BodyBinding,
    created: Instant,
}

#[derive(Clone)]
struct Confirmation {
    document: Document,
    url: String,
}

pub(crate) struct BodyProtocol {
    numbered_links: bool,
    documents: Mutex<HashMap<String, Document>>,
    numbered_documents: Mutex<HashMap<String, String>>,
    confirmations: Mutex<HashMap<(String, usize), Confirmation>>,
    #[cfg(feature = "body-webview-test")]
    test_policy_probe: Mutex<Option<(String, String)>>,
}

impl Default for BodyProtocol {
    fn default() -> Self {
        // WKWebView replaces the frame with a blank document after a custom
        // protocol 204. Use the specification's trusted numbered-link fallback.
        Self::new(cfg!(target_os = "macos"))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BodyLinks {
    pub document_token: Option<String>,
    pub links: Vec<LinkRequest>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LinkRequest {
    pub email_id: String,
    pub index: usize,
    pub url: String,
}

#[derive(Debug, PartialEq, Eq)]
enum Route {
    Document {
        id: String,
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
    let origin = match (url.scheme(), url.host_str(), url.port()) {
        ("mailx-body", Some("localhost"), None) => "mailx-body://localhost",
        ("http", Some("mailx-body.localhost"), None) => "http://mailx-body.localhost",
        _ => return None,
    };
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
    if Uuid::parse_str(id).ok()?.to_string() != *id {
        return None;
    }
    let route = match segments.first()?.as_str() {
        "document" if segments.len() == 2 && url.query().is_none() => {
            Route::Document { id: id.clone() }
        }
        kind @ ("part" | "link") if segments.len() == 3 => {
            let pairs: Vec<_> = url.query_pairs().collect();
            if pairs.len() != 1 || pairs[0].0 != "token" {
                return None;
            }
            let token = pairs[0].1.to_string();
            if Uuid::parse_str(&token).ok()?.to_string() != token {
                return None;
            }
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
    Some((route, origin.into()))
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
            documents: Mutex::new(HashMap::new()),
            numbered_documents: Mutex::new(HashMap::new()),
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
        documents.retain(|_, document| document.created.elapsed() < TOKEN_LIFETIME);
        if documents.len() >= MAX_DOCUMENTS
            && let Some(oldest) = documents
                .iter()
                .min_by_key(|(_, document)| document.created)
                .map(|(token, _)| token.clone())
        {
            documents.remove(&oldest);
        }
        let token = Uuid::new_v4().to_string();
        if self.numbered_links {
            let mut numbered = self.numbered_documents.lock().map_err(|_| missing())?;
            numbered.retain(|_, token| documents.contains_key(token));
            numbered.insert(binding.email_id().to_owned(), token.clone());
        }
        documents.insert(
            token.clone(),
            Document {
                token: token.clone(),
                binding,
                created: Instant::now(),
            },
        );
        Ok(token)
    }

    fn resolve(&self, id: &str, token: &str) -> Result<Document, AppError> {
        self.documents
            .lock()
            .map_err(|_| missing())?
            .get(token)
            .filter(|document| {
                document.binding.email_id() == id && document.created.elapsed() < TOKEN_LIFETIME
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
            Route::Document { id } => {
                let document = core.body_document(id.clone()).await?;
                let token = self.issue(document.binding)?;
                let resolve = |binding: ResourceBinding<'_>| {
                    Some(match binding {
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
                    "<!doctype html><html><head><meta charset=\"utf-8\"><title>邮件正文</title><style>html{{background:white;color:black}}body{{overflow-wrap:anywhere}}</style></head><body>{html}</body></html>"
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
                let scope = format!("{origin}/part/{id}/");
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
    ) -> Result<BodyLinks, AppError> {
        if !self.numbered_links {
            core.body_links(id, None).await?;
            return Ok(BodyLinks {
                document_token: None,
                links: vec![],
            });
        }
        let token = self
            .numbered_documents
            .lock()
            .map_err(|_| missing())?
            .get(&id)
            .cloned()
            .ok_or_else(missing)?;
        let document = self.resolve(&id, &token)?;
        let links = core.body_links(id.clone(), Some(document.binding)).await?;
        self.check_numbered_document(&id, &token)?;
        Ok(BodyLinks {
            document_token: Some(token),
            links: links
                .links
                .into_iter()
                .enumerate()
                .map(|(index, url)| LinkRequest {
                    email_id: id.clone(),
                    index,
                    url,
                })
                .collect(),
        })
    }

    fn check_numbered_document(&self, id: &str, token: &str) -> Result<(), AppError> {
        if !self.numbered_links
            || self
                .numbered_documents
                .lock()
                .map_err(|_| missing())?
                .get(id)
                .is_none_or(|current| current != token)
        {
            return Err(missing());
        }
        Ok(())
    }

    pub(crate) async fn request_link(
        &self,
        core: &MailxCore,
        id: String,
        index: usize,
        token: String,
    ) -> Result<LinkRequest, AppError> {
        self.check_numbered_document(&id, &token)?;
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
        if self.numbered_links {
            self.check_numbered_document(&id, &document.token)?;
        }
        let mut confirmations = self.confirmations.lock().map_err(|_| missing())?;
        confirmations
            .retain(|_, confirmation| confirmation.document.created.elapsed() < TOKEN_LIFETIME);
        if confirmations.len() >= MAX_DOCUMENTS {
            confirmations.clear();
        }
        confirmations.insert(
            (id.clone(), index),
            Confirmation {
                document,
                url: url.clone(),
            },
        );
        Ok(LinkRequest {
            email_id: id,
            index,
            url,
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
    ) -> Result<String, AppError> {
        let confirmation = self
            .confirmations
            .lock()
            .map_err(|_| missing())?
            .remove(&(id.clone(), index))
            .ok_or_else(missing)?;
        if confirmation.document.created.elapsed() >= TOKEN_LIFETIME {
            return Err(missing());
        }
        if self.numbered_links {
            self.check_numbered_document(&id, &confirmation.document.token)?;
        }
        let url = core
            .body_link(id.clone(), index, Some(confirmation.document.binding))
            .await?;
        if self.numbered_links {
            self.check_numbered_document(&id, &confirmation.document.token)?;
        }
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
