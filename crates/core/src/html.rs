//! Lightweight HTML to Markdown converter and sanitizer.
//!
//! Transforms web pages and documentation into token-efficient Markdown
//! while stripping scripts, styles, navigation, footers, and tracking elements.

/// Convert an HTML document into clean, readable Markdown bounded to `max_bytes`.
pub fn html_to_markdown(html: &str, max_bytes: usize) -> String {
    let mut out = String::with_capacity(html.len().min(max_bytes));
    let mut chars = html.chars().peekable();

    let mut in_pre = false;
    let mut skip_depth = 0;
    let mut skip_tag = "";
    let mut truncated = false;

    // Track active link href to format `[text](href)`
    let mut current_href: Option<String> = None;

    while let Some(c) = chars.next() {
        if c == '<' {
            // Read tag content until '>'
            let mut tag_content = String::new();
            let mut is_closing = false;

            if chars.peek() == Some(&'/') {
                is_closing = true;
                chars.next();
            } else if chars.peek() == Some(&'!') {
                // Check if comment `<!-- ... -->` or doctype
                let mut maybe_comment = String::new();
                while let Some(&nc) = chars.peek() {
                    maybe_comment.push(nc);
                    chars.next();
                    if maybe_comment.ends_with("-->")
                        || (nc == '>' && !maybe_comment.starts_with("--"))
                    {
                        break;
                    }
                }
                continue;
            }

            while let Some(&nc) = chars.peek() {
                chars.next();
                if nc == '>' {
                    break;
                }
                tag_content.push(nc);
            }

            let tag_content = tag_content.trim();
            let tag_name = tag_content
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_lowercase();
            let tag_name = tag_name.trim_end_matches('/');

            // Check elements to skip entirely
            let skippable = matches!(
                tag_name,
                "script" | "style" | "svg" | "noscript" | "nav" | "footer" | "header" | "aside"
            );

            if !is_closing && skippable {
                if skip_depth == 0 {
                    skip_tag = match tag_name {
                        "script" => "script",
                        "style" => "style",
                        "svg" => "svg",
                        "noscript" => "noscript",
                        "nav" => "nav",
                        "footer" => "footer",
                        "header" => "header",
                        "aside" => "aside",
                        _ => "",
                    };
                }
                if tag_name == skip_tag {
                    skip_depth += 1;
                }
                continue;
            }

            if is_closing && tag_name == skip_tag && skip_depth > 0 {
                skip_depth -= 1;
                if skip_depth == 0 {
                    skip_tag = "";
                }
                continue;
            }

            if skip_depth > 0 {
                continue;
            }

            // Handle standard tags
            match (is_closing, tag_name) {
                (false, "h1") => push_block_prefix(&mut out, "\n\n# "),
                (false, "h2") => push_block_prefix(&mut out, "\n\n## "),
                (false, "h3") => push_block_prefix(&mut out, "\n\n### "),
                (false, "h4") => push_block_prefix(&mut out, "\n\n#### "),
                (false, "h5") => push_block_prefix(&mut out, "\n\n##### "),
                (false, "h6") => push_block_prefix(&mut out, "\n\n###### "),
                (true, "h1" | "h2" | "h3" | "h4" | "h5" | "h6") => out.push('\n'),
                (false, "p" | "div" | "section" | "article")
                    if !out.is_empty() && !out.ends_with("\n\n") =>
                {
                    if out.ends_with('\n') {
                        out.push('\n');
                    } else {
                        out.push_str("\n\n");
                    }
                }
                (false, "br") => out.push('\n'),
                (false, "li") => push_block_prefix(&mut out, "\n- "),
                (true, "li") => {}
                (false, "blockquote") => push_block_prefix(&mut out, "\n> "),
                (false, "pre") => {
                    in_pre = true;
                    push_block_prefix(&mut out, "\n\n```\n");
                }
                (true, "pre") => {
                    in_pre = false;
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push_str("```\n\n");
                }
                (false, "code") if !in_pre => out.push('`'),
                (true, "code") if !in_pre => out.push('`'),
                (false, "strong" | "b") => out.push_str("**"),
                (true, "strong" | "b") => out.push_str("**"),
                (false, "em" | "i") => out.push('*'),
                (true, "em" | "i") => out.push('*'),
                (false, "a") => {
                    if let Some(href) = extract_attribute(tag_content, "href")
                        && !href.starts_with('#')
                        && !href.starts_with("javascript:")
                    {
                        current_href = Some(href);
                        out.push('[');
                    }
                }
                (true, "a") => {
                    if let Some(href) = current_href.take() {
                        out.push_str("](");
                        out.push_str(&href);
                        out.push(')');
                    }
                }
                (false, "hr") => push_block_prefix(&mut out, "\n\n---\n\n"),
                _ => {}
            }
        } else if skip_depth == 0 {
            if c == '&' {
                // Decode HTML entity
                let mut entity = String::new();
                while let Some(&nc) = chars.peek() {
                    chars.next();
                    if nc == ';' || entity.len() > 10 || nc.is_whitespace() {
                        if nc != ';' {
                            // Not a valid entity; emit verbatim
                            out.push('&');
                            out.push_str(&entity);
                            out.push(nc);
                            entity.clear();
                        }
                        break;
                    }
                    entity.push(nc);
                }
                if !entity.is_empty() {
                    decode_entity(&entity, &mut out);
                }
            } else if in_pre {
                out.push(c);
            } else if c.is_whitespace() {
                if !out.ends_with(' ') && !out.ends_with('\n') && !out.is_empty() {
                    out.push(' ');
                }
            } else {
                out.push(c);
            }
        }

        if out.len() >= max_bytes {
            truncated = true;
            break;
        }
    }

    // Clean up excessive whitespace
    let mut cleaned = collapse_newlines(&out);

    if truncated || cleaned.len() > max_bytes {
        if cleaned.len() > max_bytes {
            cleaned.truncate(max_bytes);
            while !cleaned.is_char_boundary(cleaned.len()) {
                cleaned.pop();
            }
        }
        cleaned.push_str("\n\n[Content truncated to 16KB limit...]");
        cleaned
    } else {
        cleaned
    }
}

