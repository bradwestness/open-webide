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

    let colon_idx = match clean.find(':') {
        Some(idx) => idx,
        None => return true,
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
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, pulldown_cmark::Parser::new(md));
    SANITIZER.with(|b| b.clean(&html).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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
