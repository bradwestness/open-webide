//! Original styled text-run boundaries, prepared without restarting segmentation.
use crate::highlight::{Token, TokenKind};
use std::{ops::Range, sync::Arc};

/// Maximum token/run operations per cooperative preparation task.
pub const PAINT_RUN_BATCH_UNITS: usize = 64;

pub struct PaintRunPreparation<'a> {
    tokens: &'a [Token],
    body_len: usize,
    normalize_cr: bool,
    max_runs: usize,
    token: usize,
    offset: usize,
    active: Option<Box<dyn Iterator<Item = Range<usize>> + 'a>>,
    runs: Vec<usize>,
    segmented_bytes: usize,
    done: bool,
    failed: bool,
}

impl<'a> PaintRunPreparation<'a> {
    pub fn new(tokens: &'a [Token], body_len: usize, normalize_cr: bool, max_runs: usize) -> Self {
        Self {
            tokens,
            body_len,
            normalize_cr,
            max_runs,
            token: 0,
            offset: 0,
            active: None,
            runs: Vec::new(),
            segmented_bytes: 0,
            done: tokens.is_empty(),
            failed: false,
        }
    }
    fn text(&self) -> &'a str {
        let text = self.tokens[self.token].text.as_str();
        if self.normalize_cr && self.token + 1 == self.tokens.len() {
            text.strip_suffix('\r').unwrap_or(text)
        } else {
            text
        }
    }
    /// Budget counts one short token or one original long-token run. A single
    /// indivisible Unicode grapheme can exceed the ordinary 512-byte run size.
    pub fn advance(&mut self, budget: usize) -> bool {
        for _ in 0..budget {
            if self.done {
                break;
            }
            let text = self.text();
            if text.len() > 512 {
                if self.active.is_none() {
                    if self.runs.len() >= self.max_runs {
                        self.failed = true;
                        self.done = true;
                        break;
                    }
                    self.active = Some(Box::new(super::visual_text_run_ranges(text)));
                }
                if let Some(run) = self.active.as_mut().and_then(Iterator::next) {
                    self.segmented_bytes += run.len();
                    if self.runs.len() >= self.max_runs {
                        self.failed = true;
                        self.done = true;
                        break;
                    }
                    self.runs.push(self.offset + run.end);
                    continue;
                }
                self.active = None;
            } else if self.tokens[self.token].kind != TokenKind::Plain
                || self
                    .tokens
                    .get(self.token + 1)
                    .is_none_or(|next| next.kind != TokenKind::Plain || next.text.len() > 512)
            {
                if self.runs.len() >= self.max_runs {
                    self.failed = true;
                    self.done = true;
                    break;
                }
                self.runs.push(self.offset + text.len());
            }
            self.offset += text.len();
            self.token += 1;
            self.done = self.token == self.tokens.len();
        }
        self.done
    }
    pub fn segmented_bytes(&self) -> usize {
        self.segmented_bytes
    }
    pub fn finish(mut self) -> Option<Arc<[usize]>> {
        if !self.done || self.failed || self.offset != self.body_len {
            return None;
        }
        self.runs.dedup();
        Some(self.runs.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn budgets_preserve_original_unicode_runs_plain_merges_and_cr_normalization() {
        let tokens = vec![
            Token {
                kind: TokenKind::Keyword,
                text: "let".into(),
            },
            Token {
                kind: TokenKind::Plain,
                text: " ".into(),
            },
            Token {
                kind: TokenKind::Plain,
                text: "name = ".into(),
            },
            Token {
                kind: TokenKind::String,
                text: "文😀e\u{301} ".repeat(3000),
            },
            Token {
                kind: TokenKind::Plain,
                text: ";\r".into(),
            },
        ];
        let len = tokens.iter().map(|token| token.text.len()).sum::<usize>() - 1;
        let mut expected = vec![3, 11];
        expected
            .extend(super::super::visual_text_run_ranges(&tokens[3].text).map(|run| 11 + run.end));
        expected.push(len);
        for budget in [1, 2, 8, 64] {
            let mut prep = PaintRunPreparation::new(&tokens, len, true, usize::MAX);
            assert!(!prep.advance(0));
            assert_eq!(prep.segmented_bytes(), 0);
            let mut turns = 0;
            while !prep.advance(budget) {
                turns += 1;
            }
            assert!(turns > 0);
            assert_eq!(prep.segmented_bytes(), tokens[3].text.len());
            assert_eq!(prep.finish().unwrap().as_ref(), expected);
        }
    }
    #[test]
    fn incomplete_wrong_extent_and_over_budget_tables_do_not_publish() {
        let tokens = vec![Token {
            kind: TokenKind::String,
            text: "a".repeat(80000),
        }];
        assert!(
            PaintRunPreparation::new(&tokens, 80000, false, 1000)
                .finish()
                .is_none()
        );
        for cap in [0, 1, 4, 16] {
            let mut prep = PaintRunPreparation::new(&tokens, 80000, false, cap);
            while !prep.advance(1) {}
            assert!(prep.segmented_bytes() <= (cap + 1) * 512);
            assert!(prep.finish().is_none());
        }
        let mut prep = PaintRunPreparation::new(&tokens, 79999, false, 1000);
        assert!(prep.advance(usize::MAX));
        assert!(prep.finish().is_none());
    }
}
