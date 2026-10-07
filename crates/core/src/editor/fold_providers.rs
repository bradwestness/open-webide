//! Language-aware lexical, indentation and explicit-region folds supplement a
//! parser provider. All offsets are source bytes; workspace/browser details stay out.
use super::{
    FoldRange, MAX_STRUCTURE_BYTES, Structure,
    lines::{lines, row_at},
    normalize_folds,
};
use crate::highlight::Language;

const MAX_FOLD_LINES: usize = 100_000;

/// Prefer a parser's ranges when available; otherwise use the shared lexer and
/// indentation. Explicit comment regions apply to either provider.
pub fn fold_ranges(
    text: &str,
    language: Language,
    parsed: Option<&[FoldRange]>,
    tab_width: usize,
) -> Vec<FoldRange> {
    if text.len() > MAX_STRUCTURE_BYTES {
        return Vec::new();
    }
    let rows = lines(text);
    if rows.len() > MAX_FOLD_LINES {
        return Vec::new();
    }
    let lexical = Structure::scan(text, language);
    let available = lexical.is_some();
    let structure = lexical.unwrap_or_default();
    let context = structure.regions();
    let mut ranges = parsed.map_or_else(Vec::new, <[FoldRange]>::to_vec);
    if parsed.is_none() && available {
        for &(offset, ch, pair) in &structure.brackets {
            if matches!(ch, '(' | '[' | '{')
                && let Some(close) = pair
            {
                let first = row_at(&rows, offset);
                let last = row_at(&rows, close);
                // Preserve a shared closing/header row, e.g. `} else {`.
                let trailing = !text[close + ch.len_utf8()..rows[last].body_end]
                    .trim()
                    .is_empty();
                ranges.push(FoldRange {
                    start_line: first,
                    end_line: last.saturating_sub(usize::from(trailing)),
                });
            }
        }
        for range in context.literals() {
            if !range.is_empty() {
                ranges.push(FoldRange {
                    start_line: row_at(&rows, range.start),
                    end_line: row_at(&rows, range.end.saturating_sub(1)),
                });
            }
        }
    }
    // Indentation is a fallback, not another interpretation of parser structure.
    if parsed.is_none() {
        let mut stack: Vec<(usize, usize)> = Vec::new();
        let mut previous: Option<(usize, usize)> = None;
        let mut last_content = 0;
        for (row, line) in rows.iter().enumerate() {
            let body = &text[line.start..line.body_end];
            let trimmed = body.trim_start();
            if trimmed.is_empty() {
                continue;
            }
            let offset = line.start + body.len() - trimmed.len();
            if context.is_opaque_body(offset) {
                last_content = row;
                continue;
            }
            let indent = body[..offset - line.start].chars().fold(0, |column, ch| {
                if ch == '\t' {
                    column + tab_width.clamp(1, 16) - column % tab_width.clamp(1, 16)
                } else {
                    column + 1
                }
            });
            while stack.last().is_some_and(|(_, depth)| indent <= *depth) {
                let (header, _) = stack.pop().unwrap();
                ranges.push(FoldRange {
                    start_line: header,
                    end_line: last_content,
                });
            }
            if let Some((header, depth)) = previous
                && indent > depth
            {
                stack.push((header, depth));
            }
            previous = Some((row, indent));
            last_content = row;
        }
        ranges.extend(stack.into_iter().map(|(header, _)| FoldRange {
            start_line: header,
            end_line: last_content,
        }));
    }
    let mut comment_start = None;
    for (row, line) in rows.iter().enumerate() {
        let body = &text[line.start..line.body_end];
        let offset = line.start + body.len() - body.trim_start().len();
        if context.is_line_comment(offset) {
            comment_start.get_or_insert(row);
        } else if let Some(start) = comment_start.take() {
            ranges.push(FoldRange {
                start_line: start,
                end_line: row.saturating_sub(1),
            });
        }
    }
    if let Some(start) = comment_start {
        ranges.push(FoldRange {
            start_line: start,
            end_line: rows.len().saturating_sub(1),
        });
    }
    let mut regions = Vec::new();
    for (row, line) in rows.iter().enumerate() {
        let body = &text[line.start..line.body_end];
        let trimmed = body.trim_start();
        let offset = line.start + body.len() - trimmed.len();
        // Markers inside quoted text are never directives. Plain text allows
        // explicit hash regions without claiming to understand its language.
        if context.is_literal(offset) {
            continue;
        }
        let marker = if language == Language::Plain {
            trimmed.strip_prefix('#')
        } else if context.is_comment(offset) {
            ["//", "#", "--", "<!--", "/*"]
                .iter()
                .find_map(|prefix| trimmed.strip_prefix(prefix))
        } else if matches!(language, Language::C | Language::Cpp) {
            trimmed.strip_prefix("#pragma")
        } else {
            None
        };
        let Some(marker) = marker else {
            continue;
        };
        let marker = marker.trim_start().trim_start_matches('#');
        let word = marker
            .split(|ch: char| ch.is_whitespace() || matches!(ch, '*' | '-'))
            .next()
            .unwrap_or_default();
        if word.eq_ignore_ascii_case("region") {
            regions.push(row);
        } else if word.eq_ignore_ascii_case("endregion")
            && let Some(header) = regions.pop()
        {
            ranges.push(FoldRange {
                start_line: header,
                end_line: row,
            });
        }
    }
    normalize_folds(ranges, rows.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bracket_folds_ignore_strings_comments_and_regexes_in_supported_languages() {
        for language in [
            Language::JavaScript,
            Language::TypeScript,
            Language::C,
            Language::Cpp,
            Language::Go,
            Language::Css,
            Language::Json,
        ] {
            let text = "{\r\n  \"text } 文😀\"\r\n}\r\n";
            assert!(fold_ranges(text, language, None, 4).contains(&FoldRange {
                start_line: 0,
                end_line: 2
            }));
        }
        let text = "function f() {\n  const pattern = /[{}]/;\n  /* fake {\n  } */\n}\n";
        let folds = fold_ranges(text, Language::JavaScript, None, 4);
        assert!(folds.contains(&FoldRange {
            start_line: 0,
            end_line: 4
        }));
        assert!(folds.contains(&FoldRange {
            start_line: 2,
            end_line: 3
        }));
    }
    #[test]
    fn indentation_handles_nested_python_yaml_plain_and_tab_stops() {
        let text = "def main():\n\tif ready:\n\t\tpass\n\n\tfinish()\nafter()\n";
        let folds = fold_ranges(text, Language::Python, None, 4);
        assert!(folds.contains(&FoldRange {
            start_line: 0,
            end_line: 4
        }));
        assert!(folds.contains(&FoldRange {
            start_line: 1,
            end_line: 2
        }));
        for language in [Language::Yaml, Language::Plain] {
            assert_eq!(
                fold_ranges("root:\n  child\nend\n", language, None, 4),
                vec![FoldRange {
                    start_line: 0,
                    end_line: 1
                }]
            );
        }
    }
    #[test]
    fn regions_are_nested_balanced_and_never_read_from_literals() {
        let text = "// #region outer\nconst s = `\n// #region fake\n`;\n// region inner\nwork();\n// endregion\n// #endregion\n";
        let folds = fold_ranges(text, Language::JavaScript, None, 4);
        assert!(folds.contains(&FoldRange {
            start_line: 0,
            end_line: 7
        }));
        assert!(folds.contains(&FoldRange {
            start_line: 4,
            end_line: 6
        }));
        assert!(!folds.iter().any(|range| range.start_line == 2));
        assert_eq!(
            fold_ranges(
                "#region a\ntext\n#endregion\n#region unmatched\n",
                Language::Plain,
                None,
                4
            ),
            vec![FoldRange {
                start_line: 0,
                end_line: 2
            }]
        );
    }
    #[test]
    fn consecutive_comment_rows_fold_without_swallowing_code_or_blank_rows() {
        for (language, prefix) in [
            (Language::Rust, "///"),
            (Language::Python, "#"),
            (Language::Sql, "--"),
        ] {
            let text = format!("{prefix} first\n{prefix} second\n\ncode\n{prefix} alone\n");
            let parsed = [];
            assert_eq!(
                fold_ranges(&text, language, Some(&parsed), 4),
                vec![FoldRange {
                    start_line: 0,
                    end_line: 1
                }]
            );
        }
    }

    #[test]
    fn multiline_literals_do_not_supply_indentation_or_region_directives() {
        let python = "def main():\n    \"\"\"doc\n           fake indent\n    \"\"\"\nafter()\n";
        let folds = fold_ranges(python, Language::Python, None, 4);
        assert!(folds.contains(&FoldRange {
            start_line: 0,
            end_line: 3
        }));
        assert!(folds.contains(&FoldRange {
            start_line: 1,
            end_line: 3
        }));
        assert!(!folds.iter().any(|range| range.start_line == 2));
        let yaml = "script: |- # literal\n  #region fake\n  work\n  #endregion\nnext: value\n";
        let folds = fold_ranges(yaml, Language::Yaml, None, 4);
        assert!(folds.contains(&FoldRange {
            start_line: 0,
            end_line: 3
        }));
        assert!(!folds.iter().any(|range| range.start_line == 1));
    }

    #[test]
    fn parser_ranges_are_authoritative_and_bounded_fallback_cannot_cross_them() {
        let parsed = [FoldRange {
            start_line: 0,
            end_line: 1,
        }];
        assert_eq!(
            fold_ranges(
                "header\n  body\n    extra\n",
                Language::Plain,
                Some(&parsed),
                4
            ),
            parsed
        );
        assert!(
            fold_ranges(
                &"x".repeat(MAX_STRUCTURE_BYTES + 1),
                Language::Plain,
                None,
                4
            )
            .is_empty()
        );
        assert!(fold_ranges(&"\n".repeat(MAX_FOLD_LINES), Language::Plain, None, 4).is_empty());
    }
}
