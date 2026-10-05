//! Rust editing policy shared by browser projects and host-independent editor tests.
//! Offsets are UTF-8 byte boundaries; browser adapters convert UTF-16 at the edge.

use std::ops::Range;

mod fold_providers;
pub use fold_providers::fold_ranges;
mod folds;
pub use folds::{FoldCommand, FoldRange, FoldState, normalize_folds};
mod projection;
pub use projection::{FoldProjection, ProjectionError, VisibleLine};
#[cfg(feature = "editor-parser")]
mod syntax;
#[cfg(feature = "editor-parser")]
pub use syntax::{SyntaxDocument, SyntaxStatus};
mod comments;
pub use comments::{block_comment, line_comment};
mod lines;
mod paste;
mod reindent;
pub use lines::LineCommand;
mod pairs;
mod structure;
pub use structure::{MAX_STRUCTURE_BYTES, Structure, supports_brackets};
mod indent;
pub use indent::{IndentStyle, Indentation};
mod configuration;
pub use configuration::{
    ConfigSource, EditorPreferences, EditorRules, LineEnding, load_rules, resolve_rules,
};

/// Minimal changed span with UTF-8 character boundaries in both versions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextChange {
    pub range: Range<usize>,
    pub new_end: usize,
}

pub fn text_change(old: &str, new: &str) -> Option<TextChange> {
    if old == new {
        return None;
    }
    let start = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .map(|(ch, _)| ch.len_utf8())
        .sum::<usize>();
    let suffix = old[start..]
        .chars()
        .rev()
        .zip(new[start..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(ch, _)| ch.len_utf8())
        .sum::<usize>();
    Some(TextChange {
        range: start..old.len() - suffix,
        new_end: new.len() - suffix,
    })
}

/// A directional selection: its head is the moving caret.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    pub const fn caret(position: usize) -> Self {
        Self {
            anchor: position,
            head: position,
        }
    }

    pub fn range(self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }
}

/// One replacement in the document before a transaction is applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub range: Range<usize>,
    pub text: String,
}

impl Edit {
    pub fn replace(range: Range<usize>, text: impl Into<String>) -> Self {
        Self {
            range,
            text: text.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditError {
    InvalidRange,
    OverlappingEdits,
    InvalidSelection,
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidRange => "Edit is outside the document or splits a Unicode character",
            Self::OverlappingEdits => "Edits overlap",
            Self::InvalidSelection => {
                "Selection is outside the document or splits a Unicode character"
            }
        })
    }
}

impl std::error::Error for EditError {}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Transaction {
    forward: Vec<Edit>,
    inverse: Vec<Edit>,
    before: Vec<Selection>,
    after: Vec<Selection>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HistoryStep {
    group: Option<u64>,
    transactions: Vec<Transaction>,
    bytes: usize,
}

/// A document owns its edit history and selections independently of its view.
/// History stores replacements rather than a whole-file snapshot for every keypress.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    text: String,
    saved: String,
    selections: Vec<Selection>,
    history: Vec<HistoryStep>,
    history_cursor: usize,
    history_bytes: usize,
    revision: u64,
    folds: FoldState,
}

const HISTORY_BYTES: usize = 16 * 1024 * 1024;
const HISTORY_STEPS: usize = 1_000;

