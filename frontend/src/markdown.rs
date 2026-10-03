use crate::text::urlenc;
use ammonia::UrlRelative;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UrlUse {
    Link,
    Image,
}

pub fn safe_url(u: &str, use_type: UrlUse) -> bool {
    let mut clean = String::with_capacity(u.len());
    for c in u.trim_ascii().chars() {
        if c != '\t' && c != '\n' && c != '\r' {
            clean.push(c);
        }
    }

    let Some(colon_idx) = clean.find(':') else {
        return true;
    };

    let before_colon = &clean[..colon_idx];

    let first_slash_q_hash = clean.find(['/', '?', '#']);
    if let Some(idx) = first_slash_q_hash
        && idx < colon_idx
    {
        return true;
    }

    let mut chars = before_colon.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {}
        _ => return true,
    }
    for c in chars {
        if !c.is_ascii_alphanumeric() && c != '+' && c != '.' && c != '-' {
            return true;
        }
    }

    let scheme = before_colon.to_ascii_lowercase();
    match use_type {
        UrlUse::Link => matches!(scheme.as_str(), "http" | "https" | "mailto"),
        UrlUse::Image => matches!(scheme.as_str(), "http" | "https"),
    }
}

pub fn svg_data_url(svg: &str) -> String {
    format!("data:image/svg+xml;charset=utf-8,{}", urlenc(svg))
}

fn sanitizer() -> ammonia::Builder<'static> {
    let mut b = ammonia::Builder::default();
    b.url_schemes(["http", "https", "mailto"].into_iter().collect());
    b.url_relative(UrlRelative::PassThrough);
    b.link_rel(Some("noopener noreferrer"));
    b.add_tag_attributes("code", &["class"]);
    b.add_tag_attributes("div", &["class"]);
    b.add_tag_attributes("th", &["style"]);
    b.add_tag_attributes("td", &["style"]);
    b.attribute_filter(|element, attribute, value| {
        if element == "img" && attribute == "src" {
            if safe_url(value, UrlUse::Image) {
                Some(value.into())
            } else {
                None
            }
        } else if element == "code" && attribute == "class" {
            if value.starts_with("language-") {
                Some(value.into())
            } else {
                None
            }
        } else if element == "div" && attribute == "class" {
            (value == "markdown-table").then(|| value.into())
        } else if matches!(element, "th" | "td") && attribute == "style" {
            matches!(
                value,
                "text-align: left" | "text-align: center" | "text-align: right"
            )
            .then(|| value.into())
        } else {
            Some(value.into())
        }
    });
    b
}

thread_local! {
    static SANITIZER: ammonia::Builder<'static> = sanitizer();
}

