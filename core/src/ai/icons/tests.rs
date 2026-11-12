use super::*;
use crate::net::test_support::{MockReply, mock_http};

pub(crate) const GOOD: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M0 0h24v24H0z"/></svg>"#;

#[test]
fn a_plain_svg_is_accepted() {
    assert!(valid_svg(GOOD.as_bytes()));
    assert!(valid_svg(
        format!("<?xml version=\"1.0\"?>\n{GOOD}").as_bytes()
    ));
}

#[test]
fn svgs_that_are_not_plain_are_rejected() {
    let reject = |svg: &str, why: &str| assert!(!valid_svg(svg.as_bytes()), "{why}");
    reject("", "empty");
    reject("not xml", "not xml");
    reject("<svg><path/></svg>", "no namespace");
    reject(
        r#"<svg xmlns="http://example.com/other"><path/></svg>"#,
        "wrong namespace",
    );
    reject(
        r#"<html xmlns="http://www.w3.org/1999/xhtml"><svg xmlns="http://www.w3.org/2000/svg"/></html>"#,
        "wrong root",
    );
    reject(
        r#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#,
        "script",
    );
    reject(
        r#"<svg xmlns="http://www.w3.org/2000/svg"><g><SCRIPT/></g></svg>"#,
        "nested script",
    );
    reject(
        r#"<svg xmlns="http://www.w3.org/2000/svg"><foreignObject><div/></foreignObject></svg>"#,
        "foreignObject",
    );
    reject(
        r#"<!DOCTYPE svg [<!ENTITY a "b">]><svg xmlns="http://www.w3.org/2000/svg"/>"#,
        "dtd",
    );
    let big = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg"><!--{}--></svg>"#,
        "x".repeat(MAX_ICON_BYTES)
    );
    reject(&big, "too large");
    assert!(!valid_svg(&[0xff, 0xfe, b'<', b's']), "not UTF-8");
    // Exactly at the limit is fine.
    let pad = MAX_ICON_BYTES - GOOD.len() - "<!---->".len();
    let exact = GOOD.replace("</svg>", &format!("<!--{}--></svg>", "x".repeat(pad)));
    assert_eq!(exact.len(), MAX_ICON_BYTES);
    assert!(valid_svg(exact.as_bytes()));
}

#[test]
fn the_icon_is_returned_as_base64_svg_data() {
    let icon = AiProviderIcon::from_svg(GOOD.as_bytes());
    assert_eq!(icon.mime_type, "image/svg+xml");
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(&icon.data)
        .unwrap();
    assert_eq!(decoded, GOOD.as_bytes());
}

#[tokio::test]
async fn fetching_validates_the_provider_id_and_the_answer() {
    let route = NetRoute::default();
    let server = mock_http(|request| match request.target.as_str() {
        "/good.svg" => MockReply::json(200, GOOD),
        "/script.svg" => MockReply::json(
            200,
            r#"<svg xmlns="http://www.w3.org/2000/svg"><script/></svg>"#,
        ),
        "/big.svg" => MockReply::json(200, &"x".repeat(MAX_ICON_BYTES + 1)),
        _ => MockReply::status(404),
    })
    .await;
    let base = server.url();
    assert_eq!(fetch(&route, &base, "good").await.unwrap(), GOOD.as_bytes());
    assert_eq!(
        fetch(&route, &base, "script").await.unwrap_err(),
        ErrorCode::AiInvalidResponse
    );
    assert_eq!(
        fetch(&route, &base, "big").await.unwrap_err(),
        ErrorCode::AiInvalidResponse
    );
    assert_eq!(
        fetch(&route, &base, "missing").await.unwrap_err(),
        ErrorCode::AiRequestRejected
    );
    let before = server.count();
    for bad in ["", "A", "a/b", "../x", "a.b", "-a", "a b", &"a".repeat(65)] {
        assert_eq!(
            fetch(&route, &base, bad).await.unwrap_err(),
            ErrorCode::Validation,
            "{bad}"
        );
    }
    assert_eq!(server.count(), before, "bad ids never reach the network");
}

#[test]
fn the_disk_cache_expires_and_revalidates() {
    let directory = tempfile::tempdir().unwrap();
    assert!(read_cached(directory.path(), "good").is_none());
    write_cached(directory.path(), "good", GOOD.as_bytes());
    assert_eq!(
        read_cached(directory.path(), "good").unwrap(),
        GOOD.as_bytes()
    );
    // A bad id never touches the disk.
    write_cached(directory.path(), "../evil", GOOD.as_bytes());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);

    // Older than 7 days: a miss.
    let file = std::fs::File::options()
        .write(true)
        .open(directory.path().join("good.svg"))
        .unwrap();
    file.set_modified(std::time::SystemTime::now() - CACHE_MAX_AGE - Duration::from_secs(60))
        .unwrap();
    assert!(read_cached(directory.path(), "good").is_none());

    // A file that was tampered with is not served.
    std::fs::write(
        directory.path().join("bad.svg"),
        r#"<svg xmlns="http://www.w3.org/2000/svg"><script/></svg>"#,
    )
    .unwrap();
    assert!(read_cached(directory.path(), "bad").is_none());
}
