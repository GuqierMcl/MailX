use base64::{Engine, engine::general_purpose::STANDARD};

use super::*;

// Independently generated PNG: a 1×1 red RGB pixel, zlib stream and CRCs.
const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC";

fn email(content_type: &str, body: &str) -> Vec<u8> {
    format!("MIME-Version: 1.0\r\nContent-Type: {content_type}\r\n\r\n{body}").into_bytes()
}

fn rendered_html(body: &str) -> Rendered {
    render(&email("text/html; charset=utf-8", body), "owner").unwrap()
}

#[test]
fn plain_text_is_escaped_never_parsed_as_markup() {
    let result = render(
        &email(
            "text/plain; charset=utf-8",
            "正文 <b>literal</b> & <script>x()</script>",
        ),
        "owner",
    )
    .unwrap();
    assert_eq!(
        result.plain_text.as_deref(),
        Some("正文 <b>literal</b> & <script>x()</script>")
    );
    let html = result.sanitized_html.unwrap();
    assert!(
        html.contains("&lt;b&gt;literal&lt;/b&gt; &amp; &lt;script&gt;"),
        "{html}"
    );
    assert!(!html.contains("<b>") && !html.contains("<script"), "{html}");
    assert!(result.links.is_empty() && result.parts.is_empty());
}

#[test]
fn plain_text_links_are_clickable_and_stop_at_punctuation_and_cjk() {
    let result = render(
        &email(
            "text/plain; charset=utf-8",
            "见 https://a.example/x?a=1&b=2，谢谢。(https://b.example/p). http://c.example/ok! \"https://d.example\"; https:// javascript:alert(1)",
        ),
        "owner",
    )
    .unwrap();
    assert_eq!(
        result.links,
        [
            "https://a.example/x?a=1&b=2",
            "https://b.example/p",
            "http://c.example/ok",
            "https://d.example/"
        ]
    );
    let html = result.sanitized_html.unwrap();
    assert!(
        !html.contains("javascript:") || !html.contains("href=\"javascript:"),
        "{html}"
    );
}

#[test]
fn plain_text_quotes_become_nested_blockquotes_with_bounded_depth() {
    let deep = format!("{} deep", ">".repeat(40));
    let result = render(
        &email(
            "text/plain; charset=utf-8",
            &format!("new\n> one\n> > two\nback\n{deep}"),
        ),
        "owner",
    )
    .unwrap();
    let html = result.sanitized_html.unwrap();
    assert_eq!(html.matches("<blockquote").count(), 2 + 8, "{html}");
    assert_eq!(html.matches("</blockquote>").count(), 2 + 8, "{html}");
    assert!(
        html.starts_with("<div style=\"white-space:pre-wrap\">new"),
        "{html}"
    );
}

#[test]
fn blank_plain_text_has_no_document() {
    let result = render(&email("text/plain", "  \n "), "owner").unwrap();
    assert!(result.sanitized_html.is_none());
}

#[test]
fn html_text_and_preview_exclude_script_and_style_text() {
    let result = rendered_html(
        "<style>p { color: red }</style><script>secret()</script><p>你好</p><b>世界 &amp; others</b>",
    );
    assert_eq!(result.plain_text.as_deref(), Some("你好 世界 & others"));
    assert_eq!(preview(Some(&"界".repeat(220))).chars().count(), 200);
    assert_eq!(preview(None), "");
}

#[test]
fn gbk_plain_and_html_decode_by_declared_charset() {
    for (mime, markup) in [("text/plain", false), ("text/html", true)] {
        let mut raw = format!("Content-Type: {mime}; charset=gbk\r\n\r\n").into_bytes();
        if markup {
            raw.extend_from_slice(b"<p>");
        }
        raw.extend_from_slice(&[0xd5, 0xfd, 0xce, 0xc4, 0xb2, 0xe2, 0xca, 0xd4]);
        if markup {
            raw.extend_from_slice(b"</p>");
        }
        let result = render(&raw, "owner").unwrap();
        assert_eq!(result.plain_text.as_deref(), Some("正文测试"));
        assert!(
            result.sanitized_html.unwrap().contains("正文测试"),
            "{mime}"
        );
        let _ = markup;
    }
}

