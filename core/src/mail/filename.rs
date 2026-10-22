//! File names for saved attachments. The sender chooses the name, so it is
//! treated as hostile: it must never leave the folder the user picked, hide its
//! real extension behind a text direction trick, or name a Windows device.

/// UTF-8 bytes. Common file systems allow 255; the rest is room for a suffix
/// such as " (1)" that a host may add to avoid a clash.
const MAX_NAME_BYTES: usize = 200;
const MAX_EXTENSION_BYTES: usize = 16;
const FALLBACK: &str = "attachment";

fn is_unsafe(ch: char) -> bool {
    ch.is_control()
        || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
        // Text direction controls and invisible characters that can disguise
        // `photo\u{202e}fdp.exe` as `photoexe.pdf`.
        || matches!(
            ch,
            '\u{061c}'
                | '\u{200b}'..='\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{2069}'
                | '\u{feff}'
        )
}

fn is_reserved_device(stem: &str) -> bool {
    let stem = stem.trim_end_matches([' ', '.']).to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|prefix| {
        stem.strip_prefix(prefix).is_some_and(|rest| {
            // The superscript digits are device names to Windows too.
            matches!(
                rest,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    })
}

fn guessed_extension(mime_type: &str) -> Option<&'static str> {
    let essence = mime_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    Some(match essence.as_str() {
        "application/pdf" => "pdf",
        "application/zip" => "zip",
        "application/json" => "json",
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "text/plain" => "txt",
        "text/html" => "html",
        "text/csv" => "csv",
        "message/rfc822" => "eml",
        _ => return None,
    })
}

fn truncate(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

/// A single path component that is safe to create on Windows, macOS and Linux.
/// `mime_type` only supplies an extension when the sender gave no usable name.
pub fn safe_file_name(name: Option<&str>, mime_type: &str) -> String {
    // Only the last component counts, whatever separator the sender used.
    let last = name.unwrap_or("").rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = last
        .chars()
        .map(|ch| if is_unsafe(ch) { '_' } else { ch })
        .collect();
    // No hidden files, no trailing dots or spaces (Windows drops them silently).
    let cleaned = cleaned.trim_matches([' ', '.']);
    let meaningful = cleaned.chars().any(|ch| !matches!(ch, '_' | ' ' | '.'));
    if cleaned.is_empty() || !meaningful {
        return match guessed_extension(mime_type) {
            Some(extension) => format!("{FALLBACK}.{extension}"),
            None => FALLBACK.to_owned(),
        };
    }
    let (stem, extension) = match cleaned.rfind('.') {
        Some(dot)
            if dot > 0
                && cleaned.len() - dot - 1 > 0
                && cleaned.len() - dot - 1 <= MAX_EXTENSION_BYTES
                && !cleaned[dot + 1..].contains(' ') =>
        {
            (&cleaned[..dot], Some(&cleaned[dot + 1..]))
        }
        _ => (cleaned, None),
    };
    // Device names are reserved with or without an extension (`NUL.txt`).
    let first_label = stem.split('.').next().unwrap_or(stem);
    let prefix = if is_reserved_device(first_label) {
        "_"
    } else {
        ""
    };
    let budget = MAX_NAME_BYTES
        .saturating_sub(prefix.len())
        .saturating_sub(extension.map_or(0, |e| e.len() + 1));
    let stem = truncate(stem, budget).trim_end_matches([' ', '.']);
    let stem = if stem.is_empty() { FALLBACK } else { stem };
    match extension {
        Some(extension) => format!("{prefix}{stem}.{extension}"),
        None => format!("{prefix}{stem}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(value: &str) -> String {
        safe_file_name(Some(value), "application/octet-stream")
    }

    #[test]
    fn ordinary_names_pass_through_unchanged() {
        for plain in [
            "report.pdf",
            "报告 2026 年度.pdf",
            "a.tar.gz",
            "Makefile",
            "x (1).docx",
        ] {
            assert_eq!(name(plain), plain);
        }
    }

    #[test]
    fn only_the_last_path_component_survives_whatever_the_separator() {
        assert_eq!(name("../../etc/passwd"), "passwd");
        assert_eq!(name("..\\..\\Windows\\win.ini"), "win.ini");
        assert_eq!(name("C:\\Users\\a\\b.txt"), "b.txt");
        assert_eq!(name("/abs/path/file.pdf"), "file.pdf");
        assert_eq!(name("dir/"), "attachment");
        assert_eq!(name(".."), "attachment");
        assert_eq!(name("../"), "attachment");
    }

    #[test]
    fn unsafe_characters_become_underscores_and_hidden_or_trailing_dots_go() {
        assert_eq!(name("a<b>c:d\"e|f?g*h.txt"), "a_b_c_d_e_f_g_h.txt");
        assert_eq!(name("line\nbreak\u{0}.txt"), "line_break_.txt");
        assert_eq!(name(".bashrc"), "bashrc");
        assert_eq!(name("report.pdf. . "), "report.pdf");
        assert_eq!(name("   spaced   "), "spaced");
    }

    #[test]
    fn text_direction_tricks_cannot_disguise_the_extension() {
        let tricked = name("invoice\u{202e}fdp.exe");
        assert!(!tricked.contains('\u{202e}'), "{tricked}");
        assert!(tricked.ends_with(".exe"), "{tricked}");
        for control in ['\u{200b}', '\u{200e}', '\u{2066}', '\u{feff}', '\u{061c}'] {
            assert_eq!(name(&format!("a{control}b.txt")), "a_b.txt");
        }
    }

    #[test]
    fn windows_device_names_are_defused_with_or_without_an_extension() {
        for device in [
            "CON",
            "con.txt",
            "NUL",
            "Aux.tar.gz",
            "COM1",
            "lpt9.log",
            "COM¹.txt",
            "CONIN$",
            "prn . txt",
        ] {
            assert!(
                name(device).starts_with('_'),
                "{device} -> {}",
                name(device)
            );
        }
        for fine in ["COM10.txt", "console.txt", "NULL", "CONFIG.sys", "com.txt"] {
            assert_eq!(name(fine), fine);
        }
    }

    #[test]
    fn long_names_are_cut_on_a_character_boundary_and_keep_their_extension() {
        let long = format!("{}.pdf", "报".repeat(500));
        let cut = name(&long);
        assert!(cut.len() <= MAX_NAME_BYTES, "{}", cut.len());
        assert!(cut.ends_with(".pdf"));
        assert!(cut.starts_with('报'));
        let odd_extension = format!("{}.{}", "a".repeat(300), "x".repeat(40));
        assert!(name(&odd_extension).len() <= MAX_NAME_BYTES);
    }

    #[test]
    fn a_missing_or_empty_name_falls_back_with_an_extension_guessed_from_the_type() {
        assert_eq!(safe_file_name(None, "application/pdf"), "attachment.pdf");
        assert_eq!(
            safe_file_name(Some(""), "image/jpeg; name=x"),
            "attachment.jpg"
        );
        assert_eq!(
            safe_file_name(Some("___"), "message/rfc822"),
            "attachment.eml"
        );
        assert_eq!(
            safe_file_name(Some("..."), "application/x-unknown"),
            "attachment"
        );
        assert_eq!(safe_file_name(None, ""), "attachment");
    }

    #[test]
    fn every_result_is_a_single_nonempty_component() {
        for hostile in [
            "",
            ".",
            "..",
            "////",
            "\\\\",
            "a/b\\c",
            "\u{202e}",
            "NUL",
            "x".repeat(1000).as_str(),
            "..\u{0}/..",
        ] {
            let result = name(hostile);
            assert!(!result.is_empty());
            assert!(
                !result.contains(['/', '\\', ':', '\u{0}']),
                "{hostile:?} -> {result:?}"
            );
            assert_ne!(result, ".");
            assert_ne!(result, "..");
            assert!(result.len() <= MAX_NAME_BYTES);
        }
    }
}
