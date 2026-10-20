//! CSS is tokenized, then rebuilt from a deliberately small passive vocabulary.
//! Neither unknown properties nor arbitrary functions survive. Escapes/comments
//! are handled by cssparser, not substring checks against attacker-controlled CSS.
use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserInput, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser, ToCss, Token,
};

pub(super) fn declarations(value: &str) -> (String, u32) {
    let mut input = ParserInput::new(value);
    let mut parser = Parser::new(&mut input);
    let mut rules = Rules::default();
    let output = declaration_list(&mut parser, &mut rules);
    (output, rules.blocked)
}

pub(super) fn stylesheet(value: &str) -> (String, u32) {
    let mut input = ParserInput::new(value);
    let mut parser = Parser::new(&mut input);
    let mut rules = Rules::default();
    let output = sheet(&mut parser, &mut rules);
    (output, rules.blocked)
}

#[derive(Default)]
struct Rules {
    blocked: u32,
    depth: usize,
}

fn declaration_list<'i>(input: &mut Parser<'i, '_>, rules: &mut Rules) -> String {
    RuleBodyParser::new(input, rules)
        .filter_map(Result::ok)
        .collect::<Vec<_>>()
        .join(";")
}

fn sheet<'i>(input: &mut Parser<'i, '_>, rules: &mut Rules) -> String {
    StyleSheetParser::new(input, rules)
        .filter_map(Result::ok)
        .collect::<Vec<_>>()
        .join("\n")
}

impl<'i> DeclarationParser<'i> for Rules {
    type Declaration = String;
    type Error = ();

    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        _: &ParserState,
    ) -> Result<String, ParseError<'i, ()>> {
        let name = name.to_ascii_lowercase();
        let mut valid = allowed_property(&name);
        let mut value = String::new();
        let image = image_property(&name);
        tokens(
            input,
            &mut value,
            &mut valid,
            &mut self.blocked,
            image,
            false,
            0,
        )?;
        if valid && !value.trim().is_empty() {
            Ok(format!("{name}:{}", value.trim()))
        } else {
            Err(input.new_custom_error(()))
        }
    }
}

impl<'i> AtRuleParser<'i> for Rules {
    type Prelude = Option<String>;
    type AtRule = String;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<Option<String>, ParseError<'i, ()>> {
        // Imports, downloadable fonts and every other at-rule are removed. Only
        // `@media` survives, with a prelude rebuilt from passive tokens; dark
        // mode queries are dropped because the document is always light.
        if !name.eq_ignore_ascii_case("media") {
            while input.next().is_ok() {}
            return Ok(None);
        }
        let mut prelude = String::new();
        let mut valid = true;
        media_tokens(input, &mut prelude, &mut valid, 0);
        Ok(valid.then(|| prelude.trim().to_owned()))
    }

    fn rule_without_block(&mut self, _: Option<String>, _: &ParserState) -> Result<String, ()> {
        Ok(String::new())
    }

    fn parse_block<'t>(
        &mut self,
        prelude: Option<String>,
        _: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<String, ParseError<'i, ()>> {
        let (Some(prelude), true) = (prelude, self.depth < 30) else {
            while input.next().is_ok() {}
            return Ok(String::new());
        };
        self.depth += 1;
        let inner = sheet(input, self);
        self.depth -= 1;
        Ok(if inner.is_empty() {
            String::new()
        } else {
            format!("@media {prelude}{{{inner}}}")
        })
    }
}

fn media_tokens<'i>(input: &mut Parser<'i, '_>, out: &mut String, valid: &mut bool, depth: usize) {
    if depth > 8 {
        *valid = false;
        return;
    }
    while let Ok(token) = input.next_including_whitespace().cloned() {
        match token {
            Token::Ident(ref ident) => {
                if ident.to_ascii_lowercase().starts_with("prefers-") {
                    *valid = false;
                }
                out.push_str(&token.to_css_string());
            }
            Token::Number { .. }
            | Token::Percentage { .. }
            | Token::Dimension { .. }
            | Token::WhiteSpace(_)
            | Token::Comma
            | Token::Colon
            | Token::Delim('/') => out.push_str(&token.to_css_string()),
            Token::ParenthesisBlock => {
                out.push('(');
                let _ = input.parse_nested_block(|inner| {
                    media_tokens(inner, out, valid, depth + 1);
                    Ok::<(), ParseError<'i, ()>>(())
                });
                out.push(')');
            }
            _ => *valid = false,
        }
    }
}

const SELECTOR_FUNCTIONS: &[&str] = &[
    "not",
    "is",
    "where",
    "nth-child",
    "nth-last-child",
    "nth-of-type",
    "nth-last-of-type",
    "lang",
];

