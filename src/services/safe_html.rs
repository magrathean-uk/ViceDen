use pulldown_cmark::{html, CowStr, Event, Options, Parser};

pub fn render_user_html(input: &str) -> String {
    let mut rendered = String::new();
    let events = Parser::new_ext(input, markdown_options()).flat_map(safe_event);
    html::push_html(&mut rendered, events);
    sanitize_rendered_html(&rendered)
}

fn markdown_options() -> Options {
    Options::ENABLE_STRIKETHROUGH
}

fn sanitize_rendered_html(input: &str) -> String {
    ammonia::Builder::default()
        .add_tags([
            "p",
            "br",
            "strong",
            "em",
            "code",
            "pre",
            "blockquote",
            "ul",
            "ol",
            "li",
            "a",
            "span",
        ])
        .add_generic_attributes(["class"])
        .add_tag_attributes("a", ["href", "title"])
        .url_relative(ammonia::UrlRelative::Deny)
        .clean(input)
        .to_string()
        .trim_end_matches('\n')
        .to_string()
}

fn safe_event(event: Event<'_>) -> Vec<Event<'static>> {
    match event {
        Event::Html(value) | Event::InlineHtml(value) => {
            vec![Event::Text(CowStr::from(value.to_string()))]
        }
        Event::Text(value) => mention_events(&value),
        other => vec![other.into_static()],
    }
}

fn mention_events(input: &str) -> Vec<Event<'static>> {
    let chars: Vec<char> = input.chars().collect();
    let mut events = Vec::new();
    let mut text = String::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '@' && is_mention_boundary(&chars, index) {
            let start = index + 1;
            let mut end = start;
            while end < chars.len() && is_handle_char(chars[end]) && end - start < 16 {
                end += 1;
            }
            let handle_len = end.saturating_sub(start);
            if (3..=16).contains(&handle_len) {
                if !text.is_empty() {
                    events.push(Event::Text(CowStr::from(std::mem::take(&mut text))));
                }
                let mut html = String::from("<span class=\"mention\">@");
                for ch in &chars[start..end] {
                    escape_char(*ch, &mut html);
                }
                html.push_str("</span>");
                events.push(Event::InlineHtml(CowStr::from(html)));
                index = end;
                continue;
            }
        }

        text.push(chars[index]);
        index += 1;
    }
    if !text.is_empty() {
        events.push(Event::Text(CowStr::from(text)));
    }
    events
}

fn is_mention_boundary(chars: &[char], index: usize) -> bool {
    index == 0 || chars[index - 1].is_whitespace()
}

fn is_handle_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'
}

fn escape_char(ch: char, output: &mut String) {
    match ch {
        '&' => output.push_str("&amp;"),
        '<' => output.push_str("&lt;"),
        '>' => output.push_str("&gt;"),
        '"' => output.push_str("&quot;"),
        '\'' => output.push_str("&#x27;"),
        _ => output.push(ch),
    }
}

#[cfg(test)]
mod tests {
    use super::render_user_html;

    #[test]
    fn escapes_script_tags() {
        let html = render_user_html("<script>alert(1)</script>");
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    }

    #[test]
    fn line_breaks_and_mentions_are_controlled() {
        let html = render_user_html("hello @atlas\nsecond line");
        assert!(html.contains("<span class=\"mention\">@atlas</span>"));
        assert!(html.contains("second line"));
    }

    #[test]
    fn markdown_links_code_and_mentions_render_without_raw_html() {
        let html = render_user_html(
            "hello **bold** `code` @atlas [site](https://example.com)<script>x</script>",
        );

        assert!(html.contains("<strong>bold</strong>"));
        assert!(html.contains("<code>code</code>"));
        assert!(html.contains("<span class=\"mention\">@atlas</span>"));
        assert!(html.contains("<a href=\"https://example.com\""));
        assert!(!html.contains("<script>"));
    }

    /// F6: comprehensive XSS regression sweep. Each input is a vector
    /// known to bypass naive sanitisers; the rendered HTML must NOT
    /// surface any active XSS surface — no `<script` tags, no
    /// `<iframe`/`<object`, and no `javascript:` URLs in hrefs.
    /// Substrings like "onerror" appearing as plain text inside an
    /// escaped span are harmless (the browser renders the text, not
    /// an attribute).
    #[test]
    fn xss_vectors_are_neutralised() {
        let vectors: &[&str] = &[
            "<script>alert(1)</script>",
            r#"<img src=x onerror="alert(1)">"#,
            "[click](javascript:alert(1))",
            r#"<svg/onload=alert(1)>"#,
            r#"<a href="data:text/html,<script>alert(1)</script>">x</a>"#,
            r#"<div style="x:expression(alert(1))">x</div>"#,
            r#"<iframe src="https://evil.test"></iframe>"#,
            r#"<object data="https://evil.test"></object>"#,
            "[click](javascript:void(0))",
            r#"<a href="javascript:alert(1)">x</a>"#,
        ];
        for input in vectors {
            let html = render_user_html(input);
            // No active script / framed-content tags survive into
            // the rendered HTML.
            assert!(
                !html.contains("<script") && !html.contains("<iframe") && !html.contains("<object"),
                "F6 regression — input {:?} produced {:?} which still has an active tag",
                input,
                html
            );
            // No javascript: URL survives in an *active* href context.
            // Active means inside a real `<a` tag (not escaped text);
            // the markdown sanitiser strips them.
            assert!(
                !html.contains("<a href=\"javascript:") && !html.contains("<a href='javascript:"),
                "F6 regression — input {:?} produced {:?} which still has a live javascript: href",
                input,
                html
            );
        }
    }

    /// F6: ensure mention-rendering doesn't accept arbitrary handles
    /// containing HTML-control characters.
    #[test]
    fn mentions_dont_pass_through_html_in_handles() {
        let html = render_user_html("hello @<script>alert(1)</script>");
        assert!(!html.contains("<script"));
    }
}
