//! Inline and side-by-side diff presentation, including word highlighting.

use super::{FileDiff, Line, LineOp, ending_note_for, lcs_matches, line_lcs};
use serde::{Deserialize, Serialize};

/// A sub-line segment of diff text for intra-line highlighting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "text", rename_all = "snake_case")]
pub enum DiffChunk {
    /// Text common to both versions.
    Unchanged(String),
    /// Text deleted from the old version.
    Deleted(String),
    /// Text inserted into the new version.
    Inserted(String),
}

impl DiffChunk {
    pub fn text(&self) -> &str {
        match self {
            Self::Unchanged(s) | Self::Deleted(s) | Self::Inserted(s) => s,
        }
    }
}

/// A line in a diff with marker ('-', '+', ' ') and intra-line word chunks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffLine {
    pub marker: char,
    pub content: String,
    pub chunks: Vec<DiffChunk>,
    /// A note for a paired line whose ending differs between old and new
    /// (e.g. `"⏎ CRLF → LF"`, `"no newline at end of file"`), or `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ending_note: Option<&'static str>,
}

impl DiffLine {
    pub fn new_plain(marker: char, content: String) -> Self {
        let chunk = match marker {
            '-' => DiffChunk::Deleted(content.clone()),
            '+' => DiffChunk::Inserted(content.clone()),
            _ => DiffChunk::Unchanged(content.clone()),
        };
        Self {
            marker,
            content,
            chunks: vec![chunk],
            ending_note: None,
        }
    }

    pub fn new_with_chunks(marker: char, content: String, chunks: Vec<DiffChunk>) -> Self {
        Self {
            marker,
            content,
            chunks,
            ending_note: None,
        }
    }

    /// Return a copy of this line with `ending_note` set.
    pub fn with_ending_note(mut self, note: Option<&'static str>) -> Self {
        self.ending_note = note;
        self
    }
}

/// Split a line into tokens (alphanumeric words, runs of whitespace, or punctuation symbols).
pub fn tokenize_diff_line(line: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut chars = line.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        let is_alphanumeric = ch.is_alphanumeric() || ch == '_';
        let is_ws = ch.is_whitespace();
        let mut end = start + ch.len_utf8();
        while let Some(&(_, next_ch)) = chars.peek() {
            if (is_alphanumeric && (next_ch.is_alphanumeric() || next_ch == '_'))
                || (is_ws && next_ch.is_whitespace())
            {
                chars.next();
                end += next_ch.len_utf8();
            } else {
                break;
            }
        }
        tokens.push(&line[start..end]);
    }
    tokens
}

/// Compute intra-line word-level diff between two lines using LCS on tokens.
/// Above this many DP cells, `compute_word_diff` falls back to a whole-middle
/// delete/insert instead of the O(m·n) token alignment.
pub const MAX_WORD_DIFF_CELLS: usize = 250_000;

fn build_word_diff_chunks(
    tokens: &[&str],
    matched: &[bool],
    changed: fn(String) -> DiffChunk,
) -> Vec<DiffChunk> {
    let mut chunks = Vec::new();
    let mut current_text = String::new();
    let mut current_is_match: Option<bool> = None;
    for (idx, &tok) in tokens.iter().enumerate() {
        let is_match = matched[idx];
        match current_is_match {
            Some(m) if m == is_match => current_text.push_str(tok),
            Some(m) => {
                chunks.push(if m {
                    DiffChunk::Unchanged(current_text)
                } else {
                    changed(current_text)
                });
                current_text = tok.to_string();
            }
            None => current_text = tok.to_string(),
        }
        current_is_match = Some(is_match);
    }
    if let Some(m) = current_is_match {
        chunks.push(if m {
            DiffChunk::Unchanged(current_text)
        } else {
            changed(current_text)
        });
    }
    chunks
}

