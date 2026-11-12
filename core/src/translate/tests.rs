use std::collections::BTreeMap;

use scraper::{Html, Node};

use super::*;

// ------------------------------------------------------------------ helpers

fn texts(html: &str) -> Vec<String> {
    extract(html, true)
        .unwrap()
        .iter()
        .map(Segment::plain_text)
        .collect()
}

fn wires(html: &str, include_quotes: bool) -> Vec<Option<String>> {
    extract(html, include_quotes)
        .unwrap()
        .iter()
        .map(|segment| Some(wire::encode(segment)))
        .collect()
}

fn text(value: &str) -> Piece {
    Piece::Text(value.to_owned())
}

fn pieces(html: &str) -> Vec<Vec<Piece>> {
    extract(html, true)
        .unwrap()
        .iter()
        .map(|segment| segment.pieces().to_vec())
        .collect()
}

/// Multiset of elements, each as its name plus sorted attributes.
fn shape(html: &str) -> BTreeMap<String, usize> {
    let document = Html::parse_fragment(html);
    let mut out = BTreeMap::new();
    for node in document.root_element().descendants() {
        if node.id() == document.root_element().id() {
            continue;
        }
        if let Node::Element(element) = node.value() {
            let mut attrs: Vec<String> = element
                .attrs()
                .map(|(name, value)| format!("{name}={value}"))
                .collect();
            attrs.sort();
            *out.entry(format!("{}[{}]", element.name(), attrs.join(",")))
                .or_insert(0) += 1;
        }
    }
    out
}

/// The security invariant: nothing in the output that the input did not have.
fn assert_subset(output: &str, input: &str) {
    let have = shape(input);
    for (element, count) in shape(output) {
        let allowed = have.get(&element).copied().unwrap_or(0);
        assert!(
            count <= allowed,
            "{element} x{count} not in the input (x{allowed}):\n{output}"
        );
    }
}

fn visible(html: &str) -> String {
    let document = Html::parse_fragment(html);
    document
        .root_element()
        .descendants()
        .filter_map(|node| node.value().as_text())
        .flat_map(|text| text.text.chars())
        .filter(|ch| !ch.is_whitespace())
        .collect()
}

/// What the parser and serializer make of the markup (attribute order, quotes).
fn norm(html: &str) -> String {
    Html::parse_fragment(html).root_element().inner_html()
}

fn apply_all(html: &str, translations: &[&str]) -> String {
    let list: Vec<Option<String>> = translations.iter().map(|t| Some((*t).to_owned())).collect();
    apply(html, true, &list).expect("segment count matches")
}

// ------------------------------------------------------------ segmentation

#[test]
fn simple_paragraphs_and_blocks() {
    assert_eq!(
        texts("<div>Hello there</div><p>Second paragraph</p>"),
        ["Hello there", "Second paragraph"]
    );
    assert_eq!(texts("Bare text at the root"), ["Bare text at the root"]);
    assert!(texts("").is_empty());
    assert!(texts("   ").is_empty());
}

#[test]
fn outlook_wrapper_spans_are_peeled() {
    let html = r#"<p class="MsoNormal"><span style="font-size:11.0pt"><font face="Calibri">Hello <b>Mr. Smith</b>, please review the attached document.</font></span></p><p class="MsoNormal"><span style="font-size:11.0pt">&nbsp;</span></p>"#;
    assert_eq!(
        pieces(html),
        [vec![
            text("Hello "),
            Piece::Open(1),
            text("Mr. Smith"),
            Piece::Close(1),
            text(", please review the attached document."),
        ]]
    );
}

#[test]
fn peeling_stops_at_a_real_run() {
    // Two inline elements: not a single wrapper, so both stay in the segment.
    let html = "<p><b>Bold</b> and <i>italic</i></p>";
    assert_eq!(
        pieces(html),
        [vec![
            Piece::Open(1),
            text("Bold"),
            Piece::Close(1),
            text(" and "),
            Piece::Open(2),
            text("italic"),
            Piece::Close(2),
        ]]
    );
}

#[test]
fn gmail_double_break_is_a_paragraph_break() {
    let html = r#"<div dir="ltr">First paragraph here.<br><br>Second paragraph here.<br>Same paragraph, new line.<div><br></div></div>"#;
    assert_eq!(
        pieces(html),
        [
            vec![text("First paragraph here.")],
            vec![
                text("Second paragraph here."),
                Piece::Break(1),
                text("Same paragraph, new line."),
            ]
        ]
    );
}

#[test]
fn breaks_with_whitespace_between_still_split() {
    let html = "<div>Alpha text<br> \n <br>\n<br>Beta text</div>";
    assert_eq!(texts(html), ["Alpha text", "Beta text"]);
}

#[test]
fn leading_and_trailing_breaks_are_trimmed_and_kept() {
    let html = "<div><br>Hello world<br></div>";
    assert_eq!(pieces(html), [vec![text("Hello world")]]);
    let out = apply_all(html, &["你好世界"]);
    assert_eq!(out, "<div><br>你好世界<br></div>");
}

#[test]
fn inline_with_paragraph_break_is_entered() {
    let html = "<div><span>One sentence.<br><br>Another sentence.</span></div>";
    assert_eq!(texts(html), ["One sentence.", "Another sentence."]);
}

#[test]
fn inline_containing_block_is_entered() {
    let html = "<span>Before <div>Inside block</div> after</span>";
    assert_eq!(texts(html), ["Before", "Inside block", "after"]);
}

#[test]
fn nested_layout_tables() {
    let html = r#"<table><tbody><tr><td><table><tbody><tr><td>Order number</td><td><b>Shipped soon</b></td></tr></tbody></table></td></tr><tr><td colspan="2">Please confirm your <a href="https://e.test/x" data-mailx-link="1">address</a>.</td></tr></tbody></table>"#;
    assert_eq!(
        texts(html),
        [
            "Order number",
            "Shipped soon",
            "Please confirm your address."
        ]
    );
}

#[test]
fn lists_segment_per_item() {
    let html = "<ul><li>One item</li><li>Two <i>items</i> here</li></ul><ol><li><p>Nested paragraph</p></li></ol>";
    assert_eq!(
        texts(html),
        ["One item", "Two items here", "Nested paragraph"]
    );
}

