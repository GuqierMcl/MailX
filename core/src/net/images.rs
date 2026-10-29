//! Bounded, credential-free HTTP for untrusted mail images. This uses the
//! account route, pins checked public addresses and revalidates every redirect.
mod transport;

use super::NetRoute;
use crate::{
    AppError, ErrorCode,
    mail::render::{InlineImage, RenderError, decode_remote_raster, remote_raster_format},
};
use bytes::Bytes;
use http_body_util::{BodyExt, Empty};
use hyper::{Request, header};
use hyper_util::rt::TokioIo;
use std::{
    net::IpAddr,
    sync::{Arc, LazyLock},
    time::Duration,
};
use transport::{connect_target, network_error, target};
use url::Url;

pub(crate) const MAX_BYTES: usize = 8 * 1024 * 1024;
static DOWNLOADS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
// The permit is moved into the blocking task. Cancelling its async waiter does
// not free a slot while a decoder is still consuming CPU and memory.
static DECODERS: LazyLock<Arc<tokio::sync::Semaphore>> =
    LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(4)));

/// Fixed diagnostics for the trusted host; never contains URLs or peer text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub enum ImageFetchError {
    TargetBlocked,
    ProxyPolicyRequired,
    Dns,
    Connect,
    Proxy,
    Tls,
    HttpStatus,
    /// The server answered 404 or 410: the image does not exist. Unlike other
    /// statuses (403/429 throttling, 5xx outages) this is a stable answer.
    NotFound,
    UnsupportedType,
    InvalidImage,
    TooLarge,
    Timeout,
    Cancelled,
}

impl From<ImageFetchError> for AppError {
    fn from(_: ImageFetchError) -> Self {
        Self::new(ErrorCode::NotFound)
    }
}

pub(crate) fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(a == 0
                || a == 10
                || a == 127
                || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192
                    && (b == 168 || (b == 0 && (c == 0 || c == 2)) || (b == 88 && c == 99)))
                || (a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100)))
                || (a == 203 && b == 0 && c == 113))
        }
        IpAddr::V6(ip) => {
            let s = ip.segments();
            // Only ordinary global unicast; reject mapped IPv4, transition,
            // documentation, local, multicast and special-purpose prefixes.
            s[0] & 0xe000 == 0x2000
                && s[0] != 0x2002
                && !(s[0] == 0x2001 && (s[1] < 0x200 || s[1] == 0xdb8))
                && !(s[0] == 0x3fff && s[1] < 0x1000)
        }
    }
}

fn checked_url(value: &str) -> Result<Url, ImageFetchError> {
    let mut url = Url::parse(value).map_err(|_| ImageFetchError::TargetBlocked)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || value.len() > 8192
    {
        return Err(ImageFetchError::TargetBlocked);
    }
    url.set_fragment(None);
    Ok(url)
}

struct ConnectionTask(tokio::task::JoinHandle<()>);
impl Drop for ConnectionTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// How long the HTTPS attempt for a plain-HTTP image may take before the
/// original address gets its turn.
const UPGRADE_ATTEMPT: Duration = Duration::from_secs(10);

/// Many campaigns still reference `http://` images from CDNs that serve the
/// same file over HTTPS. Only plain default-port URLs on named hosts qualify:
/// an explicit port means something else, and an IP literal has no certificate.
fn https_upgrade(url: &Url) -> Option<Url> {
    if url.scheme() != "http"
        || url.port().is_some()
        || !matches!(url.host(), Some(url::Host::Domain(_)))
    {
        return None;
    }
    let mut secure = url.clone();
    secure.set_scheme("https").ok()?;
    Some(secure)
}

pub(crate) async fn fetch(route: &NetRoute, value: String) -> Result<InlineImage, ImageFetchError> {
    tokio::time::timeout(Duration::from_secs(30), async {
        let _permit = DOWNLOADS
            .acquire()
            .await
            .map_err(|_| ImageFetchError::Cancelled)?;
        let url = checked_url(&value)?;
        // Tried first because plain HTTP cannot be used behind a Fake-IP (TUN)
        // proxy, where the destination can only be authenticated through TLS,
        // and because it keeps the image off the network in clear text.
        if let Some(secure) = https_upgrade(&url) {
            let upgraded = tokio::time::timeout(UPGRADE_ATTEMPT, follow(route, secure))
                .await
                .unwrap_or(Err(ImageFetchError::Timeout));
            match upgraded {
                Ok(image) => return Ok(image),
                // A proxy-domain request may only use HTTPS. Preserve the
                // useful HTTPS diagnostic instead of reporting a blocked HTTP
                // fallback that we were never going to attempt.
                Err(error) if route.proxy.is_some() => return Err(error),
                Err(_) => {}
            }
        }
        follow(route, url).await
    })
    .await
    .map_err(|_| ImageFetchError::Timeout)?
}