/// Push `chunk` onto `chunks`, merging it into a trailing `Unchanged` chunk
/// when both are `Unchanged`.
fn push_merged_unchanged(chunks: &mut Vec<DiffChunk>, chunk: DiffChunk) {
    if let DiffChunk::Unchanged(text) = &chunk
        && let Some(DiffChunk::Unchanged(prev)) = chunks.last_mut()
    {
        prev.push_str(text);
        return;
    }
    chunks.push(chunk);
}

/// Compute intra-line word-level diff between two lines using LCS on tokens.
pub fn compute_word_diff(old_line: &str, new_line: &str) -> (Vec<DiffChunk>, Vec<DiffChunk>) {
    let old_tokens = tokenize_diff_line(old_line);
    let new_tokens = tokenize_diff_line(new_line);

    let m = old_tokens.len();
    let n = new_tokens.len();

    if m == 0 && n == 0 {
        return (Vec::new(), Vec::new());
    }
    if m == 0 {
        return (Vec::new(), vec![DiffChunk::Inserted(new_line.to_string())]);
    }
    if n == 0 {
        return (vec![DiffChunk::Deleted(old_line.to_string())], Vec::new());
    }

    let mut prefix = 0;
    while prefix < m && prefix < n && old_tokens[prefix] == new_tokens[prefix] {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < m - prefix
        && suffix < n - prefix
        && old_tokens[m - 1 - suffix] == new_tokens[n - 1 - suffix]
    {
        suffix += 1;
    }

    let mid_old = &old_tokens[prefix..m - suffix];
    let mid_new = &new_tokens[prefix..n - suffix];
    let mid_m = mid_old.len();
    let mid_n = mid_new.len();

    let prefix_text = old_tokens[..prefix].concat();
    let suffix_text = old_tokens[m - suffix..].concat();

    let (old_mid_chunks, new_mid_chunks) = if mid_m.saturating_mul(mid_n) > MAX_WORD_DIFF_CELLS {
        let old_mid_text = mid_old.concat();
        let new_mid_text = mid_new.concat();
        let old_mid_chunks = if old_mid_text.is_empty() {
            Vec::new()
        } else {
            vec![DiffChunk::Deleted(old_mid_text)]
        };
        let new_mid_chunks = if new_mid_text.is_empty() {
            Vec::new()
        } else {
            vec![DiffChunk::Inserted(new_mid_text)]
        };
        (old_mid_chunks, new_mid_chunks)
    } else {
        let mut old_matched = vec![false; mid_m];
        let mut new_matched = vec![false; mid_n];
        for (i, j) in lcs_matches(mid_old, mid_new) {
            old_matched[i] = true;
            new_matched[j] = true;
        }

        (
            build_word_diff_chunks(mid_old, &old_matched, DiffChunk::Deleted),
            build_word_diff_chunks(mid_new, &new_matched, DiffChunk::Inserted),
        )
    };

    let mut old_chunks = Vec::new();
    if !prefix_text.is_empty() {
        push_merged_unchanged(&mut old_chunks, DiffChunk::Unchanged(prefix_text.clone()));
    }
    for chunk in old_mid_chunks {
        push_merged_unchanged(&mut old_chunks, chunk);
    }
    if !suffix_text.is_empty() {
        push_merged_unchanged(&mut old_chunks, DiffChunk::Unchanged(suffix_text.clone()));
    }

    let mut new_chunks = Vec::new();
    if !prefix_text.is_empty() {
        push_merged_unchanged(&mut new_chunks, DiffChunk::Unchanged(prefix_text));
    }
    for chunk in new_mid_chunks {
        push_merged_unchanged(&mut new_chunks, chunk);
    }
    if !suffix_text.is_empty() {
        push_merged_unchanged(&mut new_chunks, DiffChunk::Unchanged(suffix_text));
    }

    (old_chunks, new_chunks)
}

/// Compute the changed middle of a file edit as inline `(marker, line)` pairs.
///
/// Built on [`line_lcs`], so a line is only unchanged when both its text and
/// its ending match; an inserted line near the top no longer marks everything
/// below as changed. The result holds the changed lines in order: for each
/// edit region, the removed lines (marker `-`, from `old`) followed by the
/// added lines (marker `+`, from `new`). For a new file (`old` is `None`)
/// every line is `+`. Pure and natively unit-testable.
pub fn diff_inline_lines(diff: &FileDiff) -> Vec<(char, String)> {
    let old = diff.old.as_deref().unwrap_or_default();
    let mut out = Vec::new();
    for op in line_lcs(old, &diff.new) {
        match op {
            LineOp::Equal(_) => {}
            LineOp::Delete(l) => out.push(('-', l.text.to_string())),
            LineOp::Insert(l) => out.push(('+', l.text.to_string())),
        }
    }
    out
}

/// One ordered piece of a `line_lcs` op list: an unchanged line, or one edit
/// region (a run of removed lines followed by added lines).
enum DiffSegment<'a> {
    /// Present, unchanged, on both sides.
    Equal(Line<'a>),
    /// One edit region: removed lines followed by added lines.
    Cluster {
        dels: Vec<Line<'a>>,
        inss: Vec<Line<'a>>,
    },
}

/// Split a `line_lcs` op list into ordered [`DiffSegment`]s: each unchanged
/// line as `Equal`, and each maximal run of non-`Equal` ops as a `Cluster` of
/// removed lines followed by added lines.
fn diff_segments<'a>(ops: &[LineOp<'a>]) -> Vec<DiffSegment<'a>> {
    let mut segments = Vec::new();
    let mut i = 0;
    while i < ops.len() {
        match &ops[i] {
            LineOp::Equal(l) => {
                segments.push(DiffSegment::Equal(*l));
                i += 1;
            }
            LineOp::Delete(_) | LineOp::Insert(_) => {
                // Collect the contiguous run of Deletes followed by the
                // contiguous run of Inserts (one LCS edit region).
                let start = i;
                while i < ops.len() {
                    if let LineOp::Delete(_) = &ops[i] {
                        i += 1;
                    } else {
                        break;
                    }
                }
                let dels_end = i;
                while i < ops.len() {
                    if let LineOp::Insert(_) = &ops[i] {
                        i += 1;
                    } else {
                        break;
                    }
                }
                let dels: Vec<Line> = ops[start..dels_end]
                    .iter()
                    .filter_map(|op| match op {
                        LineOp::Delete(l) => Some(*l),
                        _ => None,
                    })
                    .collect();
                let inss: Vec<Line> = ops[dels_end..i]
                    .iter()
                    .filter_map(|op| match op {
                        LineOp::Insert(l) => Some(*l),
                        _ => None,
                    })
                    .collect();
                segments.push(DiffSegment::Cluster { dels, inss });
            }
        }
    }
    segments
}

