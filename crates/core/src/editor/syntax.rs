//! Incremental syntax analysis. Browser/native adapters supply cancellation or a
//! clock deadline; parser policy and fold extraction stay above those adapters.
use std::ops::ControlFlow;

use tree_sitter::{InputEdit, ParseOptions, Parser, Point, Tree};

use super::{FoldRange, MAX_STRUCTURE_BYTES, normalize_folds};
use crate::highlight::Language;

const MAX_PROGRESS_CHECKS: usize = 4_096;
const MAX_FOLD_NODES: usize = 100_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyntaxStatus {
    Ready { incremental: bool },
    TooLarge,
    Cancelled,
}

/// One parser and previous tree per document. Never publish an old tree for new text.
pub struct SyntaxDocument {
    parser: Option<Parser>,
    language: Language,
    ready: bool,
    tree: Option<Tree>,
    text: String,
}

impl SyntaxDocument {
    /// Languages without a grammar use shared lexical/indentation providers.
    pub fn new(language: Language) -> Option<Self> {
        let parser = if language == Language::Rust {
            let mut parser = Parser::new();
            parser
                .set_language(&tree_sitter_rust::LANGUAGE.into())
                .ok()?;
            Some(parser)
        } else {
            None
        };
        Some(Self {
            parser,
            language,
            ready: false,
            tree: None,
            text: String::new(),
        })
    }

