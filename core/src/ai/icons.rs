//! Provider icons from models.dev. The SVG is only checked and handed to the
//! frontend as data; it is never rasterized here and must never be inserted
//! into the DOM as markup (the frontend uses it as a CSS mask).

use std::{fs, io::Write, path::Path, time::Duration};

use base64::Engine;
use hyper::Method;
use serde::Serialize;
use ts_rs::TS;
use url::Url;
use uuid::Uuid;

use super::{client::http_error, validation::valid_provider_id};
use crate::{
    ErrorCode,
    net::{
        NetRoute,
        http::{self, HttpRequest},
    },
};

pub(crate) const ICON_BASE_URL: &str = "https://models.dev/logos";
pub(crate) const ICON_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const MAX_ICON_BYTES: usize = 64 * 1024;
/// A cached icon is used for this long.
pub(crate) const CACHE_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
const MAX_NODES: u32 = 20_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderIcon {
    #[ts(type = "\"image/svg+xml\"")]
    pub mime_type: String,
    /// Base64 of the SVG text.
    pub data: String,
}

impl AiProviderIcon {
    pub(crate) fn from_svg(svg: &[u8]) -> Self {
        Self {
            mime_type: "image/svg+xml".into(),
            data: base64::engine::general_purpose::STANDARD.encode(svg),
        }
    }
}

/// Whether `bytes` is a plain SVG document: at most 64 KB, UTF-8, an `svg`
/// root in the SVG namespace, and no `script` or `foreignObject` anywhere.
pub(crate) fn valid_svg(bytes: &[u8]) -> bool {
    if bytes.is_empty() || bytes.len() > MAX_ICON_BYTES {
        return false;
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let Ok(document) = resvg::usvg::roxmltree::Document::parse_with_options(
        text,
        resvg::usvg::roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: MAX_NODES,
            ..Default::default()
        },
    ) else {
        return false;
    };
    let root = document.root_element();
    root.tag_name().name() == "svg"
        && root.tag_name().namespace() == Some(SVG_NAMESPACE)
        && !document.descendants().any(|node| {
            node.is_element()
                && matches!(
                    node.tag_name().name().to_ascii_lowercase().as_str(),
                    "script" | "foreignobject"
                )
        })
}

/// The icon file for a valid provider id; the id is the only part of the name
/// that varies and it cannot contain a path separator or a dot.
fn cache_path(directory: &Path, provider_id: &str) -> Option<std::path::PathBuf> {
    valid_provider_id(provider_id).then(|| directory.join(format!("{provider_id}.svg")))
}

/// A cached icon younger than 7 days that still passes validation.
pub(crate) fn read_cached(directory: &Path, provider_id: &str) -> Option<Vec<u8>> {
    let path = cache_path(directory, provider_id)?;
    let metadata = fs::symlink_metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_ICON_BYTES as u64 {
        return None;
    }
    let age = metadata.modified().ok()?.elapsed().unwrap_or_default();
    if age > CACHE_MAX_AGE {
        return None;
    }
    let bytes = fs::read(&path).ok()?;
    valid_svg(&bytes).then_some(bytes)
}

pub(crate) fn write_cached(directory: &Path, provider_id: &str, svg: &[u8]) {
    let Some(path) = cache_path(directory, provider_id) else {
        return;
    };
    let temporary = directory.join(format!(".{provider_id}.{}.tmp", Uuid::new_v4()));
    let result = (|| {
        fs::create_dir_all(directory)?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(svg)?;
        drop(file);
        fs::rename(&temporary, &path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
}

/// Fetches and validates one icon. Anything wrong with it (status, size, type,
/// content) is `AiInvalidResponse` or the transport's code.
pub(crate) async fn fetch(
    route: &NetRoute,
    base: &str,
    provider_id: &str,
) -> Result<Vec<u8>, ErrorCode> {
    if !valid_provider_id(provider_id) {
        return Err(ErrorCode::Validation);
    }
    let url = Url::parse(&format!("{}/{provider_id}.svg", base.trim_end_matches('/')))
        .map_err(|_| ErrorCode::AiNetwork)?;
    let reply = http::send(HttpRequest {
        method: Method::GET,
        url: &url,
        route,
        headers: Vec::new(),
        body: None,
        timeout: ICON_TIMEOUT,
        max_response_bytes: MAX_ICON_BYTES,
    })
    .await
    .map_err(http_error)?;
    if reply.status != 200 {
        return Err(super::client::status_error(reply.status, false));
    }
    if valid_svg(&reply.body) {
        Ok(reply.body)
    } else {
        Err(ErrorCode::AiInvalidResponse)
    }
}

#[cfg(test)]
pub(crate) mod tests;