/// Selectors cannot load anything, so the only constraint is that the output
/// stays within tokens that cannot end the style element or the rule.
fn selector_tokens<'i>(
    input: &mut Parser<'i, '_>,
    out: &mut String,
    valid: &mut bool,
    depth: usize,
) {
    if depth > 8 {
        *valid = false;
        return;
    }
    while let Ok(token) = input.next_including_whitespace_and_comments().cloned() {
        match token {
            Token::Ident(_)
            | Token::IDHash(_)
            | Token::Hash(_)
            | Token::WhiteSpace(_)
            | Token::Comma
            | Token::Colon
            | Token::Number { .. }
            | Token::Dimension { .. }
            | Token::Percentage { .. }
            | Token::IncludeMatch
            | Token::DashMatch
            | Token::PrefixMatch
            | Token::SuffixMatch
            | Token::SubstringMatch
            | Token::Delim('.' | '*' | '>' | '+' | '~' | '|' | '^' | '$' | '=' | '-') => {
                out.push_str(&token.to_css_string());
            }
            Token::QuotedString(_) => out.push_str(&token.to_css_string().replace('<', "\\3c ")),
            Token::Comment(_) => out.push(' '),
            Token::SquareBracketBlock => {
                out.push('[');
                let _ = input.parse_nested_block(|inner| {
                    selector_tokens(inner, out, valid, depth + 1);
                    Ok::<(), ParseError<'i, ()>>(())
                });
                out.push(']');
            }
            Token::Function(ref name) => {
                let name = name.to_ascii_lowercase();
                *valid &= SELECTOR_FUNCTIONS.contains(&name.as_str());
                out.push_str(&format!("{name}("));
                let _ = input.parse_nested_block(|inner| {
                    selector_tokens(inner, out, valid, depth + 1);
                    Ok::<(), ParseError<'i, ()>>(())
                });
                out.push(')');
            }
            _ => *valid = false,
        }
    }
}

impl<'i> QualifiedRuleParser<'i> for Rules {
    type Prelude = (String, bool);
    type QualifiedRule = String;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, ParseError<'i, ()>> {
        let mut selector = String::new();
        let mut valid = true;
        selector_tokens(input, &mut selector, &mut valid, 0);
        Ok((selector, valid))
    }

    fn parse_block<'t>(
        &mut self,
        (selector, valid): Self::Prelude,
        _: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<String, ParseError<'i, ()>> {
        let body = declaration_list(input, self);
        if valid && !selector.trim().is_empty() && !body.is_empty() {
            Ok(format!("{}{{{body}}}", selector.trim()))
        } else {
            Ok(String::new())
        }
    }
}

impl<'i> RuleBodyItemParser<'i, String, ()> for Rules {
    fn parse_declarations(&self) -> bool {
        true
    }
    fn parse_qualified(&self) -> bool {
        false
    }
}

fn tokens<'i>(
    input: &mut Parser<'i, '_>,
    output: &mut String,
    valid: &mut bool,
    blocked: &mut u32,
    image: bool,
    url_string: bool,
    depth: usize,
) -> Result<(), ParseError<'i, ()>> {
    if depth > 30 {
        return Err(input.new_custom_error(()));
    }
    while !input.is_exhausted() {
        let token = input.next_including_whitespace_and_comments()?.clone();
        match token {
            Token::UnquotedUrl(ref url) => {
                if image && super::html::is_remote(url) {
                    *blocked = blocked.saturating_add(1);
                }
                *valid = false;
            }
            Token::QuotedString(ref value) => {
                if image && url_string && super::html::is_remote(value) {
                    *blocked = blocked.saturating_add(1);
                }
                // Only passive font-name-like strings are retained. In
                // particular, a quote that runs to EOF cannot turn a malformed
                // declaration tail into an accepted CSS value.
                *valid &= value.chars().all(|ch| {
                    ch.is_alphanumeric() || ch.is_whitespace() || matches!(ch, '-' | '_')
                });
                output.push_str(&token.to_css_string().replace('<', "\\3c "));
            }
            Token::Function(ref name) => {
                let name = name.to_ascii_lowercase();
                let safe = matches!(
                    name.as_str(),
                    "rgb"
                        | "rgba"
                        | "hsl"
                        | "hsla"
                        | "calc"
                        | "min"
                        | "max"
                        | "clamp"
                        | "linear-gradient"
                        | "radial-gradient"
                        | "conic-gradient"
                        | "repeating-linear-gradient"
                        | "repeating-radial-gradient"
                );
                *valid &= safe;
                output.push_str(&format!("{name}("));
                let string_is_url = matches!(
                    name.as_str(),
                    "url" | "image" | "image-set" | "-webkit-image-set"
                );
                input.parse_nested_block(|inner| {
                    tokens(
                        inner,
                        output,
                        valid,
                        blocked,
                        image,
                        string_is_url,
                        depth + 1,
                    )
                })?;
                output.push(')');
            }
            Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => {
                *valid = false;
                input.parse_nested_block(|inner| {
                    tokens(inner, output, valid, blocked, image, false, depth + 1)
                })?;
            }
            Token::Comment(_) => output.push(' '),
            Token::Ident(ref ident) => {
                // These special constructs have no place in the passive value
                // vocabulary even if a browser accepts nonstandard spellings.
                if matches!(
                    ident.to_ascii_lowercase().as_str(),
                    "expression" | "behavior" | "-moz-binding"
                ) {
                    *valid = false;
                }
                output.push_str(&token.to_css_string());
            }
            Token::Hash(_)
            | Token::IDHash(_)
            | Token::Number { .. }
            | Token::Percentage { .. }
            | Token::Dimension { .. }
            | Token::WhiteSpace(_)
            | Token::Comma
            | Token::Delim('/' | '+' | '-' | '*' | '!') => output.push_str(&token.to_css_string()),
            _ => *valid = false,
        }
    }
    Ok(())
}

