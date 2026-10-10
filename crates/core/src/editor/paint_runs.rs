//! Original styled text-run boundaries, prepared without restarting segmentation.
use crate::highlight::{Token, TokenKind};
use std::{ops::Range, sync::Arc};
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete};

/// Maximum short-token/bounded-scan operations per cooperative preparation task.
pub const PAINT_RUN_BATCH_UNITS: usize = 64;

pub struct PaintRunPreparation<'a> {
    tokens: &'a [Token],
    body_len: usize,
    normalize_cr: bool,
    max_runs: usize,
    token: usize,
    offset: usize,
    active: Option<RunScan<'a>>,
    runs: Vec<usize>,
    segmented_bytes: usize,
    scan_progress_bytes: usize,
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
            scan_progress_bytes: 0,
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
    /// Budget counts short tokens or bounded Unicode scan steps. Even one large
    /// indivisible grapheme yields while finding its complete paint boundary.
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
                    self.active = Some(RunScan::new(text));
                }
                let scan = self.active.as_mut().unwrap();
                let before = scan.progress;
                let run = scan.advance();
                self.scan_progress_bytes += scan.progress - before;
                if let Some(run) = run {
                    self.segmented_bytes += run.len();
                    if self.runs.len() >= self.max_runs {
                        self.failed = true;
                        self.done = true;
                        break;
                    }
                    self.runs.push(self.offset + run.end);
                    continue;
                }
                if !scan.done {
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
    /// Forward cursor progress plus supplied context, including suspended
    /// graphemes. Internal cursor revisits can perform additional bounded work.
    pub fn scan_progress_bytes(&self) -> usize {
        self.scan_progress_bytes
    }
    pub fn finish(mut self) -> Option<Arc<[usize]>> {
        if !self.done || self.failed || self.offset != self.body_len {
            return None;
        }
        self.runs.dedup();
        Some(self.runs.into())
    }
}

/// A cursor sees at most 512 source bytes per chunk, including backward Unicode
/// context. No complete grapheme must be materialized before a task can yield.
struct RunScan<'a> {
    text: &'a str,
    cursor: GraphemeCursor,
    chunk: Range<usize>,
    start: usize,
    progress: usize,
    done: bool,
}
impl<'a> RunScan<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            cursor: GraphemeCursor::new(0, text.len(), true),
            chunk: 0..text.floor_char_boundary(512.min(text.len())),
            start: 0,
            progress: 0,
            done: false,
        }
    }
    fn advance(&mut self) -> Option<Range<usize>> {
        let before = self.progress;
        // Bound both source progress and cursor calls: a request for more input
        // or Unicode context need not move the forward source cursor.
        for _ in 0..512 {
            if self.done || self.progress - before >= 512 {
                break;
            }
            let offset = self.cursor.cur_cursor();
            let result = self
                .cursor
                .next_boundary(&self.text[self.chunk.clone()], self.chunk.start);
            self.progress += self.cursor.cur_cursor() - offset;
            match result {
                Ok(Some(end)) => {
                    if end - self.start >= 512 || end == self.text.len() {
                        let run = self.start..end;
                        self.start = end;
                        self.done = end == self.text.len();
                        return Some(run);
                    }
                }
                Ok(None) => self.done = true,
                Err(GraphemeIncomplete::NextChunk) => {
                    // Keep the preceding scalar in the chunk. At an exact chunk
                    // start the cursor requests regional-indicator context even
                    // when its sequential parity is already known; supplying
                    // that context again would count the preceding flags twice.
                    let start = self
                        .text
                        .floor_char_boundary(self.chunk.end.saturating_sub(4));
                    self.chunk = start
                        ..self
                            .text
                            .floor_char_boundary((start + 512).min(self.text.len()));
                }
                Err(GraphemeIncomplete::PreContext(end)) => {
                    let start = self.text.ceil_char_boundary(end.saturating_sub(512));
                    self.cursor.provide_context(&self.text[start..end], start);
                    self.progress += end - start;
                }
                Err(GraphemeIncomplete::PrevChunk | GraphemeIncomplete::InvalidOffset) => {
                    unreachable!("forward cursor always retains its exact source chunk");
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn giant_graphemes_and_chunk_context_yield_without_changing_run_boundaries() {
        for text in [
            "a".repeat(1024),
            format!("{}🇺🇸", "a".repeat(508)),
            format!("{}\r\n{}", "a".repeat(511), "文😀".repeat(1000)),
            format!("e{} last", "\u{301}".repeat(40_000)),
            format!("👩{}\u{200d}👩 last", "\u{301}".repeat(40_000)),
            format!("क{}ष last", "\u{94d}\u{93c}".repeat(10_000)),
            "🇺🇸".repeat(2000),
            format!("{}{}", "\u{600}".repeat(1000), "a".repeat(1000)),
        ] {
            let expected = super::super::visual_text_run_ranges(&text)
                .map(|run| run.end)
                .collect::<Vec<_>>();
            let tokens = [Token {
                kind: TokenKind::String,
                text: text.clone(),
            }];
            for budget in [1, 8, 64] {
                let mut prep = PaintRunPreparation::new(&tokens, text.len(), false, usize::MAX);
                let mut turns = 0;
                loop {
                    let before = prep.scan_progress_bytes();
                    let done = prep.advance(budget);
                    assert!(
                        prep.scan_progress_bytes() - before <= budget * 1536,
                        "budget={budget}, progress={}, source={} bytes",
                        prep.scan_progress_bytes() - before,
                        text.len()
                    );
                    turns += 1;
                    if done {
                        break;
                    }
                    assert!(turns < text.len() * 4, "scan failed to make progress");
                }
                assert_eq!(prep.finish().unwrap().as_ref(), expected);
                if text.contains(&"\u{301}".repeat(1000)) {
                    assert!(turns > 1, "large indivisible cluster must yield");
                }
            }
        }
    }

    proptest::proptest! {
        #[test]
        fn chunked_runs_match_complete_unicode_segmentation(
            chars in proptest::collection::vec(proptest::sample::select(vec![
                'a', ' ', '\r', '\n', '\t', '文', '😀', '\u{301}', '\u{200d}',
                '🇺', '🇸', 'क', 'ष', '\u{94d}', '\u{93c}', '\u{600}',
            ]), 0..2000)
        ) {
            let text: String = chars.into_iter().collect();
            let expected = super::super::visual_text_run_ranges(&text).collect::<Vec<_>>();
            let mut scan = RunScan::new(&text);
            let mut runs = Vec::new();
            let mut turns = 0;
            while !scan.done {
                if let Some(run) = scan.advance() { runs.push(run); }
                turns += 1;
                proptest::prop_assert!(turns <= text.len() * 4 + 1);
            }
            proptest::prop_assert_eq!(runs, expected);
        }
    }

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
