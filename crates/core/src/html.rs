//! Lightweight HTML to Markdown converter and sanitizer.
//!
//! Transforms web pages and documentation into token-efficient Markdown
//! while stripping scripts, styles, navigation, footers, and tracking elements.
//!
//! Built on the `html5ever` tokenizer: HTML5 entities are fully decoded, and
//! stray `<` or `&` characters that do not start a tag or entity are passed
//! through as literal text.

use std::cell::RefCell;

use html5ever::tokenizer::{
    BufferQueue, Tag, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};

/// Convert an HTML document into clean, readable Markdown bounded to `max_bytes`.
pub fn html_to_markdown(html: &str, max_bytes: usize) -> String {
    let sink = MarkdownSink {
        state: RefCell::new(SinkState {
            out: String::with_capacity(html.len().min(max_bytes)),
            in_pre: false,
            skip_depth: 0,
            skip_tag: "",
            current_href: None,
            first_cell: true,
        }),
    };

    let input = BufferQueue::default();
    input.push_back(html.into());
    let tok = Tokenizer::new(sink, TokenizerOpts::default());
    let _ = tok.feed(&input);
    tok.end();

    let out = tok.sink.state.into_inner().out;

    // Clean up excessive whitespace
    let mut cleaned = collapse_newlines(&out);

    if cleaned.len() > max_bytes {
        let cut = cleaned.floor_char_boundary(max_bytes);
        cleaned.truncate(cut);
        cleaned.push_str(&format!("\n\n[Content truncated to {max_bytes} bytes...]"));
    }
    cleaned
}

struct SinkState {
    out: String,
    in_pre: bool,
    skip_depth: u32,
    skip_tag: &'static str,
    current_href: Option<String>,
    first_cell: bool,
}

struct MarkdownSink {
    state: RefCell<SinkState>,
}

impl TokenSink for MarkdownSink {
    type Handle = ();

    fn process_token(&self, token: Token, _line_number: u64) -> TokenSinkResult<()> {
        let mut st = self.state.borrow_mut();
        match token {
            Token::CharacterTokens(text) => {
                for c in text.chars() {
                    handle_char(&mut st, c);
                }
            }
            Token::TagToken(tag) => handle_tag(&mut st, &tag),
            _ => {} // comments, doctype, parse errors: ignored
        }
        TokenSinkResult::Continue
    }
}

fn handle_char(st: &mut SinkState, c: char) {
    if st.skip_depth > 0 {
        return;
    }
    if st.in_pre {
        st.out.push(c);
    } else if c.is_whitespace() {
        if !st.out.ends_with(' ') && !st.out.ends_with('\n') && !st.out.is_empty() {
            st.out.push(' ');
        }
    } else {
        st.out.push(c);
    }
}