#[test]
fn skipped_elements_are_never_segmented() {
    let html = concat!(
        "<style>p { color: red }</style>",
        "<script>var message = 'hello world';</script>",
        "<div hidden>Hidden text</div>",
        "<div style=\"display: NONE\">Invisible text</div>",
        "<div style=\"color:red;display:none !important\">Invisible too</div>",
        "<svg><text>Vector label</text></svg>",
        "<select><option>Choose one</option></select>",
        "<textarea>Typed words</textarea>",
        "<div>Visible words</div>",
    );
    assert_eq!(texts(html), ["Visible words"]);
    let out = apply_all(html, &["可见文字"]);
    assert!(out.contains("p { color: red }"));
    assert!(out.contains("Hidden text"));
    assert!(out.contains("Vector label"));
    assert!(out.contains("可见文字"));
}

#[test]
fn skipped_element_splits_a_sentence() {
    // Judgment call: a skipped subtree is a boundary, never part of a run.
    let html = "<p>Before <span hidden>secret</span> after</p>";
    assert_eq!(texts(html), ["Before", "after"]);
}

#[test]
fn objects_are_opaque() {
    let html = r#"<p>Call <code>ls -la now</code> to list, then <kbd>Enter</kbd> or <img data-mailx-cid="1"> done.</p>"#;
    assert_eq!(
        pieces(html),
        [vec![
            text("Call "),
            Piece::Object(1),
            text(" to list, then "),
            Piece::Object(2),
            text(" or "),
            Piece::Object(3),
            text(" done."),
        ]]
    );
}

#[test]
fn inline_without_text_is_an_object() {
    let html = r#"<p>Look at <a href="https://e.test" data-mailx-link="1"><img data-mailx-cid="1"></a> this logo.</p>"#;
    assert_eq!(
        pieces(html),
        [vec![
            text("Look at "),
            Piece::Object(1),
            text(" this logo.")
        ]]
    );
}

#[test]
fn preformatted_code_stays_untranslated() {
    assert!(texts("<pre><code>let answer = 42;\nprintln!(answer);</code></pre>").is_empty());
}