#[test]
fn alternative_preserves_real_plain_and_html_with_attachment_metadata() {
    let raw = email(
        "multipart/mixed; boundary=m",
        &format!(
            "--m\r\nContent-Type: multipart/alternative; boundary=a\r\n\r\n\
        --a\r\nContent-Type: text/plain\r\n\r\nplain choice\r\n\
        --a\r\nContent-Type: text/html\r\n\r\n<p>html choice<img src=\"cid:logo\"></p>\r\n--a--\r\n\
        --m\r\nContent-Type: image/png\r\nContent-Disposition: inline; filename=logo.png\r\nContent-ID: <logo>\r\nContent-Transfer-Encoding: base64\r\n\r\n{PNG}\r\n\
        --m\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=notes.pdf\r\nContent-Transfer-Encoding: base64\r\n\r\nUERG\r\n--m--\r\n"
        ),
    );
    let result = render(&raw, "owner").unwrap();
    assert_eq!(result.plain_text.as_deref(), Some("plain choice"));
    assert!(
        result
            .sanitized_html
            .as_deref()
            .unwrap()
            .contains("html choice")
    );
    assert_eq!(result.parts.len(), 2);
    let picture = &result.parts[0];
    assert_eq!(picture.part_index, 4);
    assert_eq!(picture.content_id.as_deref(), Some("logo"));
    assert_eq!(picture.filename.as_deref(), Some("logo.png"));
    assert!(picture.is_inline);
    assert_eq!(picture.size, STANDARD.decode(PNG).unwrap().len() as u64);
    let file = &result.parts[1];
    assert_eq!(file.filename.as_deref(), Some("notes.pdf"));
    assert_eq!(file.mime_type, "application/pdf");
    assert_eq!(file.size, 3);
    assert!(!file.is_inline);
    assert_eq!(
        inline_image(&raw, picture.part_index, "logo")
            .unwrap()
            .mime_type,
        "image/png"
    );
    assert_eq!(
        inline_image(&raw, picture.part_index, "someone-else").err(),
        Some(RenderError::InvalidImage)
    );
}

#[test]
fn every_listed_attachment_is_decodable_in_one_pass() {
    let raw = email(
        "multipart/mixed; boundary=m",
        &format!(
            "--m\r\nContent-Type: multipart/alternative; boundary=a\r\n\r\n\
        --a\r\nContent-Type: text/plain\r\n\r\nplain choice\r\n\
        --a\r\nContent-Type: text/html\r\n\r\n<p>html choice<img src=\"cid:logo\"></p>\r\n--a--\r\n\
        --m\r\nContent-Type: image/png\r\nContent-Disposition: inline; filename=logo.png\r\nContent-ID: <logo>\r\nContent-Transfer-Encoding: base64\r\n\r\n{PNG}\r\n\
        --m\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=notes.pdf\r\nContent-Transfer-Encoding: base64\r\n\r\nUERG\r\n--m--\r\n"
        ),
    );
    let files = attachment_contents(&raw).unwrap();
    let [file] = &files[..] else {
        panic!("only the PDF is an attachment")
    };
    assert_eq!(file.file_name.as_deref(), Some("notes.pdf"));
    assert_eq!(file.mime_type, "application/pdf");
    assert_eq!(file.bytes, b"PDF");
    // The same parts, read one by one, agree.
    let listed = render(&raw, "owner").unwrap();
    let pdf = listed.parts.iter().find(|part| !part.is_inline).unwrap();
    assert_eq!(
        attachment_content(&raw, pdf.part_index).unwrap().bytes,
        files[0].bytes
    );
}