fn handle_tag(st: &mut SinkState, tag: &Tag) {
    let name: &str = &tag.name;

    if tag.kind == TagKind::StartTag {
        if is_skippable(name) {
            // A self-closing skip tag (`<svg/>`) has no content and no
            // matching end tag, so it must not enter skip mode.
            if tag.self_closing {
                return;
            }
            if st.skip_depth == 0 {
                st.skip_tag = match name {
                    "script" => "script",
                    "style" => "style",
                    "svg" => "svg",
                    "noscript" => "noscript",
                    "head" => "head",
                    "nav" => "nav",
                    "footer" => "footer",
                    "header" => "header",
                    "aside" => "aside",
                    _ => "",
                };
            }
            if name == st.skip_tag {
                st.skip_depth += 1;
            }
            return;
        }

        if st.skip_depth > 0 {
            return;
        }

        match name {
            "h1" => push_block_prefix(&mut st.out, "\n\n# "),
            "h2" => push_block_prefix(&mut st.out, "\n\n## "),
            "h3" => push_block_prefix(&mut st.out, "\n\n### "),
            "h4" => push_block_prefix(&mut st.out, "\n\n#### "),
            "h5" => push_block_prefix(&mut st.out, "\n\n##### "),
            "h6" => push_block_prefix(&mut st.out, "\n\n###### "),
            "p" | "div" | "section" | "article"
                if !st.out.is_empty() && !st.out.ends_with("\n\n") =>
            {
                if st.out.ends_with('\n') {
                    st.out.push('\n');
                } else {
                    st.out.push_str("\n\n");
                }
            }
            "br" => st.out.push('\n'),
            "li" => push_block_prefix(&mut st.out, "\n- "),
            "blockquote" => push_block_prefix(&mut st.out, "\n> "),
            "pre" => {
                st.in_pre = true;
                push_block_prefix(&mut st.out, "\n\n```\n");
            }
            "code" if !st.in_pre => st.out.push('`'),
            "strong" | "b" => st.out.push_str("**"),
            "em" | "i" => st.out.push('*'),
            "a" => {
                if let Some(href) = tag
                    .attrs
                    .iter()
                    .find(|attr| &attr.name.local == "href")
                    .map(|attr| attr.value.as_ref().to_string())
                    && href_is_allowed(&href)
                {
                    st.current_href = Some(href);
                    st.out.push('[');
                }
            }
            "hr" => push_block_prefix(&mut st.out, "\n\n---\n\n"),
            "tr" => {
                push_block_prefix(&mut st.out, "\n");
                st.first_cell = true;
            }
            "td" | "th" => {
                if !st.first_cell {
                    st.out.push_str(" | ");
                }
                st.first_cell = false;
            }
            _ => {}
        }
        return;
    }

    // End tag
    if name == st.skip_tag && st.skip_depth > 0 {
        st.skip_depth -= 1;
        if st.skip_depth == 0 {
            st.skip_tag = "";
        }
        return;
    }
    if st.skip_depth > 0 {
        return;
    }

    match name {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => st.out.push('\n'),
        "pre" => {
            st.in_pre = false;
            if !st.out.ends_with('\n') {
                st.out.push('\n');
            }
            st.out.push_str("```\n\n");
        }
        "code" if !st.in_pre => st.out.push('`'),
        "strong" | "b" => st.out.push_str("**"),
        "em" | "i" => st.out.push('*'),
        "a" => {
            if let Some(href) = st.current_href.take() {
                st.out.push_str("](");
                st.out.push_str(&href);
                st.out.push(')');
            }
        }
        _ => {}
    }
}

fn is_skippable(name: &str) -> bool {
    matches!(
        name,
        "script" | "style" | "svg" | "noscript" | "head" | "nav" | "footer" | "header" | "aside"
    )
}

/// Scheme-less (relative/fragment) hrefs are allowed; an explicit scheme
/// must be http/https/mailto. Checked on a trimmed, lowercased copy.
fn href_is_allowed(href: &str) -> bool {
    let lower = href.trim().to_ascii_lowercase();
    match lower.find([':', '/', '?', '#']) {
        Some(i) if lower.as_bytes()[i] == b':' => {
            matches!(&lower[..i], "http" | "https" | "mailto")
        }
        _ => true, // no scheme before any path/query/fragment marker
    }
}

fn push_block_prefix(out: &mut String, prefix: &str) {
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(prefix.trim_start_matches('\n'));
}

