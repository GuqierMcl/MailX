//! Random secrets for the handshake token and per-start identifiers.

use std::fmt::Write as _;

/// 32 random bytes from the operating system CSPRNG, hex-encoded (64 chars).
pub(crate) fn random_token() -> Result<String, getrandom::Error> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)?;
    let mut out = String::with_capacity(64);
    for byte in bytes {
        // Writing to a String cannot fail.
        let _ = write!(out, "{byte:02x}");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::random_token;

    #[test]
    fn tokens_are_64_hex_chars_and_differ() {
        let a = random_token().unwrap();
        let b = random_token().unwrap();
        assert_eq!(a.len(), 64);
        assert!(a.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }
}