impl Document {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            saved: text.clone(),
            text,
            selections: vec![Selection::caret(0)],
            history: Vec::new(),
            history_cursor: 0,
            history_bytes: 0,
            revision: 0,
            folds: FoldState::default(),
        }
    }

    pub fn fold_state(&self) -> &FoldState {
        &self.folds
    }
    pub fn fold_state_mut(&mut self) -> &mut FoldState {
        &mut self.folds
    }
    pub fn projection(&self) -> FoldProjection {
        FoldProjection::new(&self.text, &self.folds)
    }

    pub fn fold_command(&mut self, command: FoldCommand) {
        let line = self.text[..self.selections[0].head]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count();
        match command {
            FoldCommand::Toggle(header) => {
                self.folds.toggle(header);
            }
            FoldCommand::Collapse { recursive } => {
                self.folds.collapse_at(line, recursive);
            }
            FoldCommand::Expand { recursive } => {
                self.folds.expand_at(line, recursive);
            }
            FoldCommand::CollapseAll => self.folds.collapse_all(),
            FoldCommand::ExpandAll => self.folds.expand_all(),
            FoldCommand::Reveal(line) => {
                self.folds.reveal(line);
            }
        }
        let projection = self.projection();
        for selection in &mut self.selections {
            if let Ok(visible) = projection.visible_selection(*selection)
                && let Ok(source) = projection.source_selection(visible)
            {
                *selection = source;
            }
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn selections(&self) -> &[Selection] {
        &self.selections
    }
    pub const fn revision(&self) -> u64 {
        self.revision
    }
    pub fn is_dirty(&self) -> bool {
        self.text != self.saved
    }
    pub fn mark_saved(&mut self) {
        self.saved.clone_from(&self.text);
    }
    /// A write can finish after another edit. Record the version actually written
    /// without treating the newer in-memory document as saved.
    pub fn mark_saved_version(&mut self, text: &str) {
        self.saved.clear();
        self.saved.push_str(text);
    }
    pub const fn can_undo(&self) -> bool {
        self.history_cursor > 0
    }
    pub fn can_redo(&self) -> bool {
        self.history_cursor < self.history.len()
    }

    pub fn set_selections(&mut self, selections: Vec<Selection>) -> Result<(), EditError> {
        validate_selections(&self.text, &selections)?;
        self.selections = selections;
        Ok(())
    }

    /// Validate all edits and resulting selections before changing any state.
    /// The caller supplies a typing/composition group; commands use `None` to
    /// form independent undo steps. Redo is discarded only after a valid edit.
    pub fn apply(
        &mut self,
        mut edits: Vec<Edit>,
        after: Vec<Selection>,
        group: Option<u64>,
    ) -> Result<bool, EditError> {
        edits.sort_by_key(|edit| (edit.range.start, edit.range.end));
        validate_edits(&self.text, &edits)?;
        let (text, inverse) = replace_edits(&self.text, &edits);
        validate_selections(&text, &after)?;
        if text == self.text {
            self.selections = after;
            return Ok(false);
        }
        let bytes = edits
            .iter()
            .chain(&inverse)
            .map(|edit| edit.text.len())
            .sum();
        let transaction = Transaction {
            forward: edits,
            inverse,
            before: self.selections.clone(),
            after: after.clone(),
        };
        for step in self.history.drain(self.history_cursor..) {
            self.history_bytes -= step.bytes;
        }
        if let Some(step) = self
            .history
            .last_mut()
            .filter(|step| group.is_some() && step.group == group)
        {
            step.transactions.push(transaction);
            step.bytes += bytes;
        } else {
            self.history.push(HistoryStep {
                group,
                transactions: vec![transaction],
                bytes,
            });
        }
        self.history_bytes += bytes;
        self.history_cursor = self.history.len();
        // Keep the newest step, even if one deliberate command exceeds the budget.
        while self.history.len() > 1
            && (self.history.len() > HISTORY_STEPS || self.history_bytes > HISTORY_BYTES)
        {
            self.history_bytes -= self.history.remove(0).bytes;
            self.history_cursor -= 1;
        }
        self.folds.rebase(&self.text, &text);
        self.text = text;
        self.selections = after;
        self.revision = self.revision.wrapping_add(1);
        Ok(true)
    }

    pub fn undo(&mut self) -> bool {
        if !self.can_undo() {
            return false;
        }
        let step = &self.history[self.history_cursor - 1];
        let mut text = std::borrow::Cow::Borrowed(self.text.as_str());
        for transaction in step.transactions.iter().rev() {
            text = std::borrow::Cow::Owned(replace_edits(&text, &transaction.inverse).0);
            self.selections.clone_from(&transaction.before);
        }
        let text = text.into_owned();
        self.folds.rebase(&self.text, &text);
        self.text = text;
        self.history_cursor -= 1;
        self.revision = self.revision.wrapping_add(1);
        true
    }

    pub fn redo(&mut self) -> bool {
        if !self.can_redo() {
            return false;
        }
        let step = &self.history[self.history_cursor];
        let mut text = std::borrow::Cow::Borrowed(self.text.as_str());
        for transaction in &step.transactions {
            text = std::borrow::Cow::Owned(replace_edits(&text, &transaction.forward).0);
            self.selections.clone_from(&transaction.after);
        }
        let text = text.into_owned();
        self.folds.rebase(&self.text, &text);
        self.text = text;
        self.history_cursor += 1;
        self.revision = self.revision.wrapping_add(1);
        true
    }
}

fn valid_position(text: &str, position: usize) -> bool {
    position <= text.len() && text.is_char_boundary(position)
}

fn validate_selections(text: &str, selections: &[Selection]) -> Result<(), EditError> {
    if selections.is_empty()
        || selections.iter().any(|selection| {
            !valid_position(text, selection.anchor) || !valid_position(text, selection.head)
        })
    {
        return Err(EditError::InvalidSelection);
    }
    Ok(())
}

fn validate_edits(text: &str, edits: &[Edit]) -> Result<(), EditError> {
    for edit in edits {
        if edit.range.start > edit.range.end
            || !valid_position(text, edit.range.start)
            || !valid_position(text, edit.range.end)
        {
            return Err(EditError::InvalidRange);
        }
    }
    for pair in edits.windows(2) {
        if pair[0].range.end > pair[1].range.start || pair[0].range.start == pair[1].range.start {
            return Err(EditError::OverlappingEdits);
        }
    }
    Ok(())
}

fn replace_edits(text: &str, edits: &[Edit]) -> (String, Vec<Edit>) {
    let mut result = String::with_capacity(text.len());
    let mut inverse = Vec::with_capacity(edits.len());
    let mut source = 0;
    for edit in edits {
        result.push_str(&text[source..edit.range.start]);
        let start = result.len();
        result.push_str(&edit.text);
        inverse.push(Edit::replace(
            start..result.len(),
            &text[edit.range.clone()],
        ));
        source = edit.range.end;
    }
    result.push_str(&text[source..]);
    (result, inverse)
}

/// Convert a browser UTF-16 offset to a valid UTF-8 boundary, snapping a split
/// surrogate to the beginning of its character and clamping offsets past EOF.
pub fn utf16_to_byte(text: &str, offset: usize) -> usize {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        if units + ch.len_utf16() > offset {
            return byte;
        }
        units += ch.len_utf16();
    }
    text.len()
}