fn collapse_newlines(s: &str) -> String {
    let mut res = String::with_capacity(s.len());
    let mut consecutive_newlines = 0;

    for line in s.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            consecutive_newlines += 1;
            if consecutive_newlines <= 2 {
                res.push('\n');
            }
        } else {
            if !res.is_empty() && !res.ends_with('\n') {
                res.push('\n');
            }
            consecutive_newlines = 0;
            res.push_str(trimmed);
        }
    }
    res.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strips_scripts_and_styles() {
        let html = r#"
            <html>
            <head>
                <style>body { color: red; }</style>
                <script>console.log("bad");</script>
            </head>
            <body>
                <h1>Hello World</h1>
                <p>Paragraph text.</p>
            </body>
            </html>
        "#;
        let md = html_to_markdown(html, 4096);
        assert!(!md.contains("color: red"));
        assert!(!md.contains("console.log"));
        assert!(md.contains("# Hello World"));
        assert!(md.contains("Paragraph text."));
    }

    #[test]
    fn test_strips_nav_and_footer() {
        let html = r#"
            <nav><a href="/menu">Nav Item</a></nav>
            <main>
                <p>Main content.</p>
            </main>
            <footer><p>Copyright 2026</p></footer>
        "#;
        let md = html_to_markdown(html, 4096);
        assert!(!md.contains("Nav Item"));
        assert!(!md.contains("Copyright 2026"));
        assert!(md.contains("Main content."));
    }

    #[test]
    fn test_formats_links_and_code() {
        let html = r#"
            <p>Visit <a href="https://rust-lang.org">Rust</a> for <code>fast</code> code.</p>
            <pre><code>fn main() {
    println!("hi");
}</code></pre>
        "#;
        let md = html_to_markdown(html, 4096);
        assert!(md.contains("[Rust](https://rust-lang.org)"));
        assert!(md.contains("`fast`"));
        assert!(md.contains("```\nfn main() {\n    println!(\"hi\");\n}\n```"));
    }

    #[test]
    fn test_decodes_entities() {
        let html = "<p>Rust &amp; C++ &gt; Python &quot;rocks&quot; &#39;quote&#39;</p>";
        let md = html_to_markdown(html, 4096);
        assert_eq!(md, "Rust & C++ > Python \"rocks\" 'quote'");
    }

    #[test]
    fn test_bounds_length() {
        let html = "<p>".to_string() + &"a".repeat(20000) + "</p>";
        let md = html_to_markdown(&html, 1000);
        assert!(md.len() <= 1100);
        assert!(md.contains("[Content truncated to 1000 bytes...]"));
    }

    #[test]
    fn truncation_inside_multibyte_char() {
        let html = "<p>".to_string() + &"a".repeat(999) + "€" + "</p>";
        let md = html_to_markdown(&html, 1000);
        assert!(md.len() <= 1100);
        assert!(md.contains("1000 bytes"));
    }

    #[test]
    fn test_self_closing_skip_tag() {
        let html = "<p>Intro</p><svg viewBox='0 0 1 1'/><p>Body text</p>";
        let md = html_to_markdown(html, 4096);
        assert!(md.contains("Intro"));
        assert!(md.contains("Body text"));
    }

    #[test]
    fn test_ampersand_in_text() {
        let html = "<p>Q&A</p><p>Next</p>";
        let md = html_to_markdown(html, 4096);
        assert!(md.contains("Q&A"));
        assert!(md.contains("Next"));
    }

    #[test]
    fn test_trailing_ampersand() {
        let html = "<p>AT&T</p>";
        let md = html_to_markdown(html, 4096);
        assert_eq!(md, "AT&T");
    }

    #[test]
    fn test_unescaped_less_than() {
        let html = "<p>a < b and c</p>";
        let md = html_to_markdown(html, 4096);
        assert_eq!(md, "a < b and c");
    }

    #[test]
    fn test_drops_unsafe_href_schemes() {
        let html = "<p><a href=\"javascript:alert(1)\">Click</a> and <a href=\"data:text/html,x\">Bad</a></p>";
        let md = html_to_markdown(html, 4096);
        assert!(md.contains("Click"));
        assert!(md.contains("Bad"));
        assert!(!md.contains("javascript:"));
        assert!(!md.contains("data:"));
        assert!(!md.contains('['));
    }

    #[test]
    fn test_keeps_relative_href() {
        let html = "<p><a href=\"/docs\">Docs</a></p>";
        let md = html_to_markdown(html, 4096);
        assert_eq!(md, "[Docs](/docs)");
    }

    #[test]
    fn test_data_href_not_treated_as_href() {
        let html = "<p><a data-href=\"/evil\" href=\"/ok\">text</a></p>";
        let md = html_to_markdown(html, 4096);
        assert_eq!(md, "[text](/ok)");
    }

    #[test]
    fn test_script_content_absent() {
        let html = "<p>Before</p><script>x</script><p>After</p>";
        let md = html_to_markdown(html, 4096);
        assert!(!md.contains("x"));
        assert!(md.contains("Before"));
        assert!(md.contains("After"));
    }

    #[test]
    fn test_head_content_not_leaked() {
        let html = "<html><head><title>My Title</title></head><body><p>Content</p></body></html>";
        let md = html_to_markdown(html, 4096);
        assert!(!md.contains("My Title"));
        assert!(md.contains("Content"));
    }

    #[test]
    fn test_table_as_text_rows() {
        let html = "<table><tr><td>a</td><td>b</td></tr><tr><td>c</td><td>d</td></tr></table>";
        let md = html_to_markdown(html, 4096);
        assert!(md.contains("a | b"));
        assert!(md.contains("c | d"));
    }
}