/// Compute the changed middle of a file edit as detailed `DiffLine`s with
/// intra-line chunks.
///
/// Built on [`line_lcs`]. Each edit region (a run of removed lines followed by
/// added lines) is paired positionally for step 13's word-level highlighting;
/// a paired line whose ending differs carries an `ending_note` on both sides.
/// Unpaired lines in the longer run are plain. Per region the removed lines are
/// emitted before the added lines, in the order the regions appear.
pub fn diff_inline_detailed(diff: &FileDiff) -> Vec<DiffLine> {
    let old = diff.old.as_deref().unwrap_or_default();
    let ops = line_lcs(old, &diff.new);

    let mut out = Vec::new();
    for segment in diff_segments(&ops) {
        let DiffSegment::Cluster { dels, inss } = segment else {
            continue;
        };

        let min_len = dels.len().min(inss.len());
        // Removed lines: paired (word chunks + ending note) then the
        // unpaired remainder as plain lines.
        let mut add_pairs: Vec<(String, Vec<DiffChunk>, Option<&'static str>)> =
            Vec::with_capacity(min_len);
        for k in 0..min_len {
            let (old_chunks, new_chunks) = compute_word_diff(dels[k].text, inss[k].text);
            let note = ending_note_for(dels[k].ending, inss[k].ending);
            out.push(
                DiffLine::new_with_chunks('-', dels[k].text.to_string(), old_chunks)
                    .with_ending_note(note),
            );
            add_pairs.push((inss[k].text.to_string(), new_chunks, note));
        }
        for line in dels.iter().skip(min_len) {
            out.push(DiffLine::new_plain('-', line.text.to_string()));
        }
        // Added lines: paired then the unpaired remainder.
        for (content, new_chunks, note) in add_pairs {
            out.push(DiffLine::new_with_chunks('+', content, new_chunks).with_ending_note(note));
        }
        for line in inss.iter().skip(min_len) {
            out.push(DiffLine::new_plain('+', line.text.to_string()));
        }
    }
    out
}

/// Compute the changed middle of a file edit as side-by-side `(old, new)` rows.
///
/// Built on [`line_lcs`], so every row is returned — prefix context, the
/// changed middle, and suffix context. Within each edit region the removed and
/// added lines are paired side by side, padding the shorter side with `None`
/// (a pure addition or removal); a real unchanged line found by the LCS stays
/// aligned instead of shifting into a false pair. For a new file (`old` is
/// `None`) every left cell is `None`. Pure and natively unit-testable.
pub fn diff_side_by_side(diff: &FileDiff) -> Vec<(Option<String>, Option<String>)> {
    let old = diff.old.as_deref().unwrap_or_default();
    let ops = line_lcs(old, &diff.new);

    let mut rows = Vec::new();
    for segment in diff_segments(&ops) {
        match segment {
            DiffSegment::Equal(l) => {
                rows.push((Some(l.text.to_string()), Some(l.text.to_string())));
            }
            DiffSegment::Cluster { dels, inss } => {
                for k in 0..dels.len().max(inss.len()) {
                    rows.push((
                        dels.get(k).map(|l| l.text.to_string()),
                        inss.get(k).map(|l| l.text.to_string()),
                    ));
                }
            }
        }
    }
    rows
}

/// Compute the changed middle of a file edit as side-by-side rows with intra-line chunks.
///
/// Built on [`line_lcs`], so every row is returned — prefix context, the
/// changed middle, and suffix context — and a line is only unchanged when both
/// its text and its ending match. Within each edit region the removed and
/// added lines are paired positionally for word-level highlighting, with an
/// `ending_note` on a pair whose ending differs; the unpaired remainder of the
/// longer run is plain.
pub fn diff_side_by_side_detailed(diff: &FileDiff) -> Vec<(Option<DiffLine>, Option<DiffLine>)> {
    let old = diff.old.as_deref().unwrap_or_default();
    let ops = line_lcs(old, &diff.new);

    let mut rows = Vec::new();
    for segment in diff_segments(&ops) {
        match segment {
            DiffSegment::Equal(l) => {
                rows.push((
                    Some(DiffLine::new_plain(' ', l.text.to_string())),
                    Some(DiffLine::new_plain(' ', l.text.to_string())),
                ));
            }
            DiffSegment::Cluster { dels, inss } => {
                let min_len = dels.len().min(inss.len());
                for k in 0..min_len {
                    let (old_chunks, new_chunks) = compute_word_diff(dels[k].text, inss[k].text);
                    let note = ending_note_for(dels[k].ending, inss[k].ending);
                    rows.push((
                        Some(
                            DiffLine::new_with_chunks('-', dels[k].text.to_string(), old_chunks)
                                .with_ending_note(note),
                        ),
                        Some(
                            DiffLine::new_with_chunks('+', inss[k].text.to_string(), new_chunks)
                                .with_ending_note(note),
                        ),
                    ));
                }
                for line in dels.iter().skip(min_len) {
                    rows.push((Some(DiffLine::new_plain('-', line.text.to_string())), None));
                }
                for line in inss.iter().skip(min_len) {
                    rows.push((None, Some(DiffLine::new_plain('+', line.text.to_string()))));
                }
            }
        }
    }
    rows
}
