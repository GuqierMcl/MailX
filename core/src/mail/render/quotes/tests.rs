use super::fold_quotes;

const OPEN: &str = r#"<details class="mailx-quote"><summary>QUOTE</summary>"#;

fn fold(html: &str) -> String {
    fold_quotes(html, "QUOTE")
}

/// The folded text must be exactly `hidden`, and the text before it stays out.
fn assert_folded(html: &str, visible: &str, hidden: &str) {
    let out = fold(html);
    let (before, rest) = out
        .split_once(OPEN)
        .unwrap_or_else(|| panic!("not folded: {out}"));
    assert!(before.contains(visible), "{before}");
    let inside = rest.split_once("</details>").unwrap().0;
    assert!(inside.contains(hidden), "{inside}");
    assert!(!before.contains(hidden), "{before}");
    assert_eq!(out.matches("<details").count(), 1, "{out}");
}

fn assert_unchanged(html: &str) {
    assert_eq!(fold(html), html);
}

#[test]
fn gmail_quote_folds_and_reply_text_stays() {
    let html = r#"<div dir="ltr">Thanks, see you.</div><br><div class="gmail_quote"><div dir="ltr" class="gmail_attr">On 2026-11-03 19:12, "顾七儿" &lt;<a href="mailto:2535810484@qq.com">2535810484@qq.com</a>&gt; wrote:<br></div><blockquote class="gmail_quote" style="margin:0px 0px 0px 0.8ex">Hi Leo, original</blockquote></div>"#;
    assert_folded(html, "Thanks, see you.", "Hi Leo, original");
    let out = fold(html);
    assert!(out.starts_with(r#"<div dir="ltr">Thanks, see you.</div>"#));
    assert!(out.contains("wrote:"));
}

#[test]
fn mailx_nested_reply_folds_the_outermost_once() {
    let html = r#"<p style="margin:0">Agreed.</p><p style="margin:0">2026-11-11 17:36，18187128344@163.com 写道：</p><blockquote style="margin:0"><p style="margin:0">2026-11-10 09:00，a@b.c 写道：</p><blockquote style="margin:0"><p>oldest</p></blockquote><p>older</p></blockquote>"#;
    assert_folded(html, "Agreed.", "oldest");
    let out = fold(html);
    assert_eq!(out.matches("<details").count(), 1);
    let before = out.split_once(OPEN).unwrap().0;
    assert!(!before.contains("写道"), "{before}");
}

#[test]
fn outlook_web_reply_folds_from_the_rule() {
    let html = r#"<div>My answer</div><div id="appendonsend"></div><hr style="display:inline-block;width:98%" tabindex="-1"><div id="divRplyFwdMsg" dir="ltr"><font face="Calibri"><b>From:</b> Bob<br><b>Sent:</b> Monday<br><b>Subject:</b> Hi</font></div><div>quoted body text</div>"#;
    assert_folded(html, "My answer", "quoted body text");
    let out = fold(html);
    assert!(!out.split_once(OPEN).unwrap().0.contains("<hr"));
    assert!(out.split_once(OPEN).unwrap().1.contains("<hr"));
}

#[test]
fn outlook_desktop_header_block_folds_with_its_wrapper() {
    let html = r#"<div>Reply here</div><div style="border:none;border-top:solid #E1E1E1 1.0pt;padding:3.0pt 0in 0in 0in"><p class="MsoNormal"><b>From:</b> Bob<br><b>Sent:</b> Monday<br><b>To:</b> me<br><b>Subject:</b> Plan</p></div><div><p>the earlier message</p></div>"#;
    assert_folded(html, "Reply here", "the earlier message");
    let zh = r#"<div>好的</div><div style="border:none;border-top:solid #E1E1E1 1.0pt"><p><b>发件人:</b> 张三<br><b>发送时间:</b> 周一<br><b>主题:</b> 计划</p></div><div>早前内容</div>"#;
    assert_folded(zh, "好的", "早前内容");
}

#[test]
fn qq_original_mail_separator_folds() {
    let html = r#"<div>收到</div><div><br></div><div style="font-size:12px"><div>------------------&nbsp;原始邮件&nbsp;------------------</div><div style="background:#efefef;padding:8px"><div><b>发件人:</b>&nbsp;"a" &lt;a@qq.com&gt;;</div><div><b>发送时间:</b>&nbsp;2026年11月11日(星期三) 下午5:36</div><div><b>收件人:</b>&nbsp;"b";</div><div><b>主题:</b>&nbsp;Re: 计划</div></div><div>早先正文</div></div>"#;
    assert_folded(html, "收到", "早先正文");
}

#[test]
fn separator_line_in_plain_text_node_is_split() {
    let html = "<div style=\"white-space:pre-wrap\">Sounds good\n\n-----Original Message-----\nFrom: Bob\nSubject: Hi\n\nold text</div>";
    let out = fold(html);
    assert!(out.contains("Sounds good"));
    let (before, rest) = out.split_once(OPEN).expect(&out);
    assert!(!before.contains("Original Message"));
    assert!(rest.contains("old text"));
}

#[test]
fn apple_mail_cite_blockquote_folds_with_attribution() {
    let html = r#"<div>Yes.</div><div><br><blockquote type="cite"><div>On Nov 3, 2026, at 7:12 PM, Bob &lt;b@x.com&gt; wrote:</div><br><div>question</div></blockquote></div>"#;
    assert_folded(html, "Yes.", "question");
    let separate = r#"<div>Yes.</div><div>On Nov 3, 2026, at 7:12 PM, Bob &lt;b@x.com&gt; wrote:</div><blockquote type="cite"><div>question</div></blockquote>"#;
    let out = fold(separate);
    assert!(!out.split_once(OPEN).unwrap().0.contains("wrote:"), "{out}");
}

#[test]
fn plain_text_derived_blockquote_folds_with_attribution_line() {
    let html = "<div style=\"white-space:pre-wrap\">Got it.\n\n在 2026年11月3日 19:12，张三 写道：\n<blockquote style=\"margin:0\">原文第一行\n原文第二行</blockquote></div>";
    let out = fold(html);
    let (before, rest) = out.split_once(OPEN).expect(&out);
    assert!(before.contains("Got it."));
    assert!(!before.contains("写道"));
    assert!(
        rest.starts_with("在 2026年11月3日 19:12，张三 写道："),
        "{rest}"
    );
}

#[test]
fn bare_trailing_blockquote_folds() {
    assert_folded(
        "<div>Noted</div><blockquote>quoted text</blockquote>",
        "Noted",
        "quoted text",
    );
}

#[test]
fn other_languages_attribution() {
    for line in [
        "Am 03.11.2026 um 10:00 schrieb Bob &lt;b@x.com&gt;:",
        "Le 3 nov. 2026 à 10:00, Bob a écrit :",
        "El 3 nov 2026, Bob escribió:",
        "Bob &lt;b@x.com&gt; wrote:",
    ] {
        let html = format!("<div>ok</div><div>{line}</div><blockquote>q</blockquote>");
        let out = fold(&html);
        assert!(!out.split_once(OPEN).expect(&out).0.contains(line), "{out}");
    }
}

#[test]
fn text_before_attribution_on_the_same_line_is_never_hidden() {
    let html = "<div>Thanks! On Mon, Bob wrote:</div><blockquote>q</blockquote>";
    let out = fold(html);
    let before = out.split_once(OPEN).expect(&out).0;
    assert!(before.contains("Thanks!"));
}

#[test]
fn interleaved_and_trailing_content_is_never_folded() {
    assert_unchanged(
        "<div>On Mon, Bob wrote:</div><blockquote>q</blockquote><div>my answer below</div>",
    );
    assert_unchanged(
        "<div>On Mon, Bob wrote:</div><blockquote>q1</blockquote><div>a1</div><div>On Mon, Bob wrote:</div><blockquote>q2</blockquote>",
    );
    assert_unchanged(
        r#"<div class="gmail_quote"><blockquote class="gmail_quote">q</blockquote></div><div>after</div>"#,
    );
    assert_unchanged("<div>a</div><div>From: x</div><div>just one labelled line</div>");
}

#[test]
fn a_message_that_is_only_a_quote_is_unchanged() {
    assert_unchanged("<blockquote>all quoted</blockquote>");
    assert_unchanged(
        r#"<div class="gmail_quote"><div class="gmail_attr">On Mon, Bob wrote:</div><blockquote>q</blockquote></div>"#,
    );
    assert_unchanged("<div><div>From: Bob</div><div>Subject: Hi</div><div>x</div></div>");
}

#[test]
fn forwarded_messages_are_unchanged() {
    assert_unchanged(
        r#"<div>FYI</div><div class="gmail_quote"><div class="gmail_attr">---------- Forwarded message ---------<br>From: Bob<br>Subject: Hi</div>text</div>"#,
    );
    assert_unchanged(
        "<div>FYI</div><div>-------- 转发邮件信息 --------</div><div>发件人: a</div><div>主题: b</div><div>x</div>",
    );
    assert_unchanged(r#"<div>FYI</div><div id="isForwardContent"><div>x</div></div>"#);
    assert_unchanged(
        "<div>FYI</div><div>Begin forwarded message:</div><blockquote type=\"cite\"><div>From: a</div></blockquote>",
    );
    assert_unchanged(
        "<div>FYI</div><div><b>From:</b> Bob<br><b>Subject:</b> FW: Plan</div><div>text</div>",
    );
}

#[test]
fn nothing_recognized_returns_the_original_string() {
    for html in [
        "<p>Hello</p><p>World</p>",
        "<table><tr><td>cell</td></tr></table>",
        "",
        "   ",
        "<p>The original plan wrote: nothing</p>",
    ] {
        assert_eq!(fold(html), html);
    }
}

#[test]
fn quote_directly_inside_table_cells_or_lists_is_not_folded() {
    assert_unchanged("<ul><li>a</li><blockquote>q</blockquote></ul>");
    assert_unchanged("<table><tbody><tr><td>x</td></tr></tbody></table>");
}

#[test]
fn oversized_input_is_returned_unchanged() {
    let html = format!(
        "<div>a</div><blockquote>q</blockquote>{}",
        "<div>x</div>".repeat(200_000)
    );
    assert!(html.len() > super::MAX_INPUT_BYTES);
    assert_eq!(fold(&html), html);
}

#[test]
fn deep_nesting_does_not_overflow() {
    let html = format!(
        "<div>a</div>{}q{}",
        "<blockquote>".repeat(5000),
        "</blockquote>".repeat(5000)
    );
    let _ = fold(&html);
}

#[test]
fn label_is_escaped_and_resources_inside_the_fold_survive() {
    let html = r#"<div>Hi</div><blockquote><img data-mailx-cid="pixel"></blockquote>"#;
    let out = fold_quotes(html, "<b>&");
    assert!(out.contains("<summary>&lt;b&gt;&amp;</summary>"), "{out}");
    assert!(out.contains(r#"data-mailx-cid="pixel""#));
}
