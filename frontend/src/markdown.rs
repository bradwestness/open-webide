use crate::text::{escape_html, urlenc};
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
    b.add_tag_attributes("li", &["class"]);
    b.add_tag_attributes("tr", &["class"]);
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
            matches!(
                value,
                "markdown-table"
                    | "rich-preview-block unchanged"
                    | "rich-preview-block modified"
                    | "rich-preview-block added"
                    | "rich-preview-block removed"
            )
            .then(|| value.into())
        } else if matches!(element, "li" | "tr") && attribute == "class" {
            matches!(
                value,
                "rich-preview-item added"
                    | "rich-preview-item modified"
                    | "rich-preview-item removed"
                    | "rich-preview-row added"
                    | "rich-preview-row modified"
                    | "rich-preview-row removed"
            )
            .then(|| value.into())
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
    use pulldown_cmark::{Options, Parser};
    render_events(Parser::new_ext(
        md,
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH,
    ))
}

fn render_events<'a>(events: impl Iterator<Item = pulldown_cmark::Event<'a>>) -> String {
    let html = event_html(events);
    SANITIZER.with(|b| b.clean(&html).to_string())
}

fn event_html<'a>(events: impl Iterator<Item = pulldown_cmark::Event<'a>>) -> String {
    use pulldown_cmark::{Event, Tag, TagEnd};
    let mut html = String::new();
    let events = events.flat_map(|event| {
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
    html
}

/// Render prose changes inside the current document structure, with block gutters.
/// Lists and tables retain their containers while their children and words are compared.
pub fn render_diff(old: &str, new: &str) -> String {
    let html = diff_blocks(&rendered_blocks(old), &rendered_blocks(new), true, 0);
    SANITIZER.with(|b| b.clean(&html).to_string())
}

struct MarkdownBlock {
    html: String,
    events: Vec<pulldown_cmark::Event<'static>>,
}

fn diff_blocks(
    old: &[MarkdownBlock],
    new: &[MarkdownBlock],
    gutters: bool,
    depth: usize,
) -> String {
    use openwebide_core::diff::{LineOp, line_lcs};
    use std::fmt::Write;
    let mut identities = std::collections::HashMap::new();
    let mut encode = |blocks: &[MarkdownBlock]| {
        let mut text = String::new();
        for block in blocks {
            let next = identities.len();
            let id = identities.entry(block.html.clone()).or_insert(next);
            let _ = writeln!(text, "{id}");
        }
        text
    };
    let old_tokens = encode(old);
    let new_tokens = encode(new);
    let mut html = String::new();
    let mut current = 0;
    let mut previous = 0;
    let mut inserted = Vec::new();
    let mut deleted = Vec::new();
    for op in line_lcs(&old_tokens, &new_tokens) {
        match op {
            LineOp::Equal(_) => {
                render_changed_blocks(&mut html, &mut inserted, &mut deleted, gutters, depth);
                render_block(
                    &mut html,
                    &new[current].html,
                    "unchanged",
                    gutters,
                    &new[current],
                );
                current += 1;
                previous += 1;
            }
            LineOp::Delete(_) => {
                deleted.push(&old[previous]);
                previous += 1;
            }
            LineOp::Insert(_) => {
                inserted.push(&new[current]);
                current += 1;
            }
        }
    }
    render_changed_blocks(&mut html, &mut inserted, &mut deleted, gutters, depth);
    html
}

fn render_block(html: &mut String, block: &str, change: &str, gutters: bool, node: &MarkdownBlock) {
    use pulldown_cmark::{Event, Tag};
    use std::fmt::Write;
    let native = match node.events.first() {
        Some(Event::Start(Tag::Item)) => Some(("li", "rich-preview-item")),
        Some(Event::Start(Tag::TableRow | Tag::TableHead)) => Some(("tr", "rich-preview-row")),
        _ => None,
    };
    if let Some((tag, class)) = native {
        if gutters && change != "unchanged" {
            html.push_str(&block.replacen(
                &format!("<{tag}>"),
                &format!("<{tag} class=\"{class} {change}\">"),
                1,
            ));
        } else {
            html.push_str(block);
        }
        return;
    }
    if gutters {
        let _ = write!(
            html,
            "<div class=\"rich-preview-block {change}\" title=\"{change}\">{block}</div>"
        );
    } else {
        html.push_str(block);
    }
}

fn render_changed_blocks(
    html: &mut String,
    inserted: &mut Vec<&MarkdownBlock>,
    deleted: &mut Vec<&MarkdownBlock>,
    gutters: bool,
    depth: usize,
) {
    for index in 0..inserted.len().max(deleted.len()) {
        match (deleted.get(index), inserted.get(index)) {
            (Some(old), Some(new)) => {
                let change = if depth < 64 && compatible_container(old, new) {
                    "unchanged"
                } else {
                    "modified"
                };
                render_block(html, &diff_block(old, new, depth), change, gutters, new);
            }
            (Some(old), None) => {
                render_block(html, &mark_block(old, "del"), "removed", gutters, old);
            }
            (None, Some(new)) => render_block(html, &mark_block(new, "ins"), "added", gutters, new),
            (None, None) => {}
        }
    }
    inserted.clear();
    deleted.clear();
}

fn compatible_container(old: &MarkdownBlock, new: &MarkdownBlock) -> bool {
    use pulldown_cmark::{Event, Tag};
    matches!(
        new.events.first(),
        Some(Event::Start(Tag::List(_) | Tag::Table(_)))
    ) && old.events.first() == new.events.first()
        && old.events.last() == new.events.last()
}

fn diff_block(old: &MarkdownBlock, new: &MarkdownBlock, depth: usize) -> String {
    use pulldown_cmark::Event;
    match (old.events.first(), new.events.first()) {
        (Some(Event::Start(a)), Some(Event::Start(b)))
            if a == b
                && old.events.last() == new.events.last()
                && depth < 64
                && !matches!(a, pulldown_cmark::Tag::Image { .. }) =>
        {
            // Recurse inside compatible containers instead of replacing a whole list,
            // table, paragraph or emphasis span when just one child has changed.
            let old_children = event_blocks(old.events[1..old.events.len() - 1].iter().cloned());
            let new_children = event_blocks(new.events[1..new.events.len() - 1].iter().cloned());
            let inner = diff_blocks(
                &old_children,
                &new_children,
                compatible_container(old, new),
                depth + 1,
            );
            event_html(
                [
                    new.events[0].clone(),
                    Event::Html(inner.into()),
                    new.events.last().unwrap().clone(),
                ]
                .into_iter(),
            )
        }
        (Some(Event::Text(a)), Some(Event::Text(b))) => inline_words(a, b),
        (Some(Event::Code(a)), Some(Event::Code(b))) => {
            format!("<code>{}</code>", inline_words(a, b))
        }
        _ => format!("{}{}", mark_block(old, "del"), mark_block(new, "ins")),
    }
}

fn inline_words(old: &str, new: &str) -> String {
    use openwebide_core::diff::{DiffChunk, compute_word_diff};
    use std::collections::VecDeque;
    let (before, after) = compute_word_diff(old, new);
    let mut before: VecDeque<_> = before.into();
    let mut after: VecDeque<_> = after.into();
    let mut html = String::new();
    while !before.is_empty() || !after.is_empty() {
        if matches!(before.front(), Some(DiffChunk::Deleted(_))) {
            if let Some(DiffChunk::Deleted(text)) = before.pop_front() {
                html.push_str(&format!("<del>{}</del>", escape_html(&text)));
            }
        } else if matches!(after.front(), Some(DiffChunk::Inserted(_))) {
            if let Some(DiffChunk::Inserted(text)) = after.pop_front() {
                html.push_str(&format!("<ins>{}</ins>", escape_html(&text)));
            }
        } else if let (Some(DiffChunk::Unchanged(a)), Some(DiffChunk::Unchanged(b))) =
            (before.pop_front(), after.pop_front())
        {
            let length = a.len().min(b.len());
            debug_assert_eq!(&a[..length], &b[..length]);
            html.push_str(&escape_html(&a[..length]));
            if length < a.len() {
                before.push_front(DiffChunk::Unchanged(a[length..].into()));
            }
            if length < b.len() {
                after.push_front(DiffChunk::Unchanged(b[length..].into()));
            }
        }
    }
    html
}

fn mark_block(block: &MarkdownBlock, marker: &str) -> String {
    use pulldown_cmark::Event;
    if matches!(
        block.events.first(),
        Some(Event::Start(pulldown_cmark::Tag::Image { .. }))
    ) {
        return format!("<{marker}>{}</{marker}>", block.html);
    }
    let mut events = Vec::new();
    let mut text = String::new();
    let flush = |events: &mut Vec<Event<'static>>, text: &mut String| {
        if !text.is_empty() {
            events.push(Event::Html(
                format!("<{marker}>{}</{marker}>", escape_html(text)).into(),
            ));
            text.clear();
        }
    };
    for event in block.events.iter().cloned() {
        match event {
            Event::Text(value) => text.push_str(&value),
            // Markdown source wraps become visible spaces. Keep them inside the
            // same highlight as adjacent text rather than leaving unmarked gaps.
            Event::SoftBreak => text.push('\n'),
            event => {
                flush(&mut events, &mut text);
                events.push(match event {
                    Event::Code(value) => Event::Html(
                        format!("<code><{marker}>{}</{marker}></code>", escape_html(&value)).into(),
                    ),
                    _ => event,
                });
            }
        }
    }
    flush(&mut events, &mut text);
    event_html(events.into_iter())
}

fn rendered_blocks(md: &str) -> Vec<MarkdownBlock> {
    use pulldown_cmark::{Options, Parser};
    // Parse once so reference links retain full-document context.
    event_blocks(
        Parser::new_ext(md, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH)
            .map(pulldown_cmark::Event::into_static),
    )
}

fn event_blocks(
    events: impl Iterator<Item = pulldown_cmark::Event<'static>>,
) -> Vec<MarkdownBlock> {
    use pulldown_cmark::Event;
    let mut blocks = Vec::new();
    let mut current = Vec::new();
    let mut depth = 0usize;
    for event in events {
        match &event {
            Event::Start(_) => depth += 1,
            Event::End(_) => depth = depth.saturating_sub(1),
            _ => {}
        }
        current.push(event);
        if depth == 0 {
            let html = event_html(current.iter().cloned());
            if !html.is_empty() {
                blocks.push(MarkdownBlock {
                    html,
                    events: std::mem::take(&mut current),
                });
            } else {
                current.clear();
            }
        }
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn added_and_removed_source_wraps_stay_inside_highlights() {
        let markdown = "## Added section\n\n- First line\n  continues with **bold** text\n  and [a link](https://example.com).\n\nWrapped paragraph\ncontinues here.\n";
        for (old, new, marker) in [("", markdown, "ins"), (markdown, "", "del")] {
            let html = render_diff(old, new);
            assert!(
                html.contains(&format!("<{marker}>First line\ncontinues with </{marker}>")),
                "{html}"
            );
            assert!(
                html.contains(&format!("<{marker}> text\nand </{marker}>")),
                "{html}"
            );
            assert!(
                html.contains(&format!(
                    "<{marker}>Wrapped paragraph\ncontinues here.</{marker}>"
                )),
                "{html}"
            );
            assert!(html.contains(&format!("<strong><{marker}>bold</{marker}></strong>")));
            assert!(
                !html.contains(&format!("</{marker}>\n<{marker}>")),
                "{html}"
            );
        }
    }

    #[test]
    fn one_word_in_changelog_keeps_the_section_and_list_once() {
        let old = "## Changed\n\n- Keep this item.\n- Make previews **clear** with [links](https://example.com).\n- Keep this too.\n";
        let new = old.replace("clear", "compact");
        let html = render_diff(old, &new);
        assert_eq!(html.matches("<h2>").count(), 1, "{html}");
        assert_eq!(html.matches("<ul>").count(), 1, "{html}");
        assert_eq!(html.matches("<li").count(), 3, "{html}");
        assert_eq!(html.matches("Keep this item.").count(), 1);
        assert_eq!(
            html.matches("rich-preview-item modified").count(),
            1,
            "{html}"
        );
        assert!(
            !html.contains("rich-preview-block modified"),
            "The list container must not own the gutter: {html}"
        );
        assert!(
            html.contains("<strong><del>clear</del><ins>compact</ins></strong>"),
            "{html}"
        );
        assert!(
            html.contains(" with <a href=\"https://example.com\""),
            "{html}"
        );
    }

    #[test]
    fn changed_table_cell_and_nested_list_preserve_structure() {
        let old = "| Name | Value |\n| --- | --- |\n| Keep | old value |\n\n- Parent\n  - old child\n  - unchanged\n";
        let new = old
            .replace("old value", "new value")
            .replace("old child", "new child");
        let html = render_diff(old, &new);
        assert_eq!(html.matches("<table>").count(), 1, "{html}");
        assert_eq!(html.matches("<tr").count(), 2, "{html}");
        assert_eq!(html.matches("<ul>").count(), 2, "{html}");
        assert!(
            html.contains("<del>old</del><ins>new</ins> value"),
            "{html}"
        );
        assert!(
            html.contains("<del>old</del><ins>new</ins> child"),
            "{html}"
        );
        let header = render_diff(old, &old.replace("Name", "Label"));
        assert_eq!(header.matches("<table>").count(), 1, "{header}");
        assert_eq!(
            header.matches("rich-preview-row modified").count(),
            1,
            "{header}"
        );
        assert!(
            header.contains("<th><del>Name</del><ins>Label</ins></th>"),
            "{header}"
        );
    }

    #[test]
    fn inline_word_changes_preserve_unicode_whitespace_and_escape_html() {
        assert_eq!(
            inline_words("café 😀 old end", "café 😀 new end"),
            "café 😀 <del>old</del><ins>new</ins> end"
        );
        assert_eq!(inline_words("a b", "a new b"), "a <ins>new </ins>b");
        assert_eq!(inline_words("a old b", "a b"), "a <del>old </del>b");
        assert_eq!(
            inline_words("<old>", "<new>"),
            "&lt;<del>old</del><ins>new</ins>&gt;"
        );
    }

    #[test]
    fn rich_preview_marks_blocks_and_preserves_deleted_text() {
        let html = render_diff(
            "# Keep\n\nOld text\n\nRemove me\n\nEnd\n",
            "# Keep\n\nNew text\n\nEnd\n\nAdded\n",
        );
        assert!(html.contains("rich-preview-block modified"));
        assert!(html.contains("rich-preview-block added"), "{html}");
        assert!(html.contains("rich-preview-block removed"));
        assert!(html.contains("Remove me"));
        assert!(html.contains("<del>"));
        assert!(html.contains("<h1>Keep</h1>"));
        assert!(html.contains("<p>End</p>"));
        assert!(!render_diff("same\n", "same\n").contains("<del>"));
        assert!(render_diff("last\n", "").contains("rich-preview-block removed"));
    }

    #[test]
    fn rich_preview_keeps_structured_markdown_and_reference_links() {
        let text = "- One\n- Two\n\n| Name |\n| --- |\n| Value |\n\n[link][ref]\n\n[ref]: https://example.com\n\n```rs\nlet x = 1;\n```\n\n<script>alert(1)</script>\n";
        let html = render_diff("", text);
        assert!(html.contains("<ul>"));
        assert!(html.contains("markdown-table"));
        assert!(html.contains("href=\"https://example.com\""));
        assert!(html.contains("language-rs"));
        assert!(!html.contains("<script>"));
    }

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
