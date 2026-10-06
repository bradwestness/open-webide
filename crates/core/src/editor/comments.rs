//! Comment commands use language syntax and logical-line selections.
use super::{
    Document, Edit, EditError, Selection,
    lines::{lines, row_at, selected_rows},
};
use crate::highlight::Language;

pub fn line_comment(language: Language) -> Option<&'static str> {
    match language {
        Language::Rust
        | Language::JavaScript
        | Language::TypeScript
        | Language::Jsx
        | Language::Tsx
        | Language::Java
        | Language::CSharp
        | Language::Php
        | Language::C
        | Language::Cpp
        | Language::Go => Some("//"),
        Language::Python | Language::Shell | Language::Toml | Language::Yaml => Some("#"),
        Language::Sql => Some("--"),
        _ => None,
    }
}
pub fn block_comment(language: Language) -> Option<(&'static str, &'static str)> {
    match language {
        Language::Rust
        | Language::JavaScript
        | Language::TypeScript
        | Language::Jsx
        | Language::Tsx
        | Language::Java
        | Language::CSharp
        | Language::Php
        | Language::C
        | Language::Cpp
        | Language::Go
        | Language::Css
        | Language::Sql => Some(("/*", "*/")),
        Language::Html | Language::Markdown => Some(("<!--", "-->")),
        _ => None,
    }
}
fn balanced_comments(text: &str) -> bool {
    let mut depth = 0;
    let mut position = 0;
    while position < text.len() {
        if text[position..].starts_with("/*") {
            depth += 1;
            position += 2;
        } else if text[position..].starts_with("*/") {
            if depth == 0 {
                return false;
            }
            depth -= 1;
            position += 2;
        } else {
            position += text[position..].chars().next().unwrap().len_utf8();
        }
    }
    depth == 0
}
impl Document {
    /// Unsupported syntaxes leave text unchanged rather than introduce invalid comments.
    pub fn toggle_line_comments(&mut self, language: Language) -> Result<bool, EditError> {
        let Some(marker) = line_comment(language) else {
            return self.toggle_block_comments(language);
        };
        let rows = lines(&self.text);
        let selected: Vec<_> = selected_rows(&rows, &self.selections)
            .into_iter()
            .flatten()
            .filter_map(|row| {
                let line = &rows[row];
                let text = &self.text[line.start..line.body_end];
                let prefix = text.len() - text.trim_start_matches([' ', '\t']).len();
                (!text.trim().is_empty()
                    || self
                        .selections
                        .iter()
                        .all(|selection| selection.range().is_empty()))
                .then_some((line.start + prefix, line.body_end))
            })
            .collect();
        let remove = !selected.is_empty()
            && selected
                .iter()
                .all(|&(start, end)| self.text[start..end].starts_with(marker));
        let edits = selected
            .into_iter()
            .map(|(start, end)| {
                if remove {
                    let mut stop = start + marker.len();
                    if stop < end && self.text.as_bytes()[stop] == b' ' {
                        stop += 1;
                    }
                    Edit::replace(start..stop, "")
                } else {
                    Edit::replace(start..start, format!("{marker} "))
                }
            })
            .collect();
        self.apply_mapped(edits)
    }
    pub fn toggle_block_comments(&mut self, language: Language) -> Result<bool, EditError> {
        let Some((open, close)) = block_comment(language) else {
            return Ok(false);
        };
        let rows = lines(&self.text);
        let mut changes = Vec::new();
        for selection in &self.selections {
            let mut range = selection.range();
            if range.is_empty() {
                let row = &rows[row_at(&rows, selection.head)];
                let body = &self.text[row.start..row.body_end];
                range = row.start + body.len() - body.trim_start_matches([' ', '\t']).len()
                    ..row.body_end;
            }
            if range.start > open.len()
                && self.text[..range.start].ends_with(&format!("{open} "))
                && self.text[range.end..].starts_with(&format!(" {close}"))
            {
                range.start -= open.len() + 1;
                range.end += close.len() + 1;
            }
            let selected = &self.text[range.clone()];
            let (text, anchor, head) = if selected.starts_with(open)
                && selected.ends_with(close)
                && selected.len() >= open.len() + close.len()
            {
                let inside = &selected[open.len()..selected.len() - close.len()];
                let inside = inside.strip_prefix(' ').unwrap_or(inside);
                let inside = inside.strip_suffix(' ').unwrap_or(inside);
                (inside.to_string(), 0, inside.len())
            } else {
                // A closer in the selection would terminate the outer comment early.
                // Rust permits nested comments; other languages must leave it alone.
                if (language == Language::Rust && !balanced_comments(selected))
                    || (language != Language::Rust && selected.contains(close))
                {
                    return Ok(false);
                }
                (
                    format!("{open} {selected} {close}"),
                    open.len() + 1,
                    open.len() + 1 + selected.len(),
                )
            };
            let after = if selection.range().is_empty() {
                Selection::caret(text.len())
            } else if selection.anchor <= selection.head {
                Selection { anchor, head }
            } else {
                Selection {
                    anchor: head,
                    head: anchor,
                }
            };
            changes.push((Edit::replace(range, text), after));
        }
        self.apply_caret_edits(changes)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn line_comments_toggle_mixed_indentation_blank_lines_and_reversed_crlf_selection() {
        let original = "  x\r\n\ty\r\n\r\nz";
        let mut doc = Document::new(original);
        doc.set_selections(vec![Selection {
            anchor: 11,
            head: 0,
        }])
        .unwrap();
        doc.toggle_line_comments(Language::Rust).unwrap();
        assert_eq!(doc.text(), "  // x\r\n\t// y\r\n\r\nz");
        doc.toggle_line_comments(Language::Rust).unwrap();
        assert_eq!(doc.text(), original);
        doc.undo();
        assert!(doc.text().contains("// x"));
        let mut json = Document::new("{\"x\":1}");
        assert!(!json.toggle_line_comments(Language::Json).unwrap());
    }
    #[test]
    fn blank_line_comments_work_and_rust_nesting_must_be_balanced() {
        let mut doc = Document::new("  ");
        doc.set_selections(vec![Selection::caret(2)]).unwrap();
        doc.toggle_line_comments(Language::Rust).unwrap();
        assert_eq!(doc.text(), "  // ");
        doc.toggle_line_comments(Language::Rust).unwrap();
        assert_eq!(doc.text(), "  ");
        for text in ["/* incomplete", "orphan */"] {
            let mut doc = Document::new(text);
            let before = doc.clone();
            assert!(!doc.toggle_block_comments(Language::Rust).unwrap());
            assert_eq!(doc, before);
        }
    }

    #[test]
    fn block_comments_preserve_direction_and_reject_unsafe_nesting() {
        let mut doc = Document::new("😀");
        doc.set_selections(vec![Selection { anchor: 4, head: 0 }])
            .unwrap();
        doc.toggle_block_comments(Language::Rust).unwrap();
        assert_eq!(doc.text(), "/* 😀 */");
        assert_eq!(doc.selections(), &[Selection { anchor: 7, head: 3 }]);
        doc.toggle_block_comments(Language::Rust).unwrap();
        assert_eq!(doc.text(), "😀");
        doc.undo();
        assert_eq!(doc.text(), "/* 😀 */");
        let mut css = Document::new("x */ y");
        assert!(!css.toggle_block_comments(Language::Css).unwrap());
    }
}
