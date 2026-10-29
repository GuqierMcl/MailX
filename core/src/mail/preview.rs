//! List previews built from the start of a message rather than a full download.
//!
//! The input is whatever a partial `UID FETCH` returned: a few top-level header
//! fields and a prefix of the body. The prefix may end inside a base64 group, a
//! quoted-printable escape, a multipart boundary or an HTML tag, so extraction
//! never fails for being truncated; it either finds text or reports none.
use super::render;

/// What a partial fetch yielded for one message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewText {
    /// Non-empty excerpt, already shortened for the list.
    Text(String),
    /// Parsed fine but nothing visible (style-only HTML, empty body, binary).
    Empty,
}

/// `headers` is the `HEADER.FIELDS` literal (ends with a blank line when the
/// server follows RFC 3501) and `body` the `TEXT<0.n>` literal.
pub fn extract(headers: &[u8], body: &[u8]) -> PreviewText {
    let mut raw = Vec::with_capacity(headers.len() + body.len() + 4);
    raw.extend_from_slice(headers);
    // Servers end a field list with an empty line; make that certain.
    while !raw.ends_with(b"\r\n\r\n") && !raw.ends_with(b"\n\n") {
        raw.extend_from_slice(if raw.ends_with(b"\r\n") || raw.ends_with(b"\n") {
            b"\r\n"
        } else {
            b"\r\n\r\n"
        });
    }
    raw.extend_from_slice(body);
    // mail-parser rejects a multipart that never reaches its closing delimiter.
    if let Some(boundary) = boundary(headers) {
        raw.extend_from_slice(b"\r\n--");
        raw.extend_from_slice(&boundary);
        raw.extend_from_slice(b"--\r\n");
    }
    match render::partial_text(&raw) {
        Ok(text) if !text.is_empty() => PreviewText::Text(render::preview(Some(&text))),
        _ => PreviewText::Empty,
    }
}

/// The `boundary` parameter of the top-level Content-Type, quoted or bare.
fn boundary(headers: &[u8]) -> Option<Vec<u8>> {
    let lower = headers.to_ascii_lowercase();
    let at = lower.windows(9).position(|window| window == b"boundary=")? + 9;
    let rest = &headers[at..];
    let value: Vec<u8> = match rest.first()? {
        b'"' => rest[1..]
            .iter()
            .copied()
            .take_while(|b| *b != b'"')
            .collect(),
        _ => rest
            .iter()
            .copied()
            .take_while(|b| !matches!(b, b';' | b' ' | b'\t' | b'\r' | b'\n'))
            .collect(),
    };
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN: &[u8] = b"Content-Type: text/plain; charset=utf-8\r\n\r\n";

    fn text(headers: &[u8], body: &[u8]) -> String {
        match extract(headers, body) {
            PreviewText::Text(text) => text,
            PreviewText::Empty => panic!("expected text"),
        }
    }

    #[test]
    fn truncated_alternative_prefers_the_plain_part() {
        let headers =
            b"MIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=\"b1\"\r\n\r\n";
        let body = b"--b1\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHello   there,\r\nthis is the plain part.\r\n--b1\r\nContent-Type: text/html\r\n\r\n<p>Hello <b>ther";
        assert_eq!(text(headers, body), "Hello there, this is the plain part.");
    }

    #[test]
    fn html_only_goes_through_sanitizer_and_text_conversion() {
        let headers = b"Content-Type: text/html; charset=utf-8\r\n\r\n";
        let body = b"<html><head><style>p{color:red}</style></head><body><script>x()</script><p>Quarterly <b>report</b></p><p>attached</p>";
        assert_eq!(text(headers, body), "Quarterly report attached");
    }

    #[test]
    fn base64_cut_mid_group_keeps_the_complete_prefix() {
        let headers =
            b"Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n";
        // "Hello, base64 world" encoded, cut after 6 characters of a group.
        let body = b"SGVsbG8sIGJhc2U2NCB3b3JsZA==";
        let cut = &body[..10];
        let result = text(headers, cut);
        assert_eq!(result, "Hello,");
    }

    #[test]
    fn quoted_printable_cut_mid_escape_is_tolerated() {
        let headers = b"Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n";
        let result = text(headers, b"caf=C3=A9 au lait =C3");
        assert!(result.starts_with("café au lait"), "{result}");
        let result = text(headers, b"soft=\r\nbreak =");
        assert!(result.starts_with("softbreak"), "{result}");
    }

    #[test]
    fn gbk_charset_is_decoded() {
        let headers =
            b"Content-Type: text/plain; charset=gbk\r\nContent-Transfer-Encoding: 8bit\r\n\r\n";
        // "你好，世界" in GBK.
        let body = [0xC4, 0xE3, 0xBA, 0xC3, 0xA3, 0xAC, 0xCA, 0xC0, 0xBD, 0xE7];
        assert_eq!(text(headers, &body), "你好，世界");
    }

    #[test]
    fn empty_and_style_only_bodies_have_no_text() {
        assert_eq!(extract(PLAIN, b""), PreviewText::Empty);
        assert_eq!(extract(PLAIN, b" \r\n\r\n "), PreviewText::Empty);
        let html = b"Content-Type: text/html\r\n\r\n";
        let style = format!("<style>{}</style>", "p{margin:0}".repeat(1000));
        assert_eq!(extract(html, style.as_bytes()), PreviewText::Empty);
    }

    #[test]
    fn long_text_is_shortened_like_a_full_body_preview() {
        let body = "word ".repeat(200);
        let result = text(PLAIN, body.as_bytes());
        assert_eq!(result.chars().count(), 200);
    }

    #[test]
    fn missing_header_terminator_is_repaired() {
        assert_eq!(text(b"Content-Type: text/plain", b"body"), "body");
        assert_eq!(text(b"", b"bare body"), "bare body");
    }
}