async fn follow(route: &NetRoute, mut url: Url) -> Result<InlineImage, ImageFetchError> {
    for redirects in 0..=3 {
        let checked = target(route, &url).await?;
        let mut stream = connect_target(route, &url, checked).await?;
        if url.scheme() == "https" {
            stream = super::upgrade_tls(stream, url.host_str().unwrap().trim_matches(['[', ']']))
                .await
                .map_err(network_error)?;
        }
        match exchange(stream, &url).await? {
            ImageReply::Image(image) => return Ok(image),
            ImageReply::Redirect(next) if redirects < 3 => url = next,
            _ => return Err(ImageFetchError::TargetBlocked),
        }
    }
    Err(ImageFetchError::TargetBlocked)
}

fn status_error(status: u16) -> ImageFetchError {
    match status {
        404 | 410 => ImageFetchError::NotFound,
        _ => ImageFetchError::HttpStatus,
    }
}

enum ImageReply {
    Redirect(Url),
    Image(InlineImage),
}

async fn exchange(stream: super::BoxedStream, url: &Url) -> Result<ImageReply, ImageFetchError> {
    let (mut sender, connection) = hyper::client::conn::http1::Builder::new()
        .max_headers(64)
        .max_buf_size(32 * 1024)
        .handshake(TokioIo::new(stream))
        .await
        .map_err(|_| ImageFetchError::Connect)?;
    let _connection = ConnectionTask(tokio::spawn(async move {
        let _ = connection.await;
    }));
    let authority = &url[url::Position::BeforeHost..url::Position::AfterPort];
    let path = &url[url::Position::BeforePath..url::Position::AfterQuery];
    let request = Request::builder()
        .method("GET")
        .uri(path)
        .header(header::HOST, authority)
        .header(
            header::ACCEPT,
            "image/png,image/jpeg,image/gif,image/webp,image/svg+xml,image/bmp,image/tiff,image/x-icon,image/*;q=0.9,*/*;q=0.1",
        )
        .header(header::ACCEPT_ENCODING, "identity")
        .header(header::CONNECTION, "close")
        .body(Empty::<Bytes>::new())
        .map_err(|_| ImageFetchError::TargetBlocked)?;
    let response = sender
        .send_request(request)
        .await
        .map_err(|_| ImageFetchError::Connect)?;

    if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
        let location = response
            .headers()
            .get(header::LOCATION)
            .and_then(|h| h.to_str().ok())
            .ok_or(ImageFetchError::TargetBlocked)?;
        let next = checked_url(
            url.join(location)
                .map_err(|_| ImageFetchError::TargetBlocked)?
                .as_str(),
        )?;
        if url.scheme() == "https" && next.scheme() != "https" {
            return Err(ImageFetchError::TargetBlocked);
        }
        return Ok(ImageReply::Redirect(next));
    }
    if response.status() != 200 {
        return Err(status_error(response.status().as_u16()));
    }
    if response
        .headers()
        .get(header::CONTENT_ENCODING)
        .is_some_and(|v| v != "identity")
    {
        return Err(ImageFetchError::UnsupportedType);
    }
    if response
        .headers()
        .get(header::CONTENT_LENGTH)
        .is_some_and(|v| {
            v.to_str()
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .is_none_or(|n| n > MAX_BYTES)
        })
    {
        return Err(ImageFetchError::TooLarge);
    }
    let mime = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.split(';').next())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let mut body = response.into_body();
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| ImageFetchError::Connect)?;
        if let Ok(data) = frame.into_data() {
            if bytes.len().saturating_add(data.len()) > MAX_BYTES {
                return Err(ImageFetchError::TooLarge);
            }
            bytes.extend_from_slice(&data);
        }
    }

    let image = decode_on_worker(DECODERS.clone(), move || decode_image(&bytes, &mime)).await?;
    Ok(ImageReply::Image(image))
}

