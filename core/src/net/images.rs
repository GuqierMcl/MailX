//! Bounded, credential-free HTTP for untrusted mail images. This uses the
//! account route, pins checked public addresses and revalidates every redirect.
use super::NetRoute;
use crate::{
    AppError, ErrorCode,
    mail::render::{InlineImage, validate_image},
};
use bytes::Bytes;
use http_body_util::{BodyExt, Empty};
use hyper::{Request, header};
use hyper_util::rt::TokioIo;
use std::{net::IpAddr, time::Duration};
use url::Url;

pub(crate) const MAX_BYTES: usize = 8 * 1024 * 1024;
static DOWNLOADS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
fn rejected() -> AppError {
    AppError::new(ErrorCode::NotFound)
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

fn checked_url(value: &str) -> Result<Url, AppError> {
    let mut url = Url::parse(value).map_err(|_| rejected())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || value.len() > 8192
    {
        return Err(rejected());
    }
    url.set_fragment(None);
    Ok(url)
}

async fn target(route: &NetRoute, url: &Url) -> Result<IpAddr, AppError> {
    let host = url
        .host_str()
        .ok_or_else(rejected)?
        .trim_matches(['[', ']']);
    if let Ok(ip) = host.parse::<IpAddr>() {
        return public_ip(ip).then_some(ip).ok_or_else(rejected);
    }
    // Proxy DNS cannot attest the final address. Do not leak a local lookup or
    // fall back to a direct connection to make this image work.
    if route.proxy.is_some() {
        return Err(rejected());
    }
    let addresses: Vec<_> =
        tokio::net::lookup_host((host, url.port_or_known_default().ok_or_else(rejected)?))
            .await
            .map_err(|_| rejected())?
            .map(|a| a.ip())
            .collect();
    resolved_target(url, &addresses)
}

fn resolved_target(url: &Url, addresses: &[IpAddr]) -> Result<IpAddr, AppError> {
    // Local TUN proxies commonly return RFC 2544 Fake-IP addresses for domains.
    // Treat them as a transport mapping only for HTTPS: the connection still
    // authenticates the original domain with the platform TLS verifier before
    // sending HTTP. Literal benchmark addresses and plaintext HTTP stay blocked.
    let tls_domain = url.scheme() == "https" && matches!(url.host(), Some(url::Host::Domain(_)));
    let allowed = |ip: &IpAddr| {
        public_ip(*ip)
            || (tls_domain
                && matches!(ip, IpAddr::V4(ip) if ip.octets()[0] == 198 && matches!(ip.octets()[1], 18 | 19)))
    };
    if addresses.is_empty() || !addresses.iter().all(allowed) {
        return Err(rejected());
    }
    Ok(addresses[0])
}

struct ConnectionTask(tokio::task::JoinHandle<()>);
impl Drop for ConnectionTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(crate) async fn fetch(route: NetRoute, value: String) -> Result<InlineImage, AppError> {
    tokio::time::timeout(Duration::from_secs(30), async {
        let _permit = DOWNLOADS.acquire().await.map_err(|_| rejected())?;
        let mut url = checked_url(&value)?;
        for redirects in 0..=3 {
            let ip = target(&route, &url).await?;
            let port = url.port_or_known_default().ok_or_else(rejected)?;
            let mut stream = super::connect(&route, &ip.to_string(), port)
                .await
                .map_err(|_| rejected())?;
            if url.scheme() == "https" {
                stream =
                    super::upgrade_tls(stream, url.host_str().unwrap().trim_matches(['[', ']']))
                        .await
                        .map_err(|_| rejected())?;
            }
            match exchange(stream, &url).await? {
                ImageReply::Image(image) => return Ok(image),
                ImageReply::Redirect(next) if redirects < 3 => url = next,
                _ => return Err(rejected()),
            }
        }
        Err(rejected())
    })
    .await
    .map_err(|_| rejected())?
}

enum ImageReply {
    Redirect(Url),
    Image(InlineImage),
}

async fn exchange(stream: super::BoxedStream, url: &Url) -> Result<ImageReply, AppError> {
    let (mut sender, connection) = hyper::client::conn::http1::Builder::new()
        .max_headers(64)
        .max_buf_size(32 * 1024)
        .handshake(TokioIo::new(stream))
        .await
        .map_err(|_| rejected())?;
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
            "image/png,image/jpeg,image/gif,image/webp,image/svg+xml",
        )
        .header(header::ACCEPT_ENCODING, "identity")
        .header(header::CONNECTION, "close")
        .body(Empty::<Bytes>::new())
        .map_err(|_| rejected())?;
    let response = sender.send_request(request).await.map_err(|_| rejected())?;
    if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
        let location = response
            .headers()
            .get(header::LOCATION)
            .and_then(|h| h.to_str().ok())
            .ok_or_else(rejected)?;
        let next = checked_url(url.join(location).map_err(|_| rejected())?.as_str())?;
        if url.scheme() == "https" && next.scheme() != "https" {
            return Err(rejected());
        }
        return Ok(ImageReply::Redirect(next));
    }
    if response.status() != 200
        || response
            .headers()
            .get(header::CONTENT_ENCODING)
            .is_some_and(|v| v != "identity")
        || response
            .headers()
            .get(header::CONTENT_LENGTH)
            .is_some_and(|v| {
                v.to_str()
                    .ok()
                    .and_then(|v| v.parse::<usize>().ok())
                    .is_none_or(|n| n > MAX_BYTES)
            })
    {
        return Err(rejected());
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
        let frame = frame.map_err(|_| rejected())?;
        if let Ok(data) = frame.into_data() {
            if bytes.len().saturating_add(data.len()) > MAX_BYTES {
                return Err(rejected());
            }
            bytes.extend_from_slice(&data);
        }
    }
    let image = tokio::task::spawn_blocking(move || decode_image(&bytes, &mime))
        .await
        .map_err(|_| rejected())??;
    Ok(ImageReply::Image(image))
}