fn image_property(name: &str) -> bool {
    name.starts_with("background")
        || name.starts_with("border-image")
        || name.starts_with("list-style")
        || name.starts_with("mask")
        || name.starts_with("-webkit-mask")
        || matches!(name, "content" | "cursor" | "shape-outside")
}

fn allowed_property(name: &str) -> bool {
    matches!(
        name,
        "color"
            | "background"
            | "background-color"
            | "font"
            | "font-family"
            | "font-size"
            | "font-style"
            | "font-weight"
            | "font-variant"
            | "line-height"
            | "letter-spacing"
            | "word-spacing"
            | "text-align"
            | "text-decoration"
            | "text-indent"
            | "text-transform"
            | "text-overflow"
            | "white-space"
            | "word-break"
            | "overflow-wrap"
            | "vertical-align"
            | "direction"
            | "unicode-bidi"
            | "width"
            | "height"
            | "min-width"
            | "max-width"
            | "min-height"
            | "max-height"
            | "margin"
            | "margin-top"
            | "margin-right"
            | "margin-bottom"
            | "margin-left"
            | "padding"
            | "padding-top"
            | "padding-right"
            | "padding-bottom"
            | "padding-left"
            | "border"
            | "border-top"
            | "border-right"
            | "border-bottom"
            | "border-left"
            | "border-color"
            | "border-style"
            | "border-width"
            | "border-collapse"
            | "border-spacing"
            | "border-radius"
            | "table-layout"
            | "caption-side"
            | "empty-cells"
            | "list-style-type"
            | "list-style-position"
            | "display"
            | "float"
            | "clear"
            | "box-sizing"
            | "overflow"
            | "overflow-x"
            | "overflow-y"
            | "opacity"
            | "background-image"
            | "background-repeat"
            | "background-position"
            | "background-size"
            | "background-clip"
            | "background-origin"
            | "list-style"
            | "border-top-color"
            | "border-right-color"
            | "border-bottom-color"
            | "border-left-color"
            | "border-top-style"
            | "border-right-style"
            | "border-bottom-style"
            | "border-left-style"
            | "border-top-width"
            | "border-right-width"
            | "border-bottom-width"
            | "border-left-width"
            | "border-top-left-radius"
            | "border-top-right-radius"
            | "border-bottom-left-radius"
            | "border-bottom-right-radius"
            | "outline"
            | "outline-color"
            | "outline-style"
            | "outline-width"
            | "outline-offset"
            | "box-shadow"
            | "text-shadow"
            | "text-decoration-line"
            | "text-decoration-color"
            | "text-decoration-style"
            | "text-decoration-thickness"
            | "text-underline-offset"
            | "text-size-adjust"
            | "-webkit-text-size-adjust"
            | "-ms-text-size-adjust"
            | "word-wrap"
            | "line-break"
            | "hyphens"
            | "tab-size"
            | "font-stretch"
            | "visibility"
            | "position"
            | "top"
            | "right"
            | "bottom"
            | "left"
            | "z-index"
            | "flex"
            | "flex-direction"
            | "flex-wrap"
            | "flex-flow"
            | "flex-grow"
            | "flex-shrink"
            | "flex-basis"
            | "justify-content"
            | "justify-items"
            | "justify-self"
            | "align-items"
            | "align-self"
            | "align-content"
            | "order"
            | "gap"
            | "row-gap"
            | "column-gap"
            | "object-fit"
            | "object-position"
            | "aspect-ratio"
            | "margin-inline"
            | "margin-block"
            | "padding-inline"
            | "padding-block"
    )
}
