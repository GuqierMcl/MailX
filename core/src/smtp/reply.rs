//! SMTP reply parsing (RFC 5321 §4.2) and sanitizing of server text.

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt};

use crate::secrets::SecretValue;

const MAX_LINE_BYTES: u64 = 2048;
const MAX_LINES: usize = 100;
pub(crate) const MAX_TEXT_CHARS: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IoFail {
    Eof,
    Io,
    Malformed,
    Timeout,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Reply {
    pub code: u16,
    /// Text of each reply line, without the code and separator.
    pub lines: Vec<String>,
}

impl Reply {
    pub(crate) fn text(&self) -> String {
        self.lines.join(" ")
    }
}

/// Reads one possibly multiline reply. Never reads past its final line.
pub(crate) async fn read_reply<R: AsyncBufRead + Unpin>(reader: &mut R) -> Result<Reply, IoFail> {
    let mut code = None;
    let mut lines = Vec::new();
    loop {
        let mut raw = Vec::new();
        let read = (&mut *reader)
            .take(MAX_LINE_BYTES)
            .read_until(b'\n', &mut raw)
            .await
            .map_err(|_| IoFail::Io)?;
        if read == 0 {
            return Err(IoFail::Eof);
        }
        if !raw.ends_with(b"\n") {
            // Either too long or cut off by the end of the stream.
            return Err(if raw.len() as u64 >= MAX_LINE_BYTES {
                IoFail::Malformed
            } else {
                IoFail::Eof
            });
        }
        let line = raw.strip_suffix(b"\n").unwrap_or(&raw);
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.len() < 3 || !line[..3].iter().all(u8::is_ascii_digit) {
            return Err(IoFail::Malformed);
        }
        let line_code = u16::from(line[0] - b'0') * 100
            + u16::from(line[1] - b'0') * 10
            + u16::from(line[2] - b'0');
        if !(200..600).contains(&line_code) || code.is_some_and(|code| code != line_code) {
            return Err(IoFail::Malformed);
        }
        code = Some(line_code);
        let (more, text) = match line.get(3) {
            None => (false, &line[3..]),
            Some(b' ') => (false, &line[4..]),
            Some(b'-') => (true, &line[4..]),
            Some(_) => return Err(IoFail::Malformed),
        };
        lines.push(String::from_utf8_lossy(text).into_owned());
        if lines.len() > MAX_LINES {
            return Err(IoFail::Malformed);
        }
        if !more {
            return Ok(Reply {
                code: line_code,
                lines,
            });
        }
    }
}

/// Server text for storage and logs: control characters (CR and LF included)
/// become spaces, whitespace is collapsed, the account password is masked if
/// the server echoes it, and the result has at most 200 characters.
pub(crate) fn sanitize(text: &str, password: &SecretValue) -> String {
    let secret = password.expose_secret();
    let masked;
    let text = if secret.is_empty() {
        text
    } else {
        masked = text.replace(secret, "[redacted]");
        &masked
    };
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let collapsed = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(MAX_TEXT_CHARS).collect()
}