fn decode_image(bytes: &[u8], mime: &str) -> Result<InlineImage, AppError> {
    if mime != "image/svg+xml" {
        return validate_image(bytes, mime).map_err(|_| rejected());
    }
    // Remote SVGs never reach the WebView. Rasterize static vector shapes only,
    // with no scripts, external/local images, fonts, nested SVGs or SVGZ decoding.
    use resvg::{tiny_skia, usvg};
    if bytes.len() > 1024 * 1024 {
        return Err(rejected());
    }
    let xml = usvg::roxmltree::Document::parse_with_options(
        std::str::from_utf8(bytes).map_err(|_| rejected())?,
        usvg::roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 20_000,
            ..Default::default()
        },
    )
    .map_err(|_| rejected())?;
    if xml
        .descendants()
        .any(|node| node.ancestors().count() > 32 || node.has_tag_name("filter"))
    {
        return Err(rejected());
    }
    let options = usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_string: Box::new(|_, _| None),
            resolve_data: Box::new(|_, _, _| None),
        },
        ..Default::default()
    };
    let tree = usvg::Tree::from_xmltree(&xml, &options).map_err(|_| rejected())?;
    let size = tree.size().to_int_size();
    if size.width() > 4096
        || size.height() > 4096
        || u64::from(size.width()) * u64::from(size.height()) > 4 * 1024 * 1024
        || !tree.root().has_children()
    {
        return Err(rejected());
    }
    let mut pixmap = tiny_skia::Pixmap::new(size.width(), size.height()).ok_or_else(rejected)?;
    resvg::render(
        &tree,
        tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    Ok(InlineImage {
        bytes: pixmap.encode_png().map_err(|_| rejected())?,
        mime_type: "image/png".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
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
        for value in ["198.18.0.1", "198.19.255.254"] {
            let ip = value.parse().unwrap();
            assert_eq!(resolved_target(&https, &[ip]).unwrap(), ip);
            assert!(resolved_target(&http, &[ip]).is_err());
            let literal = checked_url(&format!("https://{value}/image.png")).unwrap();
            assert!(target(&NetRoute::default(), &literal).await.is_err());
            assert!(resolved_target(&literal, &[ip]).is_err());
        }
        for value in ["127.0.0.1", "192.168.1.1", "169.254.169.254", "::1"] {
            let ip = value.parse().unwrap();
            assert!(resolved_target(&https, &[ip]).is_err());
            assert!(resolved_target(&https, &["198.18.0.1".parse().unwrap(), ip]).is_err());
        }
        assert!(resolved_target(&https, &[]).is_err());
    }
    #[tokio::test]
    async fn blocks_credentials_local_targets_and_proxy_dns_before_connecting() {
        assert!(checked_url("https://user:password@example.com/a").is_err());
        assert!(
            fetch(NetRoute::default(), "http://127.0.0.1/a".into())
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
    ) -> (Result<ImageReply, AppError>, String) {
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
        assert_eq!(image.bytes, bytes);
        assert!(request.starts_with("GET /a?b=1 HTTP/1.1\r\n"));
        let headers = request.to_ascii_lowercase();
        assert!(
            !headers.contains("cookie:")
                && !headers.contains("authorization:")
                && !headers.contains("referer:")
        );
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
}