pub fn byte_to_utf16(text: &str, offset: usize) -> Result<usize, EditError> {
    if !valid_position(text, offset) {
        return Err(EditError::InvalidSelection);
    }
    Ok(text[..offset].encode_utf16().count())
}

/// Textareas normalize CRLF to LF; DOM offsets must be mapped to the original
/// document rather than used against its differently sized line endings.
pub fn textarea_to_byte(text: &str, offset: usize) -> usize {
    let mut units = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((byte, ch)) = chars.next() {
        if units == offset {
            return byte;
        }
        if ch == '\r' && chars.peek().is_some_and(|(_, next)| *next == '\n') {
            continue;
        }
        if units + ch.len_utf16() > offset {
            return byte;
        }
        units += ch.len_utf16();
    }
    text.len()
}

pub fn byte_to_textarea(text: &str, offset: usize) -> Result<usize, EditError> {
    if !valid_position(text, offset) {
        return Err(EditError::InvalidSelection);
    }
    let mut chars = text.char_indices().peekable();
    let mut units = 0;
    while let Some((byte, ch)) = chars.next() {
        if byte >= offset {
            break;
        }
        if ch == '\r' && chars.peek().is_some_and(|(_, next)| *next == '\n') {
            continue;
        }
        units += ch.len_utf16();
    }
    Ok(units)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_snapshot_does_not_mark_newer_typing_clean() {
        let mut document = Document::new("a");
        document
            .apply(
                vec![Edit::replace(1..1, "b")],
                vec![Selection::caret(2)],
                None,
            )
            .unwrap();
        let saved = document.text().to_string();
        document
            .apply(
                vec![Edit::replace(2..2, "c")],
                vec![Selection::caret(3)],
                None,
            )
            .unwrap();
        document.mark_saved_version(&saved);
        assert!(document.is_dirty());
        document.undo();
        assert_eq!(document.text(), "ab");
        assert!(!document.is_dirty());
        document.undo();
        assert!(document.is_dirty());
    }

    #[test]
    fn atomic_multicursor_edits_round_trip_selections_and_line_endings() {
        let mut document = Document::new("😀 one\r\ntwo\r\n");
        let before = vec![Selection { anchor: 8, head: 5 }, Selection::caret(10)];
        document.set_selections(before.clone()).unwrap();
        let after = vec![Selection::caret(7), Selection::caret(13)];
        document
            .apply(
                vec![Edit::replace(10..13, "three"), Edit::replace(5..8, "ONE")],
                after.clone(),
                None,
            )
            .unwrap();
        assert_eq!(document.text(), "😀 ONE\r\nthree\r\n");
        assert!(document.is_dirty());
        assert!(document.undo());
        assert_eq!(document.text(), "😀 one\r\ntwo\r\n");
        assert_eq!(document.selections(), before);
        assert!(!document.is_dirty());
        assert!(document.redo());
        assert_eq!(document.selections(), after);
    }

    #[test]
    fn invalid_transactions_preserve_text_history_and_redo() {
        let mut document = Document::new("😀x");
        document
            .apply(
                vec![Edit::replace(4..5, "y")],
                vec![Selection::caret(5)],
                None,
            )
            .unwrap();
        document.undo();
        let original = document.clone();
        for (edits, selection) in [
            (vec![Edit::replace(1..2, "z")], Selection::caret(0)),
            (
                vec![Edit::replace(0..4, "a"), Edit::replace(0..0, "b")],
                Selection::caret(0),
            ),
            (vec![Edit::replace(4..5, "z")], Selection::caret(1)),
        ] {
            assert!(document.apply(edits, vec![selection], None).is_err());
            assert_eq!(document, original);
        }
        assert!(document.redo());
        assert_eq!(document.text(), "😀y");
    }

    #[test]
    fn typing_groups_and_commands_are_distinct_undo_steps() {
        let mut document = Document::new("");
        for (position, text) in [(0, "a"), (1, "b"), (2, "c")] {
            document
                .apply(
                    vec![Edit::replace(position..position, text)],
                    vec![Selection::caret(position + 1)],
                    Some(1),
                )
                .unwrap();
        }
        document
            .apply(
                vec![Edit::replace(0..3, "ABC")],
                vec![Selection::caret(3)],
                None,
            )
            .unwrap();
        document.undo();
        assert_eq!(document.text(), "abc");
        document.undo();
        assert_eq!(document.text(), "");
        document.redo();
        assert_eq!(document.text(), "abc");
        document.mark_saved();
        document.redo();
        assert!(document.is_dirty());
        document.undo();
        assert!(!document.is_dirty());
    }

    #[test]
    fn browser_positions_never_split_unicode() {
        let text = "a😀é\r\n";
        assert_eq!(utf16_to_byte(text, 2), 1);
        assert_eq!(utf16_to_byte(text, 3), 5);
        assert_eq!(utf16_to_byte(text, 100), text.len());
        for byte in 0..=text.len() {
            if text.is_char_boundary(byte) {
                assert_eq!(
                    utf16_to_byte(text, byte_to_utf16(text, byte).unwrap()),
                    byte
                );
            }
        }
    }
}