#[test]
fn filters_drop_candidates_without_words() {
    assert!(texts("<p>1234 - 5678</p>").is_empty());
    assert!(texts("<p>&nbsp;</p><p>---</p>").is_empty());
    assert!(texts("<p>https://example.com/path?x=1</p>").is_empty());
    assert!(texts("<p>  HTTP://EXAMPLE.COM  </p>").is_empty());
    assert!(texts("<p>www.example.com</p>").is_empty());
    assert!(texts(r#"<p><a href="x">alice@example.com</a></p>"#).is_empty());
    assert_eq!(texts("<p>Write to alice@example.com today</p>").len(), 1);
    assert_eq!(texts("<p>Visit https://example.com today</p>").len(), 1);
}

#[test]
fn candidates_with_too_many_placeholders_are_dropped() {
    let many: String = (0..65).map(|i| format!("<b>w{i}</b> ")).collect();
    assert!(texts(&format!("<p>{many}</p>")).is_empty());
    let ok: String = (0..64).map(|i| format!("<b>w{i}</b> ")).collect();
    assert_eq!(texts(&format!("<p>{ok}</p>")).len(), 1);
}

#[test]
fn whitespace_is_collapsed_and_edges_restored() {
    let html = "<div>  Hello \n\t world  <b> there </b>  friend  </div>";
    let segments = extract(html, true).unwrap();
    assert_eq!(
        segments[0].pieces(),
        [
            text("Hello world "),
            Piece::Open(1),
            text(" there "),
            Piece::Close(1),
            text(" friend"),
        ]
    );
    let out = apply_all(html, &["你好 <span id=\"1\">朋友</span> 们"]);
    assert_eq!(out, "<div> 你好 <b>朋友</b> 们 </div>");
}

#[test]
fn placeholders_number_all_kinds_in_document_order() {
    let html = "<p>One <b>two <i>three</i></b><br>four <img> five</p>";
    assert_eq!(
        pieces(html),
        [vec![
            text("One "),
            Piece::Open(1),
            text("two "),
            Piece::Open(2),
            text("three"),
            Piece::Close(2),
            Piece::Close(1),
            Piece::Break(3),
            text("four "),
            Piece::Object(4),
            text(" five"),
        ]]
    );
}

#[test]
fn segment_text_helpers() {
    let segments = extract("<p>Line one<br>Line <b>two</b> é</p>", true).unwrap();
    assert_eq!(segments[0].plain_text(), "Line one\nLine two é");
    assert_eq!(segments[0].char_count(), 19);
}

#[test]
fn sample_text_is_capped_and_ordered() {
    let segments = extract("<p>First one</p><p>Second two</p><p>Third three</p>", true).unwrap();
    assert_eq!(
        sample_text(&segments, 1000),
        "First one\nSecond two\nThird three"
    );
    assert_eq!(sample_text(&segments, 14), "First one\nSeco");
    assert_eq!(sample_text(&segments, 0), "");
}

#[test]
fn deep_nesting_does_not_overflow() {
    let depth = 20_000;
    let html = format!(
        "{}deep words here{}",
        "<span>".repeat(depth),
        "</span>".repeat(depth)
    );
    let segments = extract(&html, true).unwrap();
    assert_eq!(segments.len(), 1);
    let out = apply(&html, true, &[Some("深层文字".to_owned())]).unwrap();
    assert_eq!(visible(&out), "深层文字");
    // The HTML parser itself is quadratic on nested blocks, so keep this modest.
    let divs = format!(
        "{}deep words{}",
        "<div>".repeat(2000),
        "</div>".repeat(2000)
    );
    assert_eq!(extract(&divs, true).unwrap().len(), 1);
    assert!(apply(&divs, true, &wires(&divs, true)).is_some());
}

// --------------------------------------------------------------- quotes

const FOLDED: &str = r#"<div>Thanks, see you.</div><details class="mailx-quote"><summary>Show quoted text</summary><div>On Monday Bob wrote:</div><blockquote>Can we meet <a href="https://e.test" data-mailx-link="1">tomorrow</a>?</blockquote></details>"#;

#[test]
fn folded_quote_is_skipped_or_included() {
    let without = extract(FOLDED, false).unwrap();
    assert_eq!(
        without.iter().map(Segment::plain_text).collect::<Vec<_>>(),
        ["Thanks, see you."]
    );
    let with = extract(FOLDED, true).unwrap();
    assert_eq!(
        with.iter().map(Segment::plain_text).collect::<Vec<_>>(),
        [
            "Thanks, see you.",
            "On Monday Bob wrote:",
            "Can we meet tomorrow?"
        ]
    );
}

#[test]
fn summary_is_never_translated_and_never_affects_the_walk() {
    for label in ["Show quoted text", "显示引用内容", ""] {
        let html = FOLDED.replace("Show quoted text", label);
        let segments = extract(&html, true).unwrap();
        assert_eq!(segments.len(), 3, "{label}");
        let out = apply_all(&html, &["一", "二", "三"]);
        assert!(
            out.contains(&format!("<summary>{label}</summary>")),
            "{out}"
        );
    }
    // The cached translation of one label applies to HTML folded with another.
    let other = FOLDED.replace("Show quoted text", "Quoted text");
    let out = apply(&other, true, &wires(FOLDED, true)).unwrap();
    assert!(out.contains("<summary>Quoted text</summary>"));
}

#[test]
fn quote_excluded_apply_leaves_the_quote_alone() {
    let out = apply(FOLDED, false, &[Some("谢谢，再见。".to_owned())]).unwrap();
    assert!(out.starts_with("<div>谢谢，再见。</div>"));
    assert!(out.contains("On Monday Bob wrote:"));
    assert!(out.contains("Can we meet "));
    // A mismatched mode is a stale count.
    assert_eq!(apply(FOLDED, true, &[Some("x".to_owned())]), None);
}

#[test]
fn a_summary_outside_a_quote_is_ordinary_text() {
    let html = "<details><summary>Click for more</summary>Hidden content</details>";
    assert_eq!(texts(html), ["Click for more", "Hidden content"]);
}

#[test]
fn works_on_real_fold_quotes_output() {
    let html = r#"<div>Thanks, see you.</div><br><div class="gmail_quote"><div class="gmail_attr">On 2026-11-03, Bob &lt;bob@example.com&gt; wrote:<br></div><blockquote class="gmail_quote" style="margin:0px 0px 0px 0.8ex">Hi there, original message</blockquote></div>"#;
    let folded = crate::mail::render::fold_quotes(html, "LABEL");
    assert!(folded.contains("mailx-quote"));
    let other = crate::mail::render::fold_quotes(html, "標籤");
    assert_eq!(
        extract(&folded, false).unwrap(),
        extract(&other, false).unwrap()
    );
    assert_eq!(
        extract(&folded, true).unwrap(),
        extract(&other, true).unwrap()
    );
    assert_eq!(extract(&folded, false).unwrap().len(), 1);
    let out = apply(&folded, false, &[Some("谢谢，再见。".to_owned())]).unwrap();
    assert!(out.contains("谢谢，再见。"));
    assert!(out.contains("<summary>LABEL</summary>"));
}

// ------------------------------------------------------ pre-formatted text

#[test]
fn hard_newlines_become_breaks_in_preformatted_text() {
    let html = "<div style=\"white-space:pre-wrap\">Dear Alice,\n\nThanks for the report.\nSee you soon.\n</div>";
    assert_eq!(
        pieces(html),
        [
            vec![text("Dear Alice,")],
            vec![
                text("Thanks for the report."),
                Piece::Break(1),
                text("See you soon.")
            ],
        ]
    );
}

#[test]
fn soft_newlines_join_a_wrapped_paragraph() {
    let line1 = "This is a long paragraph that was hard wrapped by the sender's client at";
    let line2 = "about seventy-eight columns and keeps going until it finally ends here.";
    assert!(line1.chars().count() >= 60);
    let html = format!("<pre>{line1}\n{line2}\n\nNext one</pre>");
    assert_eq!(
        texts(&html),
        [format!("{line1} {line2}"), "Next one".to_owned()]
    );
}

#[test]
fn newline_before_a_blank_line_is_never_soft() {
    let long = "x".repeat(70);
    let html = format!("<pre>{long} words\n\nafter words</pre>");
    assert_eq!(
        texts(&html),
        [format!("{long} words"), "after words".into()]
    );
}

#[test]
fn line_length_carries_across_links() {
    let lead = "A long opening sentence that fills most of the line and then links to ";
    let html = format!(
        "<div style=\"white-space:pre-wrap\">{lead}<a href=\"https://e.test\" data-mailx-link=\"1\">a page</a> okay\ncontinuing on the next line here.</div>"
    );
    // The line before the newline is longer than 60 characters in total.
    assert_eq!(texts(&html).len(), 1, "{:?}", texts(&html));
}

#[test]
fn white_space_override_and_other_values() {
    for value in [
        "pre",
        "pre-wrap",
        "pre-line",
        "break-spaces",
        "PRE-WRAP !important",
    ] {
        let html = format!("<div style=\"white-space: {value}\">one two\nthree four</div>");
        assert_eq!(pieces(&html)[0].len(), 3, "{value}");
    }
    let html = "<div style=\"white-space:nowrap\">one two\nthree four</div>";
    assert_eq!(pieces(html)[0], [text("one two three four")]);
    let html = "<pre style=\"white-space:normal\">one two\nthree four</pre>";
    assert_eq!(pieces(html)[0], [text("one two three four")]);
    // Inherited by descendants.
    let html =
        "<div style=\"white-space:pre-wrap\"><blockquote>one two\nthree four</blockquote></div>";
    assert_eq!(pieces(html)[0].len(), 3);
}

#[test]
fn skipped_subtrees_inside_preformatted_text_keep_their_newlines() {
    let html = "<div style=\"white-space:pre\"><style>a {\n color: red }</style>Some words</div>";
    let out = apply_all(html, &["一些词语"]);
    assert!(out.contains("a {\n color: red }"), "{out}");
    assert!(out.contains("一些词语"));
}

#[test]
fn plain_text_mail_round_trip() {
    let body = concat!(
        "This is a long paragraph that was hard wrapped by the sender's mail client at about\r\n",
        "eighty columns and keeps going on the next line until it ends right here.\r\n",
        "\r\n",
        "Thanks,\r\n",
        "Alice\r\n",
        "Example Corp\r\n",
        "\r\n",
        "> quoted line one here\r\n",
        "> quoted line two\r\n",
    );
    let raw = format!(
        "From: a@example.com\r\nTo: b@example.com\r\nSubject: Hi\r\nMessage-ID: <1@example.com>\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{body}"
    );
    let rendered = crate::mail::render::render(raw.as_bytes(), "m1").unwrap();
    let html = rendered.sanitized_html.unwrap();
    assert!(html.contains("white-space:pre-wrap"), "{html}");
    let segments = extract(&html, true).unwrap();
    let plain: Vec<String> = segments.iter().map(Segment::plain_text).collect();
    assert_eq!(
        plain,
        [
            "This is a long paragraph that was hard wrapped by the sender's mail client at about eighty columns and keeps going on the next line until it ends right here.",
            "Thanks,\nAlice\nExample Corp",
            "quoted line one here\nquoted line two",
        ]
    );
    assert!(
        segments[1]
            .pieces()
            .iter()
            .any(|p| matches!(p, Piece::Break(_)))
    );
    // Translated signature keeps its lines.
    let mut translations = wires(&html, true);
    translations[1] = Some(r#"谢谢，<br id="1">爱丽丝<br id="2">示例公司"#.to_owned());
    let out = apply(&html, true, &translations).unwrap();
    assert!(out.contains("谢谢，<br>爱丽丝<br>示例公司"), "{out}");
    assert!(out.contains("<blockquote"));
}

// ------------------------------------------------------------ applying

#[test]
fn identity_round_trip_keeps_text_and_shape() {
    let samples = [
        r#"<p class="MsoNormal"><span style="x"><font>Hello <b>Mr. Smith</b>, review it.</font></span></p>"#,
        r#"<div dir="ltr">First paragraph.<br><br>Second one<br>same para<div><br></div></div>"#,
        r#"<table><tbody><tr><td>Name here</td><td><b>Alice</b></td></tr></tbody></table>"#,
        r#"<ul><li>One item</li><li>Two <i>items</i> here</li></ul>"#,
        r#"<p>Click <a href="https://e.test" data-mailx-link="3">here</a> to see <img data-mailx-cid="1"> the logo.</p>"#,
        r#"<p>Nested <b>bold <i>and <u>under</u></i> text</b> done<br>after</p>"#,
        FOLDED,
    ];
    for html in samples {
        let out = apply(html, true, &wires(html, true)).unwrap();
        assert_eq!(visible(&out), visible(html), "{html}\n{out}");
        assert_eq!(shape(&out), shape(html), "{html}\n{out}");
    }
}

#[test]
fn link_and_image_slots_stay_on_their_elements() {
    let html = r#"<p>Click <a href="https://e.test/a" data-mailx-link="3">here</a> to see <img data-mailx-cid="1" alt="logo"> the logo now.</p>"#;
    let out = apply_all(
        html,
        &[r#"点击<span id="1">此处</span>查看<img id="2">标志。"#],
    );
    assert_eq!(
        out,
        norm(
            r#"<p>点击<a href="https://e.test/a" data-mailx-link="3">此处</a>查看<img data-mailx-cid="1" alt="logo">标志。</p>"#
        )
    );
    assert!(out.contains(r#"data-mailx-link="3""#) && out.contains(r#"data-mailx-cid="1""#));
}

#[test]
fn reordered_and_nested_placeholders() {
    let html = r#"<p>Press <b>the <i>big</i> button</b> and <a href="u" data-mailx-link="1">read</a> it.</p>"#;
    // Placeholders: 1 b, 2 i, 3 a.
    assert_eq!(
        wire::encode(&extract(html, true).unwrap()[0]),
        r#"Press <span id="1">the <span id="2">big</span> button</span> and <span id="3">read</span> it."#
    );
    let out = apply_all(
        html,
        &[r#"<span id="3">阅读</span>它，然后按<span id="2">大</span><span id="1">按钮</span>。"#],
    );
    assert_eq!(
        out,
        norm(r#"<p><a href="u" data-mailx-link="1">阅读</a>它，然后按<i>大</i><b>按钮</b>。</p>"#)
    );
    // Nesting changed by the translator.
    let out = apply_all(
        html,
        &[r#"<span id="1">按<span id="2">大</span>按钮</span>，<span id="3">读</span>。"#],
    );
    assert_eq!(
        out,
        norm(r#"<p><b>按<i>大</i>按钮</b>，<a href="u" data-mailx-link="1">读</a>。</p>"#)
    );
}

#[test]
fn missing_optional_placeholder_drops_that_element() {
    let html = "<p>Hello <b>brave</b> new <i>world</i> today</p>";
    let out = apply_all(html, &[r#"你好新<span id="2">世界</span>"#]);
    assert_eq!(out, "<p>你好新<i>世界</i></p>");
    // A dropped break is dropped too.
    let html = "<p>Line one here<br>Line two here</p>";
    let out = apply_all(html, &["第一行第二行"]);
    assert_eq!(out, "<p>第一行第二行</p>");
}

#[test]
fn missing_required_placeholder_leaves_the_segment() {
    let link = r#"<p>Open <a href="u" data-mailx-link="1">the page</a> now</p>"#;
    assert_eq!(apply_all(link, &["现在打开页面"]), norm(link));
    let image = r#"<p>See <img data-mailx-cid="1"> now please</p>"#;
    assert_eq!(apply_all(image, &["现在请看"]), norm(image));
}

#[test]
fn invalid_placeholders_fall_back_to_plain_text_without_required_ids() {
    let html = "<p>Hello <b>big</b> world</p>";
    // Unknown id, wrong kind, duplicate id.
    for bad in [
        r#"你好<span id="9">大</span>世界"#,
        r#"你好<img id="1">世界"#,
        r#"你好<span id="1">大</span><span id="1">大</span>世界"#,
        r#"你好<span id="0">大</span>世界"#,
    ] {
        let out = apply_all(html, &[bad]);
        assert!(out.starts_with("<p>你好"), "{bad}: {out}");
        assert!(!out.contains("<b>"), "{bad}: {out}");
        assert_eq!(shape(&out).len(), 1, "{out}");
    }
    let out = apply_all(html, &[r#"你好<span id="9">大</span>世界"#]);
    assert_eq!(out, "<p>你好大世界</p>");
}

#[test]
fn invalid_placeholders_leave_segments_that_need_them() {
    let html = r#"<p>Open <a href="u" data-mailx-link="1">the page</a> now</p>"#;
    for bad in [
        r#"现在<span id="1">打开</span><span id="1">页面</span>"#,
        r#"现在<img id="1">打开"#,
        r#"现在<span id="2">打开</span>"#,
    ] {
        assert_eq!(apply_all(html, &[bad]), norm(html), "{bad}");
    }
}

#[test]
fn empty_translations_leave_the_segment() {
    let html = "<p>Hello world</p><p>Second one</p>";
    for empty in [
        "",
        "   ",
        "\n",
        "<b></b>",
        "<span id=\"1\"></span>",
        "&nbsp;",
    ] {
        assert_eq!(apply_all(html, &[empty, empty]), html, "{empty:?}");
    }
}

#[test]
fn partial_none_translations() {
    let html = "<p>Hello world</p><p>Second one</p><p>Third item</p>";
    let out = apply(html, true, &[None, Some("第二个".into()), None]).unwrap();
    assert_eq!(out, "<p>Hello world</p><p>第二个</p><p>Third item</p>");
}

#[test]
fn stale_count_is_none() {
    let html = "<p>Hello world</p><p>Second one</p>";
    assert_eq!(apply(html, true, &[]), None);
    assert_eq!(apply(html, true, &[Some("一".into())]), None);
    assert_eq!(apply(html, true, &[None, None, None]), None);
    assert!(apply(html, true, &[None, None]).is_some());
    assert_eq!(apply("", true, &[]), Some(String::new()));
}

#[test]
fn oversize_input() {
    let html = format!("<p>{}</p>", "a ".repeat(MAX_HTML_BYTES));
    assert!(html.len() > MAX_HTML_BYTES);
    assert_eq!(extract(&html, true), Err(ExtractError::TooLarge));
    assert_eq!(apply(&html, true, &[]), None);
}

#[test]
fn segment_cap_is_mirrored() {
    let html: String = (0..MAX_SEGMENTS + 20)
        .map(|i| format!("<p>Item {i}</p>"))
        .collect();
    let segments = extract(&html, true).unwrap();
    assert_eq!(segments.len(), MAX_SEGMENTS);
    let out = apply(&html, true, &wires(&html, true)).unwrap();
    assert_eq!(visible(&out), visible(&html));
    let translations = vec![Some("译文".to_owned()); MAX_SEGMENTS];
    let out = apply(&html, true, &translations).unwrap();
    assert_eq!(out.matches("译文").count(), MAX_SEGMENTS);
    assert!(out.contains("Item 5019"));
    assert_eq!(apply(&html, true, &translations[..10]), None);
}

#[test]
fn moved_top_level_nodes_survive_reordering() {
    // The break and the image are direct siblings of the text being replaced.
    let html = r#"<div>First line here<br><img data-mailx-cid="1">Second words</div>"#;
    let out = apply_all(html, &[r#"<img id="2">第二<br id="1">第一行"#]);
    assert_eq!(out, r#"<div><img data-mailx-cid="1">第二<br>第一行</div>"#);
}

#[test]
fn translation_text_is_escaped() {
    let html = "<p>Hello world</p>";
    let out = apply_all(
        html,
        &["a &lt; b &amp; c &gt; d &quot;e&quot; &#60;script&#62;"],
    );
    assert_eq!(out, "<p>a &lt; b &amp; c &gt; d \"e\" &lt;script&gt;</p>");
}

#[test]
fn hostile_translations_never_add_elements_or_attributes() {
    let with_link = r#"<div>Visit <a href="https://e.test/x" data-mailx-link="1" class="k">the page</a> and <img data-mailx-cid="2" alt="logo"> then <b>read</b>.</div><p>Second <i>part</i> here</p>"#;
    let no_required = "<div>Plain <b>bold</b> sentence<br>again</div><p>Other <u>part</u> text</p>";
    let hostile = [
        "<script>alert(1)</script>Hello",
        "<img src=x onerror=alert(1)>Hello <img id=\"2\">",
        r#"<a href="javascript:alert(1)">x</a> <span id="1">y</span><img id="2">"#,
        r#"<span id="1" onclick="alert(1)" style="x" class="y">Hi</span><img id="2">"#,
        r#"<span id="1" data-mailx-link="9" href="//evil">Hi</span><img id="2" src="//evil">"#,
        "<iframe src=\"//evil\"></iframe>text<object data=x></object><embed src=x>",
        "<form action=//evil><input name=x></form>text",
        "<svg onload=alert(1)><circle/></svg>text<math><mi>x</mi></math>",
        "<style>*{display:none}</style>text",
        "<base href=//evil><meta http-equiv=refresh content=0>text",
        r#"<span id="1">Hi<b>unbalanced"#,
        r#"</span></span></div>text<span id="1">"#,
        "&lt;script&gt;alert(1)&lt;/script&gt; &amp;lt; &#x3c;b&#x3e;",
        "Hello\u{0}<span id=\"1\">x\u{0}</span>\u{0}",
        r#"<span id="999999999999">a</span><span id="-1">b</span><span id="1e3">c</span><span id="+1">d</span><span id="１">e</span> text"#,
        r#"<span id="65537">a</span><span id="0">b</span><br id="x"><img id=""> text"#,
        r#"<span id="1"><span id="1"><span id="1">deep</span></span></span><img id="2"><img id="2">"#,
        r#"<span id=1 id=2>dup attr</span><img id=2> and"#,
        "<![CDATA[ <script> ]]> <!-- <img src=x> --> text",
        "<template><script>alert(1)</script></template>text",
        "<table><tr><td>cell</td></tr></table>text",
        "<p>para</p><div>div</div><h1>head</h1>",
        "<noscript><img src=x></noscript> text <textarea><img src=x></textarea>",
    ];
    for html in [with_link, no_required] {
        for translation in hostile {
            let list: Vec<Option<String>> = (0..extract(html, true).unwrap().len())
                .map(|_| Some(translation.to_owned()))
                .collect();
            let out = apply(html, true, &list).unwrap();
            assert_subset(&out, html);
            // No attribute value from the translation leaked in either.
            for forbidden in ["alert(1)\"", "//evil", "onerror", "onclick", "javascript:"] {
                assert!(
                    !shape(&out).keys().any(|k| k.contains(forbidden)),
                    "{forbidden}: {out}"
                );
            }
            // The output is stable under another parse/serialize round.
            assert_subset(
                &Html::parse_fragment(&out).root_element().inner_html(),
                html,
            );
        }
    }
}

#[test]
fn valid_translation_cannot_add_attributes_to_placeholders() {
    let html =
        r#"<p>Open <a href="u" data-mailx-link="1">the page</a> <img data-mailx-cid="2"> now</p>"#;
    let out = apply_all(
        html,
        &[
            r#"打开<span id="1" onclick="x" href="//evil" data-mailx-link="9">页面</span><img id="2" src="//evil" onerror="x">"#,
        ],
    );
    assert_eq!(
        out,
        norm(r#"<p>打开<a href="u" data-mailx-link="1">页面</a><img data-mailx-cid="2"></p>"#)
    );
}

#[test]
fn untranslated_preformatted_text_is_not_reflowed() {
    let line1 = "This is a long paragraph that was hard wrapped by the sender's client at";
    let html = format!(
        "<div style=\"white-space:pre-wrap\">{line1}\nabout seventy-eight columns and goes on.\n\nShort one\nsecond short\n1234\n5678</div>"
    );
    let count = extract(&html, true).unwrap().len();
    for translations in [vec![None; count], vec![Some("   ".to_owned()); count]] {
        let out = apply(&html, true, &translations).unwrap();
        let flat = |html: &str| {
            let document = Html::parse_fragment(html);
            let mut out = String::new();
            for node in document.root_element().descendants() {
                match node.value() {
                    Node::Text(text) => out.push_str(&text.text),
                    Node::Element(element) if element.name() == "br" => out.push('\n'),
                    _ => {}
                }
            }
            out
        };
        assert_eq!(flat(&out), flat(&html));
    }
}

#[test]
fn empty_decorative_inline_may_be_dropped() {
    let html = r#"<p>Hello <span style="mso-spacerun:yes"></span>brave <b>new</b> world</p>"#;
    let wire = wire::encode(&extract(html, true).unwrap()[0]);
    assert!(wire.contains(r#"<img id="1">"#), "{wire}");
    let out = apply_all(html, &[r#"你好<span id="2">新</span>世界"#]);
    assert_eq!(out, "<p>你好<b>新</b>世界</p>");
}

#[test]
fn image_link_dropped_by_translation_leaves_the_segment() {
    let html =
        r#"<p>Look <a data-mailx-link="0"><img data-mailx-cid="x"></a> at this nice logo.</p>"#;
    assert_eq!(apply_all(html, &["看这个标志。"]), norm(html));
    let kept = apply_all(html, &[r#"看<img id="1">这个标志。"#]);
    assert!(kept.contains("data-mailx-link") && kept.contains("data-mailx-cid"));
}

#[test]
fn hostile_wire_decodes_to_pieces_only() {
    let pieces = wire::decode(
        r#"<script>x</script><p>a<span id="1" onclick="x">b</span></p><br id="2"><img id="3" src="x"><br><img>c"#,
    );
    assert_eq!(
        pieces,
        [
            text("xa"),
            Piece::Open(1),
            text("b"),
            Piece::Close(1),
            Piece::Break(2),
            Piece::Object(3),
            text("c"),
        ]
    );
}

// ----------------------------------------------------------------- wire

#[test]
fn encode_escapes_and_marks() {
    let segments = extract(
        r#"<p>Tom &amp; "Jerry" &lt;3 <b>x</b><br>y <img></p>"#,
        true,
    )
    .unwrap();
    assert_eq!(
        wire::encode(&segments[0]),
        r#"Tom &amp; &quot;Jerry&quot; &lt;3 <span id="1">x</span><br id="2">y <img id="3">"#
    );
}

#[test]
fn decode_inverts_encode() {
    for html in [
        "<p>Tom &amp; Jerry &lt;b&gt; <b>x &quot;q&quot;</b><br>y</p>",
        r#"<p>One <a href="u" data-mailx-link="1">two <i>three</i></a> <img data-mailx-cid="1"> four</p>"#,
    ] {
        for segment in extract(html, true).unwrap() {
            assert_eq!(wire::decode(&wire::encode(&segment)), segment.pieces());
        }
    }
}

#[test]
fn decode_is_lenient() {
    assert_eq!(wire::decode(""), []);
    assert_eq!(wire::decode("plain &amp; simple"), [text("plain & simple")]);
    assert_eq!(
        wire::decode(r#"<b>bold</b> <span>plain span</span> <div>block</div>"#),
        [text("bold plain span block")]
    );
    assert_eq!(
        wire::decode(r#"<span id="1">a<span id="2">b"#),
        [
            Piece::Open(1),
            text("a"),
            Piece::Open(2),
            text("b"),
            Piece::Close(2),
            Piece::Close(1),
        ]
    );
    assert_eq!(wire::decode("<!-- c -->x<!DOCTYPE html>"), [text("x")]);
    assert_eq!(
        wire::decode(r#"<span id="70000">a</span><span id="-3">b</span>"#),
        [text("ab")]
    );
}

#[test]
fn decode_survives_deep_nesting() {
    let depth = 50_000;
    let input = format!(
        "{}x{}",
        r#"<span id="1">"#.repeat(depth),
        "</span>".repeat(depth)
    );
    assert_eq!(wire::decode(&input).len(), depth * 2 + 1);
}

// ------------------------------------------------------- tag-name encoding

#[test]
fn tag_encoding_writes_the_documented_tokens_and_inverts() {
    let html = r#"<p>Tom &amp; <b>x &lt;g1&gt; 3 &gt; 2</b><br>y <img data-mailx-cid="1"> "q"</p>"#;
    for segment in extract(html, true).unwrap() {
        let tags = wire::encode_tags(segment.pieces());
        assert_eq!(
            tags,
            r#"Tom &amp; <g1>x &lt;g1&gt; 3 &gt; 2</g1><n2/>y <x3/> "q""#
        );
        assert_eq!(wire::decode_tags(&tags), segment.pieces());
    }
}

#[test]
fn tag_decoding_accepts_only_exact_tokens() {
    let d = wire::decode_tags;
    assert_eq!(
        d("a<g1>b<n22/>c<x3 />d</g1>"),
        [
            text("a"),
            Piece::Open(1),
            text("b"),
            Piece::Break(22),
            text("c"),
            Piece::Object(3),
            text("d"),
            Piece::Close(1),
        ]
    );
    // Five digits, no digits, wrong kind, attributes, case, closing forms of n / x.
    for literal in [
        "<g12345>",
        "<g>",
        "</g>",
        "<n1>",
        "<x1>",
        "</n1/>",
        "</x1>",
        "<g1 onclick=alert(1)>",
        "<g1/>",
        "<G1>",
        "<g1 >",
        "<n1  />",
        "<n1/ >",
        "<g-1>",
        "<g+1>",
        "<g１>",
        "<script>x</script>",
        "<span id=\"1\">",
        "<g1",
        "<",
    ] {
        assert_eq!(d(literal), [text(literal)], "{literal}");
    }
    // Entities: the agreed set, once; anything else stays literal.
    assert_eq!(
        d("&lt;g1&gt; &amp;lt; &quot;&#39;&apos; &nbsp; &#60; &unknown; & &"),
        [text("<g1> &lt; \"'' &nbsp; &#60; &unknown; & &")]
    );
    assert_eq!(d(""), []);
    assert_eq!(
        d("\u{0}<g1>\u{0}</g1>"),
        [
            text("\u{0}"),
            Piece::Open(1),
            text("\u{0}"),
            Piece::Close(1)
        ]
    );
    assert_eq!(
        d("日本<g1>語</g1>"),
        [text("日本"), Piece::Open(1), text("語"), Piece::Close(1)]
    );
}

#[test]
fn tag_decoding_survives_deep_and_wrong_nesting() {
    let depth = 50_000;
    let input = format!("{}x{}", "<g1>".repeat(depth), "</g1>".repeat(depth));
    assert_eq!(wire::decode_tags(&input).len(), depth * 2 + 1);
    // Wrong order and unbalanced tags decode to pieces; the applier decides.
    assert_eq!(
        wire::decode_tags("</g1>a<g2>b</g1>"),
        [
            Piece::Close(1),
            text("a"),
            Piece::Open(2),
            text("b"),
            Piece::Close(1)
        ]
    );
}

#[test]
fn tag_balance_requires_matching_nesting() {
    let balanced = |answer: &str| wire::tags_balanced(&wire::decode_tags(answer));
    assert!(balanced("a<g1>b<g2>c</g2></g1><n3/><x4/>"));
    assert!(balanced("plain text"));
    assert!(!balanced("<g1>a<g2>b</g1>c</g2>"), "crossed");
    assert!(!balanced("</g1>a"), "close before open");
    assert!(!balanced("<g1>a"), "left open");
}

#[test]
fn hostile_tag_answers_never_add_elements_or_attributes() {
    let with_link = r#"<div>Visit <a href="https://e.test/x" data-mailx-link="1" class="k">the page</a> and <img data-mailx-cid="2" alt="logo"> then <b>read</b>.</div><p>Second <i>part</i> here</p>"#;
    let no_required = "<div>Plain <b>bold</b> sentence<br>again</div><p>Other <u>part</u> text</p>";
    let hostile = [
        "<script>alert(1)</script>Hello",
        "<img src=x onerror=alert(1)>Hello <x2/>",
        "<g1 onclick=\"alert(1)\" href=\"//evil\">Hi</g1><x2/>",
        "<g99999>a</g99999><g65536>b</g65536><n0/><x00002/>",
        "<g1><g1><g1>deep</g1></g1></g1><x2/><x2/>",
        "</g1></g1>text<g1>",
        "<g1>Hi<g2>unbalanced",
        "&lt;script&gt;alert(1)&lt;/script&gt; &amp;lt; &#x3c;b&#x3e; &lt;g1&gt;",
        "Hello\u{0}<g1>x\u{0}</g1>\u{0}",
        "<n1/><n1/><n1/><x1/><x1/>",
        "<span id=\"1\">a</span><br id=\"2\"><img id=\"2\"> text",
        "<![CDATA[ <script> ]]> <!-- <img src=x> --> text",
    ];
    for html in [with_link, no_required] {
        for answer in hostile {
            // The same normalisation the Edge client applies before caching.
            let normalized = wire::encode_pieces(&wire::decode_tags(answer));
            let list: Vec<Option<String>> = (0..extract(html, true).unwrap().len())
                .map(|_| Some(normalized.clone()))
                .collect();
            let out = apply(html, true, &list).unwrap();
            assert_subset(&out, html);
            for forbidden in ["alert(1)\"", "//evil", "onerror", "onclick", "javascript:"] {
                assert!(
                    !shape(&out).keys().any(|k| k.contains(forbidden)),
                    "{forbidden}: {out}"
                );
            }
        }
    }
}

#[test]
fn an_edge_style_answer_is_rewritten_in_the_original_elements() {
    let html = r#"<p>Your order <b>#4521</b> has shipped.<br>Track it <a href="u" data-mailx-link="1">on <i>our website</i></a> now <img data-mailx-cid="1"></p>"#;
    let segment = &extract(html, true).unwrap()[0];
    assert_eq!(
        wire::encode_tags(segment.pieces()),
        "Your order <g1>#4521</g1> has shipped.<n2/>Track it <g3>on <g4>our website</g4></g3> now <x5/>"
    );
    let answer = "您的订单<g1>#4521</g1>已发货。<n2/>请<g3>在<g4>我们的网站上</g4></g3>追踪<x5/>";
    let normalized = wire::encode_pieces(&wire::decode_tags(answer));
    let out = apply(html, true, &[Some(normalized)]).unwrap();
    assert_eq!(
        out,
        norm(
            r#"<p>您的订单<b>#4521</b>已发货。<br>请<a href="u" data-mailx-link="1">在<i>我们的网站上</i></a>追踪<img data-mailx-cid="1"></p>"#
        )
    );
}

// ------------------------------------------------------------- languages

#[test]
fn language_table() {
    use languages::*;
    assert_eq!(TARGETS.len(), 26);
    for code in TARGETS {
        assert!(is_target(code), "{code}");
    }
    assert!(!is_target("zh"));
    assert!(!is_target("en-US"));
    assert!(!is_target(""));
    assert_eq!(base("zh-Hans"), "zh");
    assert_eq!(base("en-US"), "en");
    assert_eq!(base("pt_BR"), "pt");
    assert_eq!(base("fr"), "fr");
    assert_eq!(base(""), "");
    assert!(same_language("zh-Hans", "zh-Hant"));
    assert!(same_language("zh", "zh-Hant"));
    assert!(same_language("EN-us", "en"));
    assert!(same_language("nb", "no"));
    assert!(same_language("no", "nb"));
    assert!(!same_language("en", "fr"));
    assert!(!same_language("", ""));
    assert_eq!(for_ui_language("zh-CN"), "zh-Hans");
    assert_eq!(for_ui_language("en"), "en");
    assert_eq!(for_ui_language("en-US"), "en");
    assert_eq!(for_ui_language("zh-TW"), "en");
    assert_eq!(for_ui_language(""), "en");
}

fn detected(text: &str) -> (String, bool) {
    let detection = detect_language(text).unwrap_or_else(|| panic!("no detection for {text}"));
    (detection.language, detection.reliable)
}

#[test]
fn detects_chinese_in_both_scripts() {
    let simplified = "您好，感谢您对我们产品的关注。我们已经收到您的订单，预计三个工作日内发货，请保持电话畅通。";
    let traditional = "您好，感謝您對我們產品的關注。我們已經收到您的訂單，預計三個工作日內發貨，請保持電話暢通。";
    assert_eq!(detected(simplified), ("zh".into(), true));
    assert_eq!(detected(traditional), ("zh".into(), true));
    // Short but long enough to say; not enough to be reliable.
    assert_eq!(
        detected("谢谢您的来信，我们会尽快回复"),
        ("zh".into(), false)
    );
}

#[test]
fn detects_japanese_and_korean() {
    assert_eq!(
        detected(
            "お世話になっております。先日ご依頼いただいた件について、資料を添付いたしましたのでご確認ください。"
        ),
        ("ja".into(), true)
    );
    // Kanji-heavy, with a little kana.
    assert_eq!(
        detected(
            "本日会議資料送付致候。確認願上候。来週月曜日午前十時開始予定。宜しくお願いします。"
        ),
        ("ja".into(), true)
    );
    assert_eq!(
        detected(
            "안녕하세요. 주문해 주셔서 감사합니다. 상품은 영업일 기준 삼 일 이내에 배송될 예정입니다."
        ),
        ("ko".into(), true)
    );
}

#[test]
fn detects_latin_and_cyrillic_languages() {
    let cases = [
        (
            "en",
            "Hello Alice, thank you for your order. We have received your payment and your package will be shipped within three business days. Please contact us if you have any questions.",
        ),
        (
            "fr",
            "Bonjour Alice, merci pour votre commande. Nous avons bien reçu votre paiement et votre colis sera expédié dans un délai de trois jours ouvrables. N'hésitez pas à nous contacter.",
        ),
        (
            "de",
            "Hallo Alice, vielen Dank für Ihre Bestellung. Wir haben Ihre Zahlung erhalten und Ihr Paket wird innerhalb von drei Werktagen versendet. Bei Fragen melden Sie sich gerne bei uns.",
        ),
        (
            "es",
            "Hola Alice, gracias por tu pedido. Hemos recibido tu pago y tu paquete será enviado en un plazo de tres días hábiles. Si tienes alguna pregunta, no dudes en contactarnos.",
        ),
        (
            "ru",
            "Здравствуйте, Алиса! Спасибо за ваш заказ. Мы получили ваш платёж, и посылка будет отправлена в течение трёх рабочих дней. Если у вас есть вопросы, свяжитесь с нами.",
        ),
    ];
    for (code, sample) in cases {
        assert_eq!(detected(sample), (code.to_owned(), true), "{code}");
    }
}

#[test]
fn urls_emails_and_digits_do_not_count() {
    let mixed = "您好，Apple iPhone 15 Pro Max 与 MacBook Air M3 已经发货，查看物流请访问 https://example.com/track?id=12345&ref=mail 或联系 support@example.com，订单号 20261103123456。";
    assert_eq!(detected(mixed), ("zh".into(), true));
    // The English around the address must not be drowned by it.
    let english = "Please write to someone.very.long.address@subdomain.example-company.com or visit www.example-company.com/support/articles/12345 for the complete documentation, thanks.";
    assert_eq!(detected(english).0, "en");
    // Only noise.
    assert_eq!(
        detect_language("https://example.com/a/b/c?d=e 123456 789 foo@example.com"),
        None
    );
}

#[test]
fn short_texts_are_unreliable_or_absent() {
    let code = detect_language("Your verification code is 482913");
    assert!(code.as_ref().is_none_or(|d| !d.reliable), "{code:?}");
    assert_eq!(detect_language("OK 123"), None);
    assert_eq!(detect_language(""), None);
    assert_eq!(detect_language("验证码已发送"), None);
    assert_eq!(detect_language("123 456 789 000 111"), None);
    let code = detect_language("验证码：482913，五分钟内有效，请勿泄露给他人");
    assert!(code.is_some_and(|d| d.language == "zh" && !d.reliable));
}

#[test]
fn language_codes_are_iso_639_1_when_available() {
    let (code, _) = detected(
        "Dit is een voorbeeld van een Nederlandse zin die lang genoeg is om te herkennen, zodat de detectie goed werkt.",
    );
    assert_eq!(code, "nl");
    let (code, _) = detected(
        "Dzień dobry, dziękujemy za złożenie zamówienia. Paczka zostanie wysłana w ciągu trzech dni roboczych.",
    );
    assert_eq!(code, "pl");
    let (code, _) = detected(
        "Merhaba, siparişiniz için teşekkür ederiz. Paketiniz üç iş günü içinde kargoya verilecektir.",
    );
    assert_eq!(code, "tr");
    let (code, _) =
        detected("مرحبا، شكرا لطلبك. تم استلام الدفع وسيتم شحن الطرد خلال ثلاثة أيام عمل.");
    assert_eq!(code, "ar");
}

#[test]
fn mostly_english_with_a_few_cjk_characters_is_english() {
    let sample = "Please find the attached quarterly report for the finance team, and let me know whether the figures in the summary look right to you. 谢谢";
    assert_eq!(detected(sample).0, "en");
}

#[test]
fn the_walk_reports_truncation_only_past_the_segment_cap() {
    let paragraphs = |n: usize| {
        (0..n)
            .map(|i| format!("<p>item {i}</p>"))
            .collect::<String>()
    };
    let exact = extract_checked(&paragraphs(MAX_SEGMENTS), true).unwrap();
    assert_eq!(
        (exact.segments.len(), exact.truncated),
        (MAX_SEGMENTS, false)
    );
    let over = extract_checked(&paragraphs(MAX_SEGMENTS + 1), true).unwrap();
    assert_eq!((over.segments.len(), over.truncated), (MAX_SEGMENTS, true));
    // Trailing markup that yields no segment is not "more text".
    let tail = format!("{}<p></p><div> </div>", paragraphs(MAX_SEGMENTS));
    assert!(!extract_checked(&tail, true).unwrap().truncated);
    assert_eq!(
        extract(&paragraphs(MAX_SEGMENTS + 1), true).unwrap(),
        over.segments
    );
}