#[test]
fn nested_message_entities_do_not_disappear_or_replace_outer_body() {
    let raw = email(
        "multipart/mixed; boundary=m",
        "--m\r\nContent-Type: text/plain\r\n\r\nouter\r\n--m\r\nContent-Type: message/rfc822\r\nContent-Disposition: attachment; filename=attached.eml\r\n\r\nSubject: nested\r\nContent-Type: multipart/mixed; boundary=n\r\n\r\n--n\r\nContent-Type: text/html\r\n\r\n<p>nested body</p>\r\n--n\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=nested.bin\r\n\r\nabc\r\n--n--\r\n--m--\r\n",
    );
    let result = render(&raw, "owner").unwrap();
    assert_eq!(result.plain_text.as_deref(), Some("outer"));
    assert!(result.sanitized_html.unwrap().contains("outer"));
    assert!(
        result
            .parts
            .iter()
            .any(|part| part.filename.as_deref() == Some("attached.eml"))
    );
    assert!(
        result
            .parts
            .iter()
            .any(|part| part.filename.as_deref() == Some("nested.bin"))
    );
    assert!(
        result
            .parts
            .iter()
            .any(|part| part.mime_type == "text/html")
    );
    let ids: std::collections::HashSet<_> =
        result.parts.iter().map(|part| part.part_index).collect();
    assert_eq!(ids.len(), result.parts.len());
}

#[test]
fn mime_depth_is_limited_and_invalid_containers_are_rejected() {
    fn nested(containers: usize) -> String {
        let mut message = "Content-Type: text/plain\r\n\r\nleaf".to_owned();
        for index in 0..containers {
            message = format!(
                "Content-Type: multipart/mixed; boundary=b{index}\r\n\r\n--b{index}\r\n{message}\r\n--b{index}--\r\n"
            );
        }
        message
    }
    assert!(render(nested(29).as_bytes(), "owner").is_ok());
    assert_eq!(
        render(nested(30).as_bytes(), "owner").err(),
        Some(RenderError::Protocol)
    );
    assert_eq!(
        render(
            &email("multipart/mixed; boundary=absent", "not a boundary"),
            "owner"
        )
        .err(),
        Some(RenderError::Protocol)
    );
}

#[test]
fn retained_body_is_utf8_bounded_and_explicitly_truncated() {
    let body = format!("{}界界", "a".repeat(MAX_BODY_BYTES - 1));
    let result = render(&email("text/plain; charset=utf-8", &body), "owner").unwrap();
    assert!(result.truncated);
    assert_eq!(result.plain_text.unwrap().len(), MAX_BODY_BYTES - 1);
    let result = rendered_html(&format!("<p>{}", "x".repeat(MAX_BODY_BYTES)));
    assert!(result.truncated);
    assert!(result.sanitized_html.is_some());
}

#[test]
fn raw_message_size_limit_is_enforced_before_parsing() {
    assert_eq!(
        render(&vec![b'x'; MAX_MESSAGE_BYTES + 1], "owner").err(),
        Some(RenderError::TooLarge)
    );
}

#[test]
fn malicious_html_samples_do_not_survive_as_active_content() {
    for input in [
        "<script>alert(1)</script><p>safe</p>",
        "<img src=x onerror=alert(1)><b onclick=alert(1)>safe</b>",
        "<a href=javascript:alert(1)>safe</a>",
        "<a href=JaVaScRiPt:alert(1)>safe</a>",
        "<a href=javascript&colon;alert(1)>safe</a>",
        "<svg><script>alert(1)</script><foreignObject><img onerror=x src=x></foreignObject></svg>",
        "<math><mtext><table><mglyph><style><!--</style><img title=\"--><img src=x onerror=alert(1)>\">",
        "<meta http-equiv=refresh content='0;url=https://evil.invalid'>",
        "<base href=https://evil.invalid/><a href=/relative>safe</a>",
        "<form action=https://evil.invalid><input autofocus onfocus=x><button>send</button></form>",
        "<iframe src=https://evil.invalid srcdoc='<script>x</script>'></iframe><object data=x></object><embed src=x>",
    ] {
        let result = rendered_html(input);
        let html = result.sanitized_html.unwrap().to_ascii_lowercase();
        for forbidden in [
            "<script",
            "onerror",
            "onclick",
            "javascript:",
            "<svg",
            "<math",
            "http-equiv",
            "<base",
            "<form",
            "<input",
            "<iframe",
            "<object",
            "<embed",
            "srcdoc=",
        ] {
            assert!(
                !html.contains(forbidden),
                "forbidden construct {forbidden} survived"
            );
        }
        assert!(result.links.is_empty());
    }
}

