//! IMAP's modified UTF-7 for mailbox names (RFC 3501 §5.1.3).
//!
//! Printable ASCII stands for itself, `&-` is a literal `&`, and other text is
//! UTF-16BE in base64 using `,` instead of `/`, between `&` and `-`.

/// Decodes a mailbox name; `None` when the name is not valid modified UTF-7,
/// so callers can fall back to the raw name instead of showing garbage.
pub(crate) fn decode(name: &str) -> Option<String> {
    let mut output = String::with_capacity(name.len());
    let mut rest = name;
    while let Some(start) = rest.find('&') {
        output.push_str(&rest[..start]);
        let shifted = &rest[start + 1..];
        let end = shifted.find('-')?;
        if end == 0 {
            output.push('&');
        } else {
            output.push_str(&decode_shifted(&shifted[..end])?);
        }
        rest = &shifted[end + 1..];
    }
    output.push_str(rest);
    Some(output)
}

fn decode_shifted(encoded: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(encoded.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0;
    for character in encoded.bytes() {
        buffer = (buffer << 6) | u32::from(sextet(character)?);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push((buffer >> bits) as u8);
        }
    }
    // Leftover bits are padding and must be zero.
    if buffer & ((1 << bits) - 1) != 0 || bytes.len() % 2 != 0 {
        return None;
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
        .collect();
    String::from_utf16(&units).ok()
}

fn sextet(character: u8) -> Option<u8> {
    match character {
        b'A'..=b'Z' => Some(character - b'A'),
        b'a'..=b'z' => Some(character - b'a' + 26),
        b'0'..=b'9' => Some(character - b'0' + 52),
        b'+' => Some(62),
        b',' => Some(63),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn decodes_names_used_by_real_servers() {
        assert_eq!(decode("INBOX").as_deref(), Some("INBOX"));
        assert_eq!(decode("&XfJT0ZAB-").as_deref(), Some("已发送"));
        assert_eq!(decode("&g0l6P3ux-").as_deref(), Some("草稿箱"));
        assert_eq!(decode("&XfJSIJZk-").as_deref(), Some("已删除"));
        assert_eq!(decode("&V4NXPpCuTvY-").as_deref(), Some("垃圾邮件"));
        assert_eq!(
            decode("[Gmail]/&V4NXPpCuTvY-").as_deref(),
            Some("[Gmail]/垃圾邮件")
        );
        assert_eq!(
            decode("Projects/&ZeVnLIqe-").as_deref(),
            Some("Projects/日本語")
        );
    }

    #[test]
    fn handles_literal_ampersands_and_surrogate_pairs() {
        assert_eq!(decode("Tom &- Jerry").as_deref(), Some("Tom & Jerry"));
        assert_eq!(decode("&2D3eAA-").as_deref(), Some("😀"));
    }

    #[test]
    fn rejects_malformed_names() {
        assert_eq!(decode("&XfJT0ZAB"), None, "unterminated shift");
        assert_eq!(decode("&Xf!-"), None, "character outside the alphabet");
        assert_eq!(decode("&2D0-"), None, "unpaired surrogate");
        assert_eq!(decode("&AB-"), None, "odd byte count");
    }
}
