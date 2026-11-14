//! A Server-Sent Events parser fed with arbitrary byte chunks. Chunk
//! boundaries may fall anywhere, including inside a UTF-8 sequence or a CRLF
//! pair: bytes are only decoded once a whole line is in.

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SseEvent {
    /// The `event:` field, if any.
    pub event: Option<String>,
    /// The `data:` lines joined with `\n`.
    pub data: String,
}

#[derive(Default)]
pub(super) struct SseParser {
    buffer: Vec<u8>,
    event: Option<String>,
    data: String,
    has_data: bool,
    started: bool,
}

impl SseParser {
    /// Feeds bytes; completed events are appended to `out`.
    pub(super) fn push(&mut self, chunk: &[u8], out: &mut Vec<SseEvent>) {
        self.buffer.extend_from_slice(chunk);
        let mut start = 0;
        loop {
            let rest = &self.buffer[start..];
            let Some(end) = rest.iter().position(|b| matches!(b, b'\n' | b'\r')) else {
                break;
            };
            let mut next = end + 1;
            if rest[end] == b'\r' {
                match rest.get(end + 1) {
                    // A lone CR at the end of the chunk may be half a CRLF.
                    None => break,
                    Some(b'\n') => next += 1,
                    Some(_) => {}
                }
            }
            let line = rest[..end].to_vec();
            start += next;
            self.line(&line, out);
        }
        self.buffer.drain(..start);
    }

    /// Ends the stream. A last line or event without its closing newline is
    /// processed instead of dropped (providers do end this way).
    pub(super) fn finish(&mut self, out: &mut Vec<SseEvent>) {
        let rest = std::mem::take(&mut self.buffer);
        let rest = rest.strip_suffix(b"\r").unwrap_or(&rest);
        if !rest.is_empty() {
            self.line(rest, out);
        }
        self.dispatch(out);
    }

    fn line(&mut self, line: &[u8], out: &mut Vec<SseEvent>) {
        let mut line = String::from_utf8_lossy(line);
        if !self.started {
            self.started = true;
            if let Some(stripped) = line.strip_prefix('\u{feff}') {
                line = stripped.to_owned().into();
            }
        }
        if line.is_empty() {
            self.dispatch(out);
            return;
        }
        if line.starts_with(':') {
            return;
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (&*line, ""),
        };
        match field {
            "event" => self.event = Some(value.to_owned()),
            "data" => {
                if self.has_data {
                    self.data.push('\n');
                }
                self.data.push_str(value);
                self.has_data = true;
            }
            // `id`, `retry` and unknown fields are of no use here.
            _ => {}
        }
    }

    fn dispatch(&mut self, out: &mut Vec<SseEvent>) {
        let event = self.event.take();
        if self.has_data {
            out.push(SseEvent {
                event,
                data: std::mem::take(&mut self.data),
            });
            self.has_data = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(chunks: &[&[u8]]) -> Vec<SseEvent> {
        let mut parser = SseParser::default();
        let mut out = Vec::new();
        for chunk in chunks {
            parser.push(chunk, &mut out);
        }
        parser.finish(&mut out);
        out
    }

    fn ev(event: Option<&str>, data: &str) -> SseEvent {
        SseEvent {
            event: event.map(str::to_owned),
            data: data.to_owned(),
        }
    }

    #[test]
    fn data_lines_dispatch_on_a_blank_line() {
        let out = parse(&[b"data: one\n\ndata:two\n\n"]);
        assert_eq!(out, [ev(None, "one"), ev(None, "two")]);
    }

    #[test]
    fn event_names_and_multi_line_data() {
        let out = parse(&[b"event: delta\ndata: a\ndata: b\ndata\n\n"]);
        assert_eq!(out, [ev(Some("delta"), "a\nb\n")]);
    }

    #[test]
    fn crlf_cr_and_lf_all_end_lines() {
        let out = parse(&[b"data: a\r\n\r\ndata: b\r\rdata: c\n\n"]);
        assert_eq!(out, [ev(None, "a"), ev(None, "b"), ev(None, "c")]);
    }

    #[test]
    fn a_crlf_split_between_chunks_is_one_terminator() {
        let out = parse(&[b"data: a\r", b"\n\r", b"\ndata: b\n\n"]);
        assert_eq!(out, [ev(None, "a"), ev(None, "b")]);
    }

    #[test]
    fn comments_and_unknown_fields_are_ignored() {
        let out = parse(&[b": keep-alive\nid: 7\nretry: 100\nfoo: bar\ndata: x\n\n"]);
        assert_eq!(out, [ev(None, "x")]);
    }

    #[test]
    fn an_event_without_data_is_not_dispatched() {
        let out = parse(&[b"event: ping\n\ndata: x\n\n"]);
        assert_eq!(out, [ev(None, "x")]);
    }

    #[test]
    fn only_one_leading_space_is_removed() {
        let out = parse(&[b"data:  two\n\n"]);
        assert_eq!(out, [ev(None, " two")]);
    }

    #[test]
    fn any_chunking_gives_the_same_events_including_inside_utf8() {
        let text = "data: 你好, \u{1F600}\nevent: e\ndata: x\r\n\r\n: c\ndata: y\n\n";
        let whole = parse(&[text.as_bytes()]);
        assert_eq!(whole.len(), 2);
        assert_eq!(whole[0], ev(Some("e"), "你好, \u{1F600}\nx"));
        for size in 1..=7 {
            let chunks: Vec<&[u8]> = text.as_bytes().chunks(size).collect();
            assert_eq!(parse(&chunks), whole, "chunk size {size}");
        }
    }

    #[test]
    fn a_bom_at_the_start_is_skipped() {
        let out = parse(&[b"\xEF\xBB\xBFdata: x\n\n"]);
        assert_eq!(out, [ev(None, "x")]);
    }

    #[test]
    fn a_final_event_without_its_blank_line_is_kept() {
        let out = parse(&[b"data: a\n\ndata: b"]);
        assert_eq!(out, [ev(None, "a"), ev(None, "b")]);
    }
}