    pub fn update(
        &mut self,
        text: &str,
        mut should_continue: impl FnMut() -> bool,
    ) -> SyntaxStatus {
        if text.len() > MAX_STRUCTURE_BYTES {
            self.clear();
            return SyntaxStatus::TooLarge;
        }
        if !should_continue() {
            self.clear();
            return SyntaxStatus::Cancelled;
        }
        if self.ready && self.text == text {
            return SyntaxStatus::Ready { incremental: true };
        }
        let Some(parser) = self.parser.as_mut() else {
            self.text.clear();
            self.text.push_str(text);
            self.ready = true;
            return SyntaxStatus::Ready { incremental: false };
        };
        let mut previous = self.tree.clone();
        if let Some(tree) = previous.as_mut() {
            tree.edit(&input_edit(&self.text, text));
        }
        let incremental = previous.is_some();
        let mut checks = 0;
        let mut progress = |_: &tree_sitter::ParseState| {
            checks += 1;
            if checks > MAX_PROGRESS_CHECKS || !should_continue() {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        };
        let tree = parser.parse_with_options(
            &mut |offset, _| &text.as_bytes()[offset..],
            previous.as_ref(),
            Some(ParseOptions::new().progress_callback(&mut progress)),
        );
        if let Some(tree) = tree {
            self.ready = true;
            self.tree = Some(tree);
            self.text.clear();
            self.text.push_str(text);
            SyntaxStatus::Ready { incremental }
        } else {
            self.clear();
            SyntaxStatus::Cancelled
        }
    }

    fn clear(&mut self) {
        if let Some(parser) = self.parser.as_mut() {
            parser.reset();
        }
        self.ready = false;
        self.tree = None;
        self.text.clear();
    }

    pub fn folds(&self) -> Vec<FoldRange> {
        self.folds_with_tab_width(4)
    }

    pub fn folds_with_tab_width(&self, tab_width: usize) -> Vec<FoldRange> {
        if !self.ready {
            return Vec::new();
        }
        let parsed = self.tree.as_ref().map(|_| self.parser_folds());
        super::fold_ranges(&self.text, self.language, parsed.as_deref(), tab_width)
    }

    fn parser_folds(&self) -> Vec<FoldRange> {
        let Some(tree) = &self.tree else {
            return Vec::new();
        };
        let mut cursor = tree.walk();
        let mut ranges = Vec::new();
        let lines: Vec<_> = self.text.split('\n').collect();
        let mut visited = 0;
        loop {
            visited += 1;
            if visited > MAX_FOLD_NODES {
                return Vec::new();
            }
            let node = cursor.node();
            if matches!(
                node.kind(),
                "block"
                    | "declaration_list"
                    | "enum_variant_list"
                    | "field_declaration_list"
                    | "match_block"
                    | "use_list"
                    | "token_tree"
                    | "block_comment"
                    | "array_expression"
                    | "arguments"
                    | "parameters"
                    | "tuple_expression"
                    | "raw_string_literal"
            ) && !node.is_missing()
            {
                let start = node.start_position();
                let end = node.end_position();
                // Preserve a closing row when another construct begins there,
                // e.g. `} else {`: its header needs its own fold control.
                let trailing_code = lines
                    .get(end.row)
                    .and_then(|line| line.get(end.column..))
                    .is_some_and(|rest| !rest.trim().is_empty());
                ranges.push(FoldRange {
                    start_line: start.row,
                    end_line: end
                        .row
                        .saturating_sub(usize::from(end.column == 0 || trailing_code)),
                });
            }
            if cursor.goto_first_child() {
                continue;
            }
            loop {
                if cursor.goto_next_sibling() {
                    break;
                }
                if !cursor.goto_parent() {
                    return normalize_folds(ranges, self.text.split('\n').count());
                }
            }
        }
    }
}

fn point(text: &str, offset: usize) -> Point {
    let before = &text[..offset];
    Point {
        row: before.bytes().filter(|byte| *byte == b'\n').count(),
        column: before
            .rfind('\n')
            .map_or(offset, |newline| offset - newline - 1),
    }
}

fn input_edit(old: &str, new: &str) -> InputEdit {
    let change = super::text_change(old, new).unwrap_or(super::TextChange {
        range: 0..0,
        new_end: 0,
    });
    let start = change.range.start;
    let old_end = change.range.end;
    let new_end = change.new_end;
    InputEdit {
        start_byte: start,
        old_end_byte: old_end,
        new_end_byte: new_end,
        start_position: point(old, start),
        old_end_position: point(old, old_end),
        new_end_position: point(new, new_end),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn positions(tree: &Tree) -> Vec<String> {
        let mut cursor = tree.walk();
        let mut positions = Vec::new();
        loop {
            let node = cursor.node();
            positions.push(format!(
                "{}:{}..{}:{:?}..{:?}:{}:{}",
                node.kind(),
                node.start_byte(),
                node.end_byte(),
                node.start_position(),
                node.end_position(),
                node.is_missing(),
                node.is_error()
            ));
            if cursor.goto_first_child() {
                continue;
            }
            loop {
                if cursor.goto_next_sibling() {
                    break;
                }
                if !cursor.goto_parent() {
                    return positions;
                }
            }
        }
    }

    #[test]
    fn incremental_unicode_crlf_edits_match_fresh_trees_and_fold_ranges() {
        let mut syntax = SyntaxDocument::new(Language::Rust).unwrap();
        let sources = [
            "// 😀\r\nfn main() {\r\n    let x = r###\" } /* \"###;\r\n    /* outer\r\n       /* nested */\r\n    */\r\n}\r\n",
            "// 😀\r\nfn main() {\r\n    if true {\r\n        let x = r###\" } /* \"###;\r\n    }\r\n}\r\n",
            "fn main() {\n    let text = \"文😀\";\n}\n",
            "",
        ];
        for (index, source) in sources.into_iter().enumerate() {
            assert_eq!(
                syntax.update(source, || true),
                SyntaxStatus::Ready {
                    incremental: index != 0
                }
            );
            let mut fresh = SyntaxDocument::new(Language::Rust).unwrap();
            fresh.update(source, || true);
            assert_eq!(
                syntax.tree.as_ref().unwrap().root_node().to_sexp(),
                fresh.tree.as_ref().unwrap().root_node().to_sexp()
            );
            assert_eq!(
                syntax.tree.as_ref().unwrap().root_node().end_byte(),
                source.len()
            );
            assert_eq!(
                positions(syntax.tree.as_ref().unwrap()),
                positions(fresh.tree.as_ref().unwrap())
            );
            assert_eq!(syntax.folds(), fresh.folds());
            assert!(!syntax.tree.as_ref().unwrap().root_node().has_error());
        }
    }

    #[test]
    fn syntax_folds_ignore_braces_in_literals_and_include_nested_comments() {
        let source = "fn main() {\n    let text = r###\" { } \"###;\n    /* outer\n       /* nested */\n    */\n}\n";
        let mut syntax = SyntaxDocument::new(Language::Rust).unwrap();
        syntax.update(source, || true);
        assert_eq!(
            syntax.folds(),
            vec![
                FoldRange {
                    start_line: 0,
                    end_line: 5
                },
                FoldRange {
                    start_line: 2,
                    end_line: 4
                }
            ]
        );
        assert!(SyntaxDocument::new(Language::Python).is_some());
    }

    #[test]
    fn shared_closing_and_opening_rows_keep_both_fold_headers_visible() {
        let mut syntax = SyntaxDocument::new(Language::Rust).unwrap();
        syntax.update(
            "fn main() {\n    if true {\n        a();\n    } else {\n        b();\n    }\n}\n",
            || true,
        );
        assert_eq!(
            syntax.folds(),
            vec![
                FoldRange {
                    start_line: 0,
                    end_line: 6
                },
                FoldRange {
                    start_line: 1,
                    end_line: 2
                },
                FoldRange {
                    start_line: 3,
                    end_line: 5
                },
            ]
        );
    }

    #[test]
    fn cancelled_and_oversize_parses_drop_stale_trees_and_can_recover() {
        let mut syntax = SyntaxDocument::new(Language::Rust).unwrap();
        syntax.update("fn main() {\n}\n", || true);
        assert!(!syntax.folds().is_empty());
        assert_eq!(
            syntax.update("fn other() {\n}\n", || false),
            SyntaxStatus::Cancelled
        );
        assert!(syntax.folds().is_empty());
        assert_eq!(
            syntax.update(&"x".repeat(MAX_STRUCTURE_BYTES + 1), || true),
            SyntaxStatus::TooLarge
        );
        assert_eq!(
            syntax.update("fn main() {\n}\n", || true),
            SyntaxStatus::Ready { incremental: false }
        );
        // Cancel within parser progress, not just before entering the runtime.
        let mut calls = 0;
        assert_eq!(
            syntax.update(&"fn main() {}\n".repeat(10_000), || {
                calls += 1;
                calls < 2
            }),
            SyntaxStatus::Cancelled
        );
        assert!(calls >= 2);
        assert!(syntax.folds().is_empty());
        assert_eq!(
            syntax.update("fn recovered() {}", || true),
            SyntaxStatus::Ready { incremental: false }
        );
    }
}
