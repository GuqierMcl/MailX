//! Synthetic local messages exercise transfer decoding and text preparation.
//! They do not establish accuracy on real provider templates.

use base64::{Engine, engine::general_purpose::STANDARD};
use mailx_core::mail::verification_code::{
    Detection, DetectionError, MAX_EML_BYTES, MAX_TEXT_BYTES, detect_eml,
};

fn found(raw: &[u8], expected: &str) {
    let result = detect_eml(raw).expect("synthetic MIME should parse");
    let Detection::Found { candidate } = result else {
        panic!("synthetic MIME did not produce one candidate");
    };
    assert_eq!(candidate.code, expected);
}

fn plain(subject: &str, body: &str) -> Vec<u8> {
    format!(
        "From: sender@example.test\r\nSubject: {subject}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{body}"
    )
    .into_bytes()
}

fn html(body: &str) -> Vec<u8> {
    format!(
        "From: sender@example.test\r\nSubject: Sign in\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{body}"
    )
    .into_bytes()
}

#[test]
fn reads_the_committed_synthetic_sample() {
    found(
        include_bytes!("../../tests/fixtures/verification-code/sample.eml"),
        "482913",
    );
}

#[test]
fn decodes_encoded_subject_and_preserves_leading_zeroes() {
    let subject = STANDARD.encode("验证码：001234");
    found(&plain(&format!("=?UTF-8?B?{subject}?="), ""), "001234");
}

#[test]
fn decodes_base64_and_quoted_printable_bodies() {
    let base64 = format!(
        "Subject: Sign in\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{}",
        STANDARD.encode("您的验证码是：482913。请勿分享。")
    );
    found(base64.as_bytes(), "482913");
    found(
        b"Subject: Sign in\r\nContent-Type: text/plain; charset=iso-8859-1\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nCaf=E9. Your verification code is 48=\r\n2913.",
        "482913",
    );
}

#[test]
fn html_only_keeps_visible_code_and_excludes_quotes_and_hidden_content() {
    found(
        &html(
            "<style>.x::before {content:'Verification code: 222222'}</style>\
             <script>Verification code: 333333</script>\
             <p>Your verification code is <b>482913</b>.</p>\
             <div hidden>Verification code: 444444</div>\
             <div aria-hidden='true'>Verification code: 555555</div>\
             <p style='DISPLAY: none !important'>Verification code: 666666</p>\
             <p style='visibility: hidden'>Verification code: 777777</p>\
             <blockquote><div class='gmail_quote'>Verification code: 888888</div></blockquote>\
             <div class='other gmail_quote'>Verification code: 999999</div>\
             <img src='https://example.test/pixel?code=111111'>",
        ),
        "482913",
    );
}

#[test]
fn html_entities_and_inline_text_are_decoded_without_losing_layout_boundaries() {
    found(
        &html("<p>Your verification&nbsp;code:</p><p><b>482</b><b>913</b></p>"),
        "482913",
    );
}

#[test]
fn hidden_or_quoted_only_html_does_not_produce_a_code() {
    assert!(matches!(
        detect_eml(&html(
            "<blockquote>Verification code: 482913</blockquote>\
             <div style='mso-hide:all'>Verification code: 762951</div>"
        )),
        Ok(Detection::NotFound)
    ));
}

#[test]
fn removing_hidden_content_never_builds_a_code_from_separate_fragments() {
    for hidden in ["<span hidden>99</span>", "<blockquote>99</blockquote>"] {
        assert_eq!(
            detect_eml(&html(&format!("<p>Verification code: 1234{hidden}56</p>"))),
            Ok(Detection::NotFound)
        );
    }
}

#[test]
fn prefers_nonempty_plain_alternative_and_falls_back_from_empty_plain() {
    let make = |plain: &str| {
        format!(
            "Subject: Sign in\r\nContent-Type: multipart/alternative; boundary=alt\r\n\r\n\
             --alt\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{plain}\r\n\
             --alt\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
             <p>Verification code: 762951</p>\r\n--alt--\r\n"
        )
    };
    found(make("Verification code: 482913").as_bytes(), "482913");
    found(make(" \r\n").as_bytes(), "762951");
}

#[test]
fn does_not_read_text_attachments_or_nested_messages() {
    let raw = "Subject: Sign in\r\nContent-Type: multipart/mixed; boundary=mix\r\n\r\n\
        --mix\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nVerification code: 482913\r\n\
        --mix\r\nContent-Type: text/plain\r\nContent-Disposition: attachment; filename=old.txt\r\n\r\nVerification code: 762951\r\n\
        --mix\r\nContent-Type: message/rfc822\r\n\r\nSubject: Verification code: 888888\r\n\r\nVerification code: 999999\r\n\
        --mix--\r\n";
    found(raw.as_bytes(), "482913");
}

#[test]
fn malformed_input_is_an_error_not_a_partial_match() {
    for raw in [
        b"".as_slice(),
        b"Verification code: 482913".as_slice(),
        b"Subject: Sign in\r\nContent-Type: multipart/mixed; boundary=absent\r\n\r\nVerification code: 482913".as_slice(),
    ] {
        assert!(matches!(detect_eml(raw), Err(DetectionError::InvalidMessage)));
    }
}

#[test]
fn rejects_oversize_raw_and_decoded_content_without_prefix_detection() {
    assert!(matches!(
        detect_eml(&vec![b'x'; MAX_EML_BYTES + 1]),
        Err(DetectionError::InputTooLarge)
    ));
    let text = format!("Verification code: 482913\n{}", "x".repeat(MAX_TEXT_BYTES));
    assert!(matches!(
        detect_eml(&plain("Sign in", &text)),
        Err(DetectionError::InputTooLarge)
    ));
    let oversized_html = format!(
        "Subject: Verification code: 482913\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>{}</p>",
        "x".repeat(MAX_TEXT_BYTES)
    );
    assert!(matches!(
        detect_eml(oversized_html.as_bytes()),
        Err(DetectionError::InputTooLarge)
    ));
}
