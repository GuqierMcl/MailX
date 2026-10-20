//! Plain-text bodies become escaped HTML so they share the sanitizer, link
//! handling and isolated document of HTML bodies. The text is never parsed as
//! markup: every character is escaped before any tag of ours is added.

const QUOTE_STYLE: &str =
    "margin:0 0 0 .4em;padding-left:.8em;border-left:3px solid #c8c8c8;color:#666";

pub(super) fn to_html(text: &str) -> String {
    let mut out = String::from(r#"<div style="white-space:pre-wrap">"#);
    let mut depth = 0usize;
    let mut run: Vec<&str> = Vec::new();
    let flush = |out: &mut String, run: &mut Vec<&str>| {
        if !run.is_empty() {
            out.push_str(&linkify(&run.join("\n")));
            run.clear();
        }
    };
    for line in text.lines() {
        let (quote_depth, content) = split_quote(line);
        if quote_depth != depth {
            flush(&mut out, &mut run);
            while depth < quote_depth {
                out.push_str(&format!(r#"<blockquote style="{QUOTE_STYLE}">"#));
                depth += 1;
            }
            while depth > quote_depth {
                out.push_str("</blockquote>");
                depth -= 1;
            }
        }
        run.push(content);
    }
    flush(&mut out, &mut run);
    for _ in 0..depth {
        out.push_str("</blockquote>");
    }
    out.push_str("</div>");
    out
}

/// `> > text` is depth 2. At most 8 levels are distinguished; deeper
/// markers stay in the text so a hostile message cannot nest elements.
fn split_quote(line: &str) -> (usize, &str) {
    let mut rest = line;
    let mut depth = 0;
    while depth < 8 {
        let trimmed = rest.trim_start_matches(' ');
        let Some(after) = trimmed.strip_prefix('>') else {
            break;
        };
        rest = after.strip_prefix(' ').unwrap_or(after);
        depth += 1;
    }
    (depth, rest)
}

fn escape(value: &str, out: &mut String) {
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
}

fn linkify(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < text.len() {
        let next = ["http://", "https://"]
            .iter()
            .filter_map(|scheme| lower[at..].find(scheme).map(|i| at + i))
            .min();
        let Some(start) = next else { break };
        let end = url_end(text, start);
        if end <= start + "http://".len() {
            escape(&text[at..start + 1], &mut out);
            at = start + 1;
            continue;
        }
        escape(&text[at..start], &mut out);
        out.push_str(r#"<a href=""#);
        escape(&text[start..end], &mut out);
        out.push_str(r#"">"#);
        escape(&text[start..end], &mut out);
        out.push_str("</a>");
        at = end;
    }
    escape(&text[at..], &mut out);
    out
}

/// ASCII-only so a URL ends at adjacent CJK text or punctuation; trailing
/// sentence punctuation and unbalanced closing brackets are not part of it.
fn url_end(text: &str, start: usize) -> usize {
    let mut end = start;
    for (offset, ch) in text[start..].char_indices() {
        if !ch.is_ascii_graphic()
            || matches!(ch, '<' | '>' | '"' | '`' | '\\' | '{' | '}' | '|' | '^')
        {
            break;
        }
        end = start + offset + 1;
    }
    loop {
        let url = &text[start..end];
        let Some(last) = url.chars().last() else {
            break;
        };
        let unbalanced = |open: char| url.matches(last).count() > url.matches(open).count();
        let trim = match last {
            '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '*' => true,
            ')' => unbalanced('('),
            ']' => unbalanced('['),
            _ => false,
        };
        if !trim {
            break;
        }
        end -= 1;
    }
    end
}