pub fn render(md: &str) -> String {
    use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
    let mut html = String::new();
    let parser = Parser::new_ext(md, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH);
    let events = parser.flat_map(|event| {
        let pair = match event {
            Event::Start(Tag::Table(_)) => [
                Some(Event::Html("<div class=\"markdown-table\">".into())),
                Some(event),
            ],
            Event::End(TagEnd::Table) => [Some(event), Some(Event::Html("</div>".into()))],
            _ => [Some(event), None],
        };
        pair.into_iter().flatten()
    });
    pulldown_cmark::html::push_html(&mut html, events);
    SANITIZER.with(|b| b.clean(&html).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_preserve_structure_alignment_and_inline_formatting() {
        let html = render(
            "| Name | Count | State |\n| :--- | ---: | :---: |\n| `file` | 2 | **ready** |\n",
        );
        assert!(html.contains("<div class=\"markdown-table\"><table>"));
        assert!(html.contains("<th style=\"text-align: right\">Count</th>"));
        assert!(html.contains("<td style=\"text-align: center\"><strong>ready</strong></td>"));
        assert!(html.contains("<code>file</code>"));
        assert!(html.replace('\n', "").contains("</table></div>"));
        assert!(render("~~removed~~").contains("<del>removed</del>"));
        // Table parsing must not alter literal examples inside fenced code.
        assert!(!render("```\n| A | B |\n| --- | --- |\n```").contains("<table>"));
    }

    #[test]
    fn table_cells_cannot_introduce_arbitrary_styles_or_scripts() {
        let html = render(
            "| Value |\n| --- |\n| <img src=x onerror=alert(1)> |\n\n<table><tr><td style=\"position:fixed\" onclick=\"alert(1)\">unsafe</td></tr></table>",
        );
        assert!(html.contains("<table>"));
        assert!(!html.contains("onerror"));
        assert!(!html.contains("onclick"));
        assert!(!html.contains("position:fixed"));
    }

    #[test]
    fn test_safe_url() {
        assert!(safe_url("http://example.com", UrlUse::Link));
        assert!(safe_url("https://example.com", UrlUse::Link));
        assert!(safe_url("mailto:a@b.com", UrlUse::Link));
        assert!(!safe_url("mailto:a@b.com", UrlUse::Image));
        assert!(safe_url("./rel.md", UrlUse::Link));
        assert!(safe_url("#anchor", UrlUse::Link));
        assert!(!safe_url("javascript:alert(1)", UrlUse::Link));
        assert!(!safe_url("JaVaScRiPt:alert(1)", UrlUse::Link));
        assert!(!safe_url("java\tscript:alert(1)", UrlUse::Link));
        assert!(!safe_url("data:image/png;base64,x", UrlUse::Image));
        assert!(!safe_url("vbscript:x", UrlUse::Link));
    }

    #[test]
    fn test_svg_data_url() {
        let svg = "<svg onload=x>";
        let url = svg_data_url(svg);
        assert!(!url.contains('<'));
        assert!(!url.contains('>'));
        assert!(!url.contains('"'));
        assert!(url.starts_with("data:image/svg+xml;charset=utf-8,"));
    }

    #[test]
    fn test_render_preserves_safe() {
        assert!(render("[link](https://example.com)").contains("href=\"https://example.com\""));
        assert!(render("[link](./rel.md)").contains("href=\"./rel.md\""));
        assert!(render("[a](#anchor)").contains("href=\"#anchor\""));
        assert!(render("<a href=\"mailto:a@b.com\">M</a>").contains("href=\"mailto:a@b.com\""));
        assert!(render("**bold**").contains("<strong>bold</strong>"));
        assert!(render("- A").contains("<li>A</li>"));
        assert!(
            render("<details><summary>S</summary>body</details>")
                .contains("<details><summary>S</summary>body</details>")
        );
        assert!(render("<kbd>Ctrl</kbd>").contains("<kbd>Ctrl</kbd>"));
        assert!(render("H<sub>2</sub>O").contains("H<sub>2</sub>O"));
        assert!(render("a<br>b").contains("<br>"));

        let code_html = render("```rust\nfn main() {}\n```");
        assert!(code_html.contains("class=\"language-rust\""));

        // escaped script in code fence
        let script_code = render("```\n<script>\n```");
        assert!(script_code.contains("&lt;script&gt;"));
    }

    #[test]
    fn test_render_strips_unsafe() {
        assert!(!render("<script>alert(1)</script>").contains("<script>"));
        assert!(!render("<style>body{color:red}</style>").contains("<style>"));
        assert!(!render("<svg><rect/></svg>").contains("<svg>"));
        assert!(!render("<iframe src=\"\"></iframe>").contains("<iframe"));
        assert!(!render("<img src=\"x\" onerror=\"alert(1)\">").contains("onerror"));
        assert!(!render("<body onload=\"alert(1)\">").contains("onload"));

        assert!(!render("<a href=\"javascript:alert(1)\">x</a>").contains("href=\"javascript"));
        assert!(!render("<a href=\"JaVaScRiPt:alert(1)\">x</a>").contains("href="));
        assert!(!render("<a href=\"java\tscript:alert(1)\">x</a>").contains("href="));
        assert!(!render("<a href=\"&#106;avascript:alert(1)\">x</a>").contains("href="));
        assert!(!render("<javascript:alert(1)>").contains("href="));
        assert!(!render("![i](javascript:alert(1))").contains("src="));
        assert!(!render("![i](data:image/png;base64,x)").contains("src="));
        assert!(!render("<a href=\"vbscript:x\">").contains("href="));
        assert!(!render("<img src=\"mailto:x\">").contains("src="));
    }
}