#[test]
fn css_is_structurally_rebuilt_and_remote_image_references_count_once() {
    let result = rendered_html(
        r#"
      <style>@import url(https://evil.invalid/import);
      @font-face { font-family: bait; src: url(https://evil.invalid/font); }
      p { color:red; background-image: url(https://evil.invalid/css); }
      @media screen { p { background: url(https://evil.invalid/media); } }
      </style>
      <p style="font-size: 14px; color: blue; background: u\72l('https://evil.invalid/escaped'); width: e\78pression(alert(1)); behavior: url(https://evil.invalid/behavior); -moz-binding:url(https://evil.invalid/binding)">safe</p>
      <img src="https://evil.invalid/src" srcset="https://evil.invalid/one 1x, //evil.invalid/two 2x">
      <table background="//evil.invalid/background"></table><video poster="https://evil.invalid/poster"></video>
      <p style="background-image: image-set('https://evil.invalid/set' 1x, url(//evil.invalid/set2) 2x)">safe</p>
    "#,
    );
    let html = result.sanitized_html.unwrap();
    assert_eq!(result.remote_images_blocked, 10);
    assert_eq!(remote_images(&html), ["https://evil.invalid/src"]);
    let html = bind_resources(&html, |_| None);
    assert!(
        html.contains("color:red")
            && html.contains("font-size:14px")
            && html.contains("color:blue")
    );
    for forbidden in [
        "url(",
        "@import",
        "font-face",
        "expression",
        "-moz-binding",
        "behavior",
        "evil.invalid",
        "srcset",
        "poster",
        "background=",
    ] {
        assert!(
            !html.contains(forbidden),
            "forbidden CSS/resource {forbidden} survived"
        );
    }
}

#[test]
fn css_parse_failures_and_escaped_active_values_fail_closed() {
    for style in [
        "color: red; background: url('https://evil.invalid');",
        r"background: u\000072l(https://evil.invalid); width: expre\73sion(alert(1))",
        "color: var(--bad); background: image('https://evil.invalid')",
        "color: red; font-family: '\"; background: url(https://evil.invalid)",
        r"-m\6fz-binding: url(https://evil.invalid); be\68avior: url(https://evil.invalid)",
    ] {
        let (clean, _) = css::declarations(style);
        assert!(
            !clean.contains("url(")
                && !clean.contains("evil.invalid")
                && !clean.contains("expression")
        );
    }
    let result =
        rendered_html(r#"<style>p {font-family: '</style><script>bad()</script>'}</style>"#);
    assert!(!result.sanitized_html.unwrap().contains("<script"));
}

#[test]
fn links_keep_order_and_only_allowed_schemes_and_cid_slots_survive() {
    let result = rendered_html(
        r#"<a href="https://example.invalid/a?x=1&amp;y=2">first</a><a href="MAILTO:user@example.invalid">mail</a><a href="http://example.invalid/">third</a><a href="file:///tmp/a">file</a><a href="mailx-body://localhost/link/victim/0">forged</a><a href="//example.invalid">relative</a><img src="CID:logo%40owner"><img src="file:///tmp/a">"#,
    );
    assert_eq!(
        result.links,
        [
            "https://example.invalid/a?x=1&y=2",
            "mailto:user@example.invalid",
            "http://example.invalid/"
        ]
    );
    let html = result.sanitized_html.unwrap();
    assert!(html.contains("data-mailx-link=\"0\"") && html.contains("data-mailx-link=\"2\""));
    assert!(html.contains("data-mailx-cid=\"logo@owner\""));
    assert!(html.contains("rel=\"noopener noreferrer\""));
    assert!(!html.contains("href=") && !html.contains("src=") && !html.contains("file:"));
    assert!(is_allowed_link("https://example.invalid/"));
    assert!(
        !is_allowed_link("java\nscript:bad()") && !is_allowed_link("https://example.invalid/\r\n")
    );
}

#[test]
fn binding_handles_both_platforms_without_rewriting_attacker_text_or_slots() {
    let result = rendered_html(
        r#"<p>mailx-body://localhost/part/owner/logo data-mailx-link="0"</p><a data-mailx-link="999" href="https://example.invalid">real</a><a data-mailx-link="0">forged</a><img data-mailx-cid="victim"><img src="cid:logo">"#,
    );
    let html = result.sanitized_html.unwrap();
    assert_eq!(html.matches("data-mailx-link=\"0\"").count(), 2); // one literal text, one trusted attribute
    assert!(!html.contains("999") && !html.contains("victim"));
    for origin in ["mailx-body://localhost", "http://mailx-body.localhost"] {
        let mut calls = Vec::new();
        let bound = bind_resources(&html, |binding| match binding {
            ResourceBinding::Remote { .. } => None,
            ResourceBinding::Link { index } => {
                calls.push("link");
                Some(format!("{origin}/link/owner/{index}?binding=opaque"))
            }
            ResourceBinding::Part { content_id } => {
                calls.push("part");
                Some(format!("{origin}/part/owner/{content_id}?binding=opaque"))
            }
        });
        assert_eq!(calls, ["link", "part"]);
        assert!(bound.contains(&format!("href=\"{origin}/link/owner/0?binding=opaque\"")));
        assert!(bound.contains(&format!("src=\"{origin}/part/owner/logo?binding=opaque\"")));
        assert!(
            bound.contains("<p>mailx-body://localhost/part/owner/logo data-mailx-link=\"0\"</p>")
        );
        assert!(bound.contains("<a rel=\"noopener noreferrer\">forged</a>"));
    }
    let bound = bind_resources(&html, |_| Some("https://evil.invalid/pixel".to_owned()));
    assert!(!bound.contains("href=") && !bound.contains("src="));
}

#[test]
fn numbered_binding_keeps_order_and_cids_without_creating_navigable_anchors() {
    let rendered = rendered_html(
        r#"<p>literal [1] data-mailx-link="1"</p><a data-mailx-link="999" href="https://first.invalid">first</a><a data-mailx-link="0">forged [99]</a><a href="mailto:second@example.invalid"><b>second</b></a><img src="cid:logo">"#,
    );
    assert_eq!(
        rendered.links,
        ["https://first.invalid/", "mailto:second@example.invalid"]
    );
    let bound =
        bind_numbered_resources(&rendered.sanitized_html.unwrap(), |binding| match binding {
            ResourceBinding::Remote { .. } => None,
            ResourceBinding::Part { content_id } => Some(format!(
                "mailx-body://localhost/part/owner/{content_id}?token=opaque"
            )),
            ResourceBinding::Link { .. } => {
                panic!("numbered links must never resolve navigable URLs")
            }
        });
    assert!(!bound.contains("href=") && !bound.contains("999"));
    assert!(bound.contains("<p>literal [1] data-mailx-link=\"1\"</p>"));
    assert!(bound.contains("first [1]</a>"));
    assert!(bound.contains(">forged [99]</a>"));
    assert!(bound.contains("<b>second</b> [2]</a>"));
    assert!(bound.contains("src=\"mailx-body://localhost/part/owner/logo?token=opaque\""));
}

#[test]
fn only_valid_passive_data_images_are_restored() {
    let data = format!("data:image/png;base64,{PNG}");
    let result = rendered_html(&format!(
        r#"<img src="{data}"><img src="data:image/svg+xml;base64,PHN2Zy8+"><img src="data:text/html;base64,PGgxPmJhZDwvaDE+"><img src="data:image/png;base64,PGh0bWw+YmFkPC9odG1sPg=="><img data-mailx-data="0"><a href="{data}">no</a>"#
    ));
    let html = result.sanitized_html.unwrap();
    assert_eq!(html.matches("src=\"data:image/png").count(), 1);
    assert!(
        !html.contains("image/svg")
            && !html.contains("text/html")
            && !html.contains("data-mailx-data")
    );
    assert!(result.links.is_empty());
    assert_eq!(result.remote_images_blocked, 0);
    assert!(
        images::data_url(&format!(
            "data:image/png;base64,{}",
            "A".repeat(MAX_DATA_IMAGE_URL_BYTES)
        ))
        .is_none()
    );
    let percent = STANDARD
        .decode(PNG)
        .unwrap()
        .iter()
        .map(|byte| format!("%{byte:02x}"))
        .collect::<String>();
    assert_eq!(
        images::data_url(&format!("data:image/png,{percent}")),
        Some(data)
    );
}

#[test]
fn images_require_real_decoding_size_limits_and_matching_mime() {
    let bytes = STANDARD.decode(PNG).unwrap();
    assert!(images::validate(&bytes, "image/png").is_ok());
    assert!(images::validate(&bytes, "image/jpeg").is_err());
    assert!(images::validate(b"<svg><script>bad()</script></svg>", "image/png").is_err());
    assert!(images::validate(&bytes[..bytes.len() - 4], "image/png").is_err());
    let mut appended = bytes.clone();
    appended.extend_from_slice(b"<script>bad()</script>");
    assert!(images::validate(&appended, "image/png").is_err());
    assert!(images::validate(&vec![0; MAX_INLINE_IMAGE_BYTES + 1], "image/png").is_err());
    let mut cursor = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(8_193, 1)
        .write_to(&mut cursor, image::ImageFormat::Png)
        .unwrap();
    assert!(images::validate(cursor.get_ref(), "image/png").is_err());
    let mut cursor = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(1, 1)
        .write_to(&mut cursor, image::ImageFormat::Jpeg)
        .unwrap();
    assert!(images::validate(cursor.get_ref(), "image/jpeg").is_ok());
}

#[test]
fn duplicate_cid_is_deterministic_and_never_bypasses_part_identity() {
    let raw = email(
        "multipart/related; boundary=m",
        &format!(
            "--m\r\nContent-Type: text/html\r\n\r\n<img src=cid:same>\r\n--m\r\nContent-Type: image/png\r\nContent-ID: <same>\r\nContent-Transfer-Encoding: base64\r\n\r\n{PNG}\r\n--m\r\nContent-Type: image/png\r\nContent-ID: <same>\r\nContent-Transfer-Encoding: base64\r\n\r\n{PNG}\r\n--m--\r\n"
        ),
    );
    let result = render(&raw, "owner").unwrap();
    assert_eq!(result.parts.len(), 2);
    assert!(inline_image(&raw, result.parts[0].part_index, "same").is_ok());
    assert_eq!(
        inline_image(&raw, result.parts[1].part_index, "same").err(),
        Some(RenderError::InvalidImage)
    );
}

#[test]
fn deeply_nested_attached_messages_are_rejected_without_recursive_drop() {
    let mut raw = "Content-Type: message/rfc822\r\n\r\n"
        .repeat(2_000)
        .into_bytes();
    raw.extend_from_slice(b"Content-Type: text/plain\r\n\r\nleaf");
    assert_eq!(render(&raw, "owner").err(), Some(RenderError::Protocol));
}

#[test]
fn animated_png_and_declared_decoding_bombs_are_rejected() {
    let png = STANDARD.decode(PNG).unwrap();
    let mut animated = png[..33].to_vec();
    // The container gate rejects acTL before any decoder sees it; its CRC does
    // not matter because animation is unsupported regardless of validity.
    animated.extend_from_slice(b"\0\0\0\x08acTL\0\0\0\x01\0\0\0\0\0\0\0\0");
    animated.extend_from_slice(&png[33..]);
    assert!(images::validate(&animated, "image/png").is_err());

    // A syntactically valid IHDR for an image within the dimension limit whose
    // decoded RGB buffer would require 192 MiB, beyond the 64 MiB budget.
    let mut bomb = png.clone();
    bomb[16..20].copy_from_slice(&8_192u32.to_be_bytes());
    bomb[20..24].copy_from_slice(&8_192u32.to_be_bytes());
    let mut crc = !0u32;
    for byte in &bomb[12..29] {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    bomb[29..33].copy_from_slice(&(!crc).to_be_bytes());
    assert!(images::validate(&bomb, "image/png").is_err());
}

#[test]
fn remote_images_are_inert_until_host_binding_and_forged_slots_are_removed() {
    let rendered = rendered_html(
        r#"<img src="https://example.com/pixel"><img data-mailx-remote="https://evil.invalid/forged"><a title="misleading" href="https://example.com/real">link</a>"#,
    );
    let html = rendered.sanitized_html.unwrap();
    assert_eq!(remote_images(&html), ["https://example.com/pixel"]);
    let blocked = bind_resources(&html, |_| None);
    assert!(!blocked.contains("src="));
    assert!(!blocked.contains("data-mailx-"));
    assert!(blocked.contains("title=\"https://example.com/real\""));
    let allowed = bind_resources(&html, |slot| match slot {
        ResourceBinding::Remote { index } => Some(format!(
            "mailx-body://localhost/remote/owner/{index}?token=opaque"
        )),
        _ => None,
    });
    assert!(allowed.contains("src=\"mailx-body://localhost/remote/owner/0?token=opaque\""));
    assert!(!allowed.contains("evil.invalid"));
}

#[test]
fn gif_and_webp_are_validated_and_rendered_as_a_bounded_first_frame() {
    use image::{DynamicImage, ImageFormat};
    for (format, mime) in [
        (ImageFormat::Gif, "image/gif"),
        (ImageFormat::WebP, "image/webp"),
    ] {
        let mut data = std::io::Cursor::new(Vec::new());
        DynamicImage::new_rgb8(2, 3)
            .write_to(&mut data, format)
            .unwrap();
        let rendered = validate_image(data.get_ref(), mime).unwrap();
        assert_eq!(rendered.mime_type, "image/png");
        assert_eq!(
            image::guess_format(&rendered.bytes).unwrap(),
            ImageFormat::Png
        );
        assert!(validate_image(b"not an image", mime).is_err());
    }
}

#[test]
fn document_title_text_does_not_leak_into_the_body() {
    let result = rendered_html(
        "<html><head><title>Subject line</title></head><body><p>Hello</p></body></html>",
    );
    let html = result.sanitized_html.unwrap();
    assert!(!html.contains("Subject line"), "{html}");
    assert!(html.contains("Hello"));
    assert_eq!(result.plain_text.as_deref(), Some("Hello"));
}

#[test]
fn body_and_legacy_layout_attributes_survive_for_centering() {
    let html = rendered_html(
        r##"<html><body bgcolor="#eee" style="text-align:center" onload="x()"><table align="center" width="600" cellpadding="0" border="0"><tr><td align="center" valign="top">x</td></tr></table><div align="center">y</div></body></html>"##,
    )
    .sanitized_html
    .unwrap();
    assert!(
        !html.contains("<body") && !html.contains("onload"),
        "{html}"
    );
    assert!(
        html.contains(r#"class="mailx-body""#) && html.contains(r##"bgcolor="#eee""##),
        "{html}"
    );
    for expected in [
        r#"width="600""#,
        r#"cellpadding="0""#,
        r#"valign="top""#,
        r#"<div align="center">y"#,
    ] {
        assert!(html.contains(expected), "{expected} in {html}");
    }
}

#[test]
fn harmless_presentational_markup_and_css_are_kept() {
    let html = rendered_html(
        r##"<style>
        #hdr{color:red} a[href^="https"]{color:blue} td:not(.x){padding:1px}
        @media (max-width:600px){.w{width:100%}} @media (prefers-color-scheme:dark){p{color:white}}
        .b{background:linear-gradient(#fff,#eee);box-shadow:0 0 1px #000;word-wrap:break-word;border-top-color:#ccc}
        </style><section id="hdr"><font color="red" size="2">a</font><big>b</big>
        <table><tfoot><tr><td nowrap>c</td></tr></tfoot></table></section>"##,
    )
    .sanitized_html
    .unwrap();
    for expected in [
        "#hdr{color:red}",
        r#"a[href^="https"]{color:blue}"#,
        "td:not(.x){padding:1px}",
        "@media (max-width:600px){.w{width:100%}}",
        "linear-gradient(",
        "box-shadow:",
        "word-wrap:break-word",
        "border-top-color:#ccc",
        "<font",
        "<tfoot",
        "<section",
        r#"id="hdr""#,
    ] {
        assert!(html.contains(expected), "{expected} in {html}");
    }
    assert!(!html.contains("prefers-color-scheme"), "{html}");
}

#[test]
fn relaxed_css_and_markup_still_block_active_or_remote_content() {
    let result = rendered_html(
        r##"<style>a[x="\3c /style>"]{color:red}
        .a{background:linear-gradient(url(https://t.example/x.png),#fff)}
        .b{background-image:url("https://t.example/y.png")}
        @media screen{.c{background:url(https://t.example/z.png)}}
        @import url(https://t.example/i.css); @font-face{src:url(https://t.example/f.woff)}
        </style><form action="https://t.example"><input name=x></form><button>go</button>
        <svg><script>1</script></svg><video src="https://t.example/v.mp4"></video>
        <div id="x" onclick="alert(1)" aria-label="ok">kept</div>"##,
    );
    let html = result.sanitized_html.unwrap();
    for forbidden in [
        "<script",
        "alert(1)",
        "t.example",
        "<form",
        "<input",
        "<button",
        "<svg",
        "<video",
        "onclick",
    ] {
        assert!(!html.contains(forbidden), "{forbidden} in {html}");
    }
    assert!(
        html.contains("kept") && html.contains(r#"aria-label="ok""#),
        "{html}"
    );
}

#[test]
fn body_marker_merges_with_existing_class_and_cannot_be_forged_into_other_attributes() {
    let html = rendered_html(r#"<body class="x"><p data-mailx-body="1" class="y">t</p></body>"#)
        .sanitized_html
        .unwrap();
    assert!(html.contains(r#"class="mailx-body x""#), "{html}");
    assert!(!html.contains("data-mailx-body"), "{html}");
}

#[test]
fn favicons_are_decoded_under_limits_and_always_leave_as_png() {
    use image::{DynamicImage, ImageFormat};
    let mut ico = std::io::Cursor::new(Vec::new());
    DynamicImage::new_rgba8(32, 32)
        .write_to(&mut ico, ImageFormat::Ico)
        .unwrap();
    for mime in ["image/x-icon", "image/vnd.microsoft.icon", "IMAGE/ICO"] {
        let rendered = validate_image(ico.get_ref(), mime).unwrap();
        assert_eq!(rendered.mime_type, "image/png", "{mime}");
        assert_eq!(
            image::guess_format(&rendered.bytes).unwrap(),
            ImageFormat::Png
        );
    }
    // The declared type must match, and garbage is not an icon.
    assert!(validate_image(ico.get_ref(), "image/png").is_err());
    assert!(validate_image(b"\0\0\x01\0\x01\0 not really an icon", "image/x-icon").is_err());
    assert!(validate_image(b"", "image/x-icon").is_err());
}
