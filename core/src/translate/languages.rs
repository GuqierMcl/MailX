//! Target languages offered for translation and code comparison helpers.

/// BCP 47 codes; Chinese is split by script because services differ on it.
pub const TARGETS: &[&str] = &[
    "zh-Hans", "zh-Hant", "en", "ja", "ko", "fr", "de", "es", "pt", "it", "ru", "nl", "pl", "tr",
    "ar", "id", "uk", "sv", "da", "fi", "cs", "el", "hu", "ro", "bg", "nb",
];

pub fn is_target(code: &str) -> bool {
    TARGETS.contains(&code)
}

/// Primary subtag as written (`zh-Hans` -> `zh`, `en-US` -> `en`). Callers
/// compare it case-insensitively; `same_language` does.
pub fn base(code: &str) -> &str {
    code.split(['-', '_']).next().unwrap_or("")
}

/// Same primary language: script and region are ignored, so `zh-Hans`,
/// `zh-Hant` and a detected `zh` all match; `nb` and `no` are one language.
pub fn same_language(a: &str, b: &str) -> bool {
    fn canonical(code: &str) -> String {
        let base = base(code).to_ascii_lowercase();
        if base == "no" { "nb".to_owned() } else { base }
    }
    let a = canonical(a);
    !a.is_empty() && a == canonical(b)
}

/// Default target for a UI language: Simplified Chinese for `zh-CN`, else English.
pub fn for_ui_language(ui: &str) -> &'static str {
    if ui.eq_ignore_ascii_case("zh-CN") {
        "zh-Hans"
    } else {
        "en"
    }
}
