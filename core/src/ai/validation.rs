//! Input rules shared by settings, the app service and the catalog.

use url::Url;

pub(crate) const MAX_CONNECTIONS: usize = 20;
pub(crate) const MAX_NAME_CHARS: usize = 64;
const MAX_URL_CHARS: usize = 2048;
const MAX_KEY_BYTES: usize = 4096;
const MAX_MODEL_ID_CHARS: usize = 256;
pub(crate) const MAX_MODEL_NAME_CHARS: usize = 256;
/// The id of a connection to a service that is not in the catalog.
pub(crate) const CUSTOM_PROVIDER_ID: &str = "custom";

/// `http` or `https`, a host, no account, query or fragment, at most 2048
/// characters. The trailing `/` is dropped; the path (usually a version
/// segment such as `/v1`) is kept.
pub(crate) fn normalize_base_url(input: &str) -> Option<String> {
    let input = input.trim();
    if input.is_empty() || input.chars().count() > MAX_URL_CHARS {
        return None;
    }
    let url = Url::parse(input).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none_or(str::is_empty)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let text = url.as_str().trim_end_matches('/');
    (text.chars().count() <= MAX_URL_CHARS).then(|| text.to_owned())
}

/// Lowercase letters, digits and hyphens, starting with a letter or digit.
pub(crate) fn valid_provider_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    (1..=64).contains(&bytes.len())
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

/// Trimmed, 1 to 64 characters, no control characters.
pub(crate) fn normalize_name(input: &str) -> Option<String> {
    let name = input.trim();
    let length = name.chars().count();
    ((1..=MAX_NAME_CHARS).contains(&length) && !name.chars().any(char::is_control))
        .then(|| name.to_owned())
}

/// Trimmed; empty means no key. A key goes into an HTTP header, so it must be
/// printable ASCII.
pub(crate) fn normalize_key(input: &str) -> Option<String> {
    let key = input.trim();
    (key.len() <= MAX_KEY_BYTES && key.bytes().all(|b| (0x20..0x7f).contains(&b)))
        .then(|| key.to_owned())
}

/// Model ids come from the provider (or the user) and go back into requests.
pub(crate) fn valid_model_id(id: &str) -> bool {
    let length = id.chars().count();
    (1..=MAX_MODEL_ID_CHARS).contains(&length)
        && id.trim() == id
        && !id.chars().any(char::is_control)
}

/// Display names are free text; this only bounds them.
pub(crate) fn clean_model_name(name: &str) -> Option<String> {
    let name = name.trim();
    (!name.is_empty() && !name.chars().any(char::is_control))
        .then(|| name.chars().take(MAX_MODEL_NAME_CHARS).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_urls_are_validated_and_normalized() {
        let ok = |input: &str| normalize_base_url(input);
        assert_eq!(
            ok("https://api.openai.com/v1/").as_deref(),
            Some("https://api.openai.com/v1")
        );
        assert_eq!(
            ok("  http://127.0.0.1:11434/v1  ").as_deref(),
            Some("http://127.0.0.1:11434/v1")
        );
        assert_eq!(
            ok("https://api.deepseek.com").as_deref(),
            Some("https://api.deepseek.com")
        );
        assert_eq!(
            ok("http://localhost:8080/a/b//").as_deref(),
            Some("http://localhost:8080/a/b")
        );
        assert_eq!(
            ok("http://[::1]:1234/v1").as_deref(),
            Some("http://[::1]:1234/v1")
        );
        for bad in [
            "",
            "   ",
            "api.openai.com/v1",
            "ftp://example.com/v1",
            "file:///etc/passwd",
            "https://",
            "https://user@example.com/v1",
            "https://user:secret@example.com/v1",
            "https://example.com/v1?key=1",
            "https://example.com/v1?",
            "https://example.com/v1#frag",
            "https://example.com/v1#",
            "javascript:alert(1)",
        ] {
            assert_eq!(ok(bad), None, "{bad}");
        }
        let long = format!("https://example.com/{}", "a".repeat(2048));
        assert_eq!(ok(&long), None);
        let exact = format!(
            "https://example.com/{}",
            "a".repeat(2048 - "https://example.com/".len())
        );
        assert!(ok(&exact).is_some());
    }

    #[test]
    fn provider_ids_names_keys_and_model_ids() {
        for good in ["openai", "alibaba-cn", "0x", "a", &"a".repeat(64)] {
            assert!(valid_provider_id(good), "{good}");
        }
        for bad in [
            "",
            "-a",
            "A",
            "a_b",
            "a.b",
            "a/b",
            "../x",
            "é",
            &"a".repeat(65),
        ] {
            assert!(!valid_provider_id(bad), "{bad}");
        }
        assert_eq!(normalize_name("  My AI  ").as_deref(), Some("My AI"));
        assert_eq!(normalize_name("   "), None);
        assert_eq!(normalize_name(&"x".repeat(65)), None);
        assert!(normalize_name(&"x".repeat(64)).is_some());
        assert_eq!(normalize_name("a\nb"), None);
        assert_eq!(normalize_key("  sk-abc  ").as_deref(), Some("sk-abc"));
        assert_eq!(normalize_key("   ").as_deref(), Some(""));
        assert_eq!(normalize_key("sk-\u{0}x"), None);
        assert_eq!(normalize_key("sk-a\nb"), None);
        assert_eq!(normalize_key("密钥"), None);
        assert_eq!(normalize_key(&"k".repeat(4097)), None);
        assert!(normalize_key(&"k".repeat(4096)).is_some());
        assert!(valid_model_id("gpt-4o"));
        assert!(valid_model_id("accounts/fireworks/models/llama"));
        assert!(!valid_model_id(""));
        assert!(!valid_model_id(" x"));
        assert!(!valid_model_id("a\tb"));
    }
}