async fn decode_on_worker(
    permits: Arc<tokio::sync::Semaphore>,
    decode: impl FnOnce() -> Result<InlineImage, ImageFetchError> + Send + 'static,
) -> Result<InlineImage, ImageFetchError> {
    let permit = permits
        .acquire_owned()
        .await
        .map_err(|_| ImageFetchError::Cancelled)?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        decode()
    })
    .await
    .map_err(|_| ImageFetchError::Cancelled)?
}

fn decode_image(bytes: &[u8], _mime: &str) -> Result<InlineImage, ImageFetchError> {
    // Bytes, not a CDN's Content-Type, determine the decoder. Even a label such
    // as text/html cannot turn validated raster data into active HTML: output
    // comes exclusively from our bounded image path with its own MIME.
    if remote_raster_format(bytes).is_some() {
        return decode_remote_raster(bytes).map_err(|error| match error {
            RenderError::TooLarge => ImageFetchError::TooLarge,
            _ => ImageFetchError::InvalidImage,
        });
    }
    // Remote SVGs never reach the WebView. Rasterize static vector shapes only,
    // with no scripts, external/local images, fonts, nested SVGs or SVGZ decoding.
    use resvg::{tiny_skia, usvg};
    let text = std::str::from_utf8(bytes).map_err(|_| ImageFetchError::UnsupportedType)?;
    if !text
        .trim_start_matches(['\u{feff}', ' ', '\r', '\n', '\t'])
        .starts_with('<')
    {
        return Err(ImageFetchError::UnsupportedType);
    }
    if bytes.len() > 1024 * 1024 {
        return Err(ImageFetchError::TooLarge);
    }
    let xml = usvg::roxmltree::Document::parse_with_options(
        text,
        usvg::roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 20_000,
            ..Default::default()
        },
    )
    .map_err(|_| ImageFetchError::InvalidImage)?;
    let root = xml.root_element();
    if root.tag_name().name() != "svg"
        || root.tag_name().namespace() != Some("http://www.w3.org/2000/svg")
    {
        return Err(ImageFetchError::UnsupportedType);
    }
    if xml
        .descendants()
        .any(|node| node.ancestors().count() > 32 || node.has_tag_name("filter"))
    {
        return Err(ImageFetchError::InvalidImage);
    }
    let options = usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_string: Box::new(|_, _| None),
            resolve_data: Box::new(|_, _, _| None),
        },
        ..Default::default()
    };
    let tree =
        usvg::Tree::from_xmltree(&xml, &options).map_err(|_| ImageFetchError::InvalidImage)?;
    let size = tree.size().to_int_size();
    if size.width() > 4096
        || size.height() > 4096
        || u64::from(size.width()) * u64::from(size.height()) > 4 * 1024 * 1024
        || !tree.root().has_children()
    {
        return Err(ImageFetchError::TooLarge);
    }
    let mut pixmap =
        tiny_skia::Pixmap::new(size.width(), size.height()).ok_or(ImageFetchError::TooLarge)?;
    resvg::render(
        &tree,
        tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    let bytes = pixmap
        .encode_png()
        .map_err(|_| ImageFetchError::InvalidImage)?;
    if bytes.len() > crate::mail::render::MAX_REMOTE_IMAGE_OUTPUT_BYTES {
        return Err(ImageFetchError::TooLarge);
    }
    Ok(InlineImage {
        bytes,
        mime_type: "image/png".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use transport::resolved_targets;

    #[test]
    fn only_404_and_410_are_confirmed_missing() {
        for status in [404, 410] {
            assert_eq!(status_error(status), ImageFetchError::NotFound);
        }
        for status in [400, 401, 403, 408, 429, 500, 502, 503, 504] {
            assert_eq!(status_error(status), ImageFetchError::HttpStatus);
        }
    }

    #[test]
    fn denies_local_metadata_reserved_and_transition_addresses() {
        for value in [
            "0.0.0.0",
            "10.0.0.1",
            "127.0.0.1",
            "100.100.100.200",
            "169.254.169.254",
            "172.20.0.1",
            "192.168.0.1",
            "198.18.0.1",
            "224.0.0.1",
            "::1",
            "::ffff:8.8.8.8",
            "fe80::1",
            "fc00::1",
            "2002:0808:0808::",
            "2001:2::1",
            "2001:db8::1",
        ] {
            assert!(!public_ip(value.parse().unwrap()), "{value}");
        }
        for value in [
            "8.8.8.8",
            "1.1.1.1",
            "2606:4700:4700::1111",
            "2001:4860:4860::8888",
        ] {
            assert!(public_ip(value.parse().unwrap()), "{value}");
        }
    }

    #[tokio::test]
    async fn fake_ip_dns_is_supported_only_for_tls_authenticated_domains() {
        let https = checked_url("https://images.example.com/image.png").unwrap();
        let http = checked_url("http://images.example.com/image.png").unwrap();
        for value in [
            "198.18.0.1",
            "198.19.255.254",
            "2001:2::1",
            "2001:2:0:ffff:ffff:ffff:ffff:ffff",
        ] {
            let ip = value.parse().unwrap();
            assert_eq!(resolved_targets(&https, &[ip]).unwrap(), [ip]);
            assert!(resolved_targets(&http, &[ip]).is_err());
            let literal = checked_url(&format!(
                "https://{}/image.png",
                std::net::SocketAddr::new(ip, 443)
            ))
            .unwrap();
            assert!(target(&NetRoute::default(), &literal).await.is_err());
            assert!(resolved_targets(&literal, &[ip]).is_err());
        }
        // A dual-stack answer must work in either order; previously the IPv6
        // mapping rejected the whole answer, even with a usable IPv4 mapping.
        let v4 = "198.18.0.200".parse().unwrap();
        let v6 = "2001:2::c5".parse().unwrap();
        for addresses in [[v4, v6], [v6, v4]] {
            assert_eq!(resolved_targets(&https, &addresses).unwrap(), addresses);
            assert!(resolved_targets(&http, &addresses).is_err());
        }
        for value in [
            "127.0.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "::1",
            "fc00::1",
            "fdfe:dcba:9876::1",
            "::ffff:198.18.0.1",
            "2001:1:ffff:ffff:ffff:ffff:ffff:ffff",
            "2001:2:1::1",
            "2001:3::1",
        ] {
            let ip = value.parse().unwrap();
            assert!(resolved_targets(&https, &[ip]).is_err());
            assert_eq!(resolved_targets(&https, &[v4, ip, v4]).unwrap(), [v4]);
            assert_eq!(resolved_targets(&https, &[v6, ip, v6]).unwrap(), [v6]);
        }
        assert!(resolved_targets(&https, &[]).is_err());
        let public = "8.8.8.8".parse().unwrap();
        assert_eq!(
            resolved_targets(
                &https,
                &[
                    "127.0.0.1".parse().unwrap(),
                    public,
                    "192.168.1.1".parse().unwrap()
                ]
            )
            .unwrap(),
            [public]
        );
    }
    #[tokio::test]
    async fn blocks_credentials_local_targets_and_proxy_dns_before_connecting() {
        assert!(checked_url("https://user:password@example.com/a").is_err());
        assert!(
            fetch(&NetRoute::default(), "http://127.0.0.1/a".into())
                .await
                .is_err()
        );
        let route = NetRoute {
            proxy: Some(super::super::ProxyRoute {
                kind: crate::accounts::types::ProxyKind::Socks5,
                host: "127.0.0.1".into(),
                port: 9,
                auth: None,
            }),
            ..Default::default()
        };
        assert!(
            target(&route, &checked_url("https://example.com/a").unwrap())
                .await
                .is_err()
        );
    }
    async fn synthetic_response(
        headers: &str,
        bytes: &[u8],
    ) -> (Result<ImageReply, ImageFetchError>, String) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (client, mut server) = tokio::io::duplex(65536);
        let reply = [headers.as_bytes(), bytes].concat();
        let task = tokio::spawn(async move {
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(server.read_u8().await.unwrap());
            }
            let _ = server.write_all(&reply).await;
            String::from_utf8(request).unwrap()
        });
        let response = exchange(
            Box::new(client),
            &checked_url("https://images.example.invalid/a?b=1").unwrap(),
        )
        .await;
        (response, task.await.unwrap())
    }

    #[tokio::test]
    async fn real_http_exchange_sends_no_ambient_credentials_and_decodes_images() {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC").unwrap();
        let (result, request) = synthetic_response(&format!("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nSet-Cookie: tracking=1\r\n\r\n", bytes.len()), &bytes).await;
        let ImageReply::Image(image) = result.unwrap() else {
            panic!("expected image")
        };
        assert_eq!(
            image::load_from_memory(&image.bytes).unwrap().to_rgba8(),
            image::load_from_memory(&bytes).unwrap().to_rgba8()
        );
        assert!(request.starts_with("GET /a?b=1 HTTP/1.1\r\n"));
        let headers = request.to_ascii_lowercase();
        assert!(
            !headers.contains("cookie:")
                && !headers.contains("authorization:")
                && !headers.contains("referer:")
        );
    }

    #[tokio::test]
    async fn mislabeled_remote_rasters_are_fully_validated_and_served_with_their_actual_mime() {
        let mut png = Vec::new();
        for (format, declared, expected) in [
            (
                image::ImageFormat::Png,
                "image/jpeg;charset=UTF-8",
                "image/png",
            ),
            (image::ImageFormat::Jpeg, "image/png", "image/jpeg"),
        ] {
            let mut output = std::io::Cursor::new(Vec::new());
            image::DynamicImage::new_rgb8(2, 2)
                .write_to(&mut output, format)
                .unwrap();
            let bytes = output.into_inner();
            let (reply, _) = synthetic_response(
                &format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {declared}\r\nContent-Length: {}\r\n\r\n",
                    bytes.len()
                ),
                &bytes,
            )
            .await;
            let ImageReply::Image(image) = reply.unwrap() else {
                panic!("expected raster image")
            };
            assert_eq!(image.mime_type, expected);
            assert_eq!(
                image::load_from_memory(&image.bytes).unwrap().to_rgba8(),
                image::load_from_memory(&bytes).unwrap().to_rgba8()
            );
            if format == image::ImageFormat::Png {
                png = bytes;
            }
        }
        // A recognized magic number still requires complete decoding. MIME is
        // advisory; active content never becomes an image merely by its label.
        assert!(decode_image(&png[..png.len() - 1], "image/jpeg").is_err());
        assert!(decode_image(b"<html>not an image</html>", "image/png").is_err());
        assert!(decode_image(b"<svg xmlns='http://www.w3.org/2000/svg'/>", "image/jpeg").is_err());
        for declared in ["text/html", "application/octet-stream", "", "image/svg+xml"] {
            assert_eq!(decode_image(&png, declared).unwrap().mime_type, "image/png");
        }
    }

    #[tokio::test]
    async fn remote_svg_becomes_static_png_without_loading_referenced_resources() {
        let directory = tempfile::TempDir::new().unwrap();
        let local = directory.path().join("private.svg");
        std::fs::write(&local, r#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2" height="2" fill="blue"/></svg>"#).unwrap();
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2" onload="alert(1)">
            <rect width="2" height="2" fill="red"/>
            <script>alert(1)</script><foreignObject width="2" height="2"><iframe src="https://example.invalid/"/></foreignObject>
            <image href="{}" width="2" height="2"/>
            <image href="https://example.invalid/private.png" width="2" height="2"/>
            <image href="data:image/svg+xml,%3Csvg/%3E" width="2" height="2"/>
            </svg>"#,
            local.display()
        );
        let (result, _) = synthetic_response(
            &format!(
                "HTTP/1.1 200 OK\r\nContent-Type: image/svg+xml\r\nContent-Length: {}\r\n\r\n",
                svg.len()
            ),
            svg.as_bytes(),
        )
        .await;
        let ImageReply::Image(image) = result.unwrap() else {
            panic!("expected PNG")
        };
        assert_eq!(image.mime_type, "image/png");
        let decoded = image::load_from_memory(&image.bytes).unwrap().to_rgba8();
        assert!(decoded.pixels().all(|pixel| pixel.0 == [255, 0, 0, 255]));
    }

    #[test]
    fn svg_rejects_entities_oversized_output_deep_trees_and_filters() {
        let rect = r#"<rect width="2" height="2"/>"#;
        for svg in [
            "<!DOCTYPE svg [<!ENTITY x 'private'>]><svg xmlns='http://www.w3.org/2000/svg'>&x;</svg>".to_owned(),
            format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="9000" height="9000">{rect}</svg>"#),
            format!(r#"<svg xmlns="http://www.w3.org/2000/svg">{}{rect}{}</svg>"#, "<g>".repeat(40), "</g>".repeat(40)),
            format!(r#"<svg xmlns="http://www.w3.org/2000/svg"><filter id="f"/>{rect}</svg>"#),
            format!(r#"<svg xmlns="http://www.w3.org/2000/svg">{}</svg>"#, rect.repeat(20_001)),
        ] {
            assert!(decode_image(svg.as_bytes(), "image/svg+xml").is_err());
        }
        assert!(decode_image(&vec![b' '; 1024 * 1024 + 1], "image/svg+xml").is_err());
    }

    #[test]
    fn svg_is_identified_by_its_root_and_rasterized_even_with_missing_or_wrong_mime() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2" height="2" fill="red"/></svg>"#;
        for mime in ["", "application/octet-stream", "text/html", "image/png"] {
            let image = decode_image(svg, mime).unwrap();
            assert_eq!(image.mime_type, "image/png");
            let rgba = image::load_from_memory(&image.bytes).unwrap().into_rgba8();
            assert!(rgba.pixels().all(|p| p.0 == [255, 0, 0, 255]));
        }
        for bytes in [
            b"<html><body>error</body></html>".as_slice(),
            b"<svg xmlns='https://other.invalid'/>",
            b"<html xmlns='http://www.w3.org/2000/svg'/>",
        ] {
            assert!(decode_image(bytes, "image/svg+xml").is_err());
        }
    }

    #[tokio::test]
    async fn cancelling_a_decode_waiter_keeps_its_worker_budget_until_work_finishes() {
        let permits = Arc::new(tokio::sync::Semaphore::new(1));
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, work) = std::sync::mpsc::channel();
        let worker = tokio::spawn(decode_on_worker(permits.clone(), move || {
            started.send(()).unwrap();
            work.recv().unwrap();
            Err(ImageFetchError::InvalidImage)
        }));
        ready.await.unwrap();
        worker.abort();
        assert!(matches!(worker.await, Err(error) if error.is_cancelled()));
        assert_eq!(permits.available_permits(), 0);
        release.send(()).unwrap();
        let _released = tokio::time::timeout(Duration::from_secs(1), permits.acquire())
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn http_bounds_and_redirect_destinations_are_checked() {
        for (headers, bytes) in [
            (
                "HTTP/1.1 200 OK\r\nContent-Length: 8388609\r\n\r\n",
                b"".as_slice(),
            ),
            (
                "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 3\r\n\r\n",
                b"bad".as_slice(),
            ),
            (
                "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: 0\r\n\r\n",
                b"".as_slice(),
            ),
            (
                "HTTP/1.1 302 Found\r\nLocation: http://example.invalid/unsafe-downgrade\r\nContent-Length: 0\r\n\r\n",
                b"".as_slice(),
            ),
            (
                "HTTP/1.1 302 Found\r\nLocation: file:///private\r\nContent-Length: 0\r\n\r\n",
                b"".as_slice(),
            ),
        ] {
            assert!(synthetic_response(headers, bytes).await.0.is_err());
        }
        let (redirect, _) = synthetic_response("HTTP/1.1 302 Found\r\nLocation: https://127.0.0.1/private\r\nContent-Length: 0\r\n\r\n", b"").await;
        let ImageReply::Redirect(url) = redirect.unwrap() else {
            panic!("expected redirect")
        };
        assert!(target(&NetRoute::default(), &url).await.is_err());
        let body = vec![b'x'; MAX_BYTES + 1];
        let chunks = [
            format!("{:x}\r\n", body.len()).as_bytes(),
            &body,
            b"\r\n0\r\n\r\n",
        ]
        .concat();
        assert!(
            synthetic_response(
                "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nTransfer-Encoding: chunked\r\n\r\n",
                &chunks
            )
            .await
            .0
            .is_err()
        );
    }

    #[test]
    fn plain_http_image_urls_on_named_hosts_are_tried_over_https_first() {
        let upgrade = |value: &str| https_upgrade(&Url::parse(value).unwrap()).map(String::from);
        assert_eq!(
            upgrade("http://cdn.example.com/a/b.png?x=1&y=2"),
            Some("https://cdn.example.com/a/b.png?x=1&y=2".into())
        );
        assert_eq!(
            upgrade("http://cdn.example.com:80/a.png"),
            Some("https://cdn.example.com/a.png".into())
        );
        for kept in [
            "https://cdn.example.com/a.png",
            "http://cdn.example.com:8080/a.png",
            "http://203.0.113.9/a.png",
            "http://[2001:db8::1]/a.png",
        ] {
            assert_eq!(upgrade(kept), None, "{kept}");
        }
    }
}