fn push_block_prefix(out: &mut String, prefix: &str) {
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(prefix.trim_start_matches('\n'));
}

fn extract_attribute(tag: &str, attr: &str) -> Option<String> {
    let needle = format!("{attr}=");
    let idx = tag.find(&needle)?;
    let rest = &tag[idx + needle.len()..];
    let quote = rest.chars().next()?;
    if quote == '"' || quote == '\'' {
        let after_quote = &rest[1..];
        let end_idx = after_quote.find(quote)?;
        Some(after_quote[..end_idx].to_string())
    } else {
        let end_idx = rest.find(char::is_whitespace).unwrap_or(rest.len());
        Some(rest[..end_idx].to_string())
    }
}

fn decode_entity(entity: &str, out: &mut String) {
    match entity {
        "amp" => out.push('&'),
        "lt" => out.push('<'),
        "gt" => out.push('>'),
        "quot" => out.push('"'),
        "apos" | "#39" => out.push('\''),
        "nbsp" => out.push(' '),
        "copy" => out.push('©'),
        "mdash" => out.push('—'),
        "ndash" => out.push('–'),
        _ => {
            if let Some(rest) = entity.strip_prefix("#x")
                && let Ok(code) = u32::from_str_radix(rest, 16)
                && let Some(ch) = char::from_u32(code)
            {
                out.push(ch);
                return;
            } else if let Some(rest) = entity.strip_prefix('#')
                && let Ok(code) = rest.parse::<u32>()
                && let Some(ch) = char::from_u32(code)
            {
                out.push(ch);
                return;
            }
            out.push('&');
            out.push_str(entity);
            out.push(';');
        }
    }
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
        assert!(md.contains("[Content truncated to 16KB limit...]"));
    }
}
