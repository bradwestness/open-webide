//! Rust editing policy shared by browser projects and host-independent editor tests.
//! Offsets are UTF-8 byte boundaries; browser adapters convert UTF-16 at the edge.

use std::ops::Range;

mod fold_providers;
pub use fold_providers::fold_ranges;
mod navigation;
mod search;
pub use navigation::{
    has_adjacent_bracket, indent_guide_columns, line_column, matching_bracket,
    matching_bracket_with_context, navigation_target, offset_at_line_column,
};
pub use search::{SearchError, SearchMatch, SearchOptions, SearchPattern};
mod clipboard;
mod motion;
mod pointer;
pub use pointer::{PointerSelection, selection_scroll_delta};
mod motion_queue;
pub use motion_queue::{MotionQueue, MotionRequest};
mod visual_index;
mod visual_motion;
pub use visual_index::VisualLineIndex;
mod visual_neighbors;
pub use visual_motion::{
    MAX_VISUAL_CARETS, VisualCaret, VisualLayout, visual_caret_offsets, visual_line_offsets,
    visual_text_runs,
};
pub use visual_neighbors::{
    VisualLineRows, visual_neighbor_rows, visual_probe_rows, visual_row_id,
};
mod native;
mod native_value;
pub use clipboard::{CLIPBOARD_SELECTIONS_MIME, ClipboardContent};
pub use motion::SelectionMotion;
mod selections;
pub use native::NativeInputKind;
pub use selections::{
    MAX_SELECTIONS, SelectionCommand, SelectionError, column_selections, normalize_selections,
};
mod recovery;
pub use recovery::{
    DocumentRecovery, EditorRecovery, EditorRecoveryFile, EditorRecoveryRecord, EditorRecoveryRoot,
    MAX_RECOVERY_BYTES, MAX_RECOVERY_FILES, RecoveryDiskState, RecoveryScroll, recovery_body_limit,
};
mod folds;
pub use folds::{FoldCommand, FoldRange, FoldState, normalize_folds};
mod capacity;
pub use capacity::{
    EditorLimit, MAX_EDITOR_BYTES, MAX_EDITOR_LINE_BYTES, MAX_EDITOR_LINES, TEXT_PAGE_BYTES,
    TextPage, editor_limit,
};
mod paint;
pub use paint::{PaintCoverage, PaintPosition, PaintSelection};
mod viewport;
pub use viewport::{
    DocumentExtent, EditorViewport, MAX_MEASURE_BATCHES_PER_FRAME, MAX_MEASURE_BYTES,
    MAX_MEASURE_FONT_WAIT_MS, MAX_MEASURE_ROWS, MeasuredRows, RowMeasurementPlan, RowPaintWindow,
    horizontal_paint_bounds, needs_measured_batches, row_measurement_batch, wrapped_paint_window,
};
mod projection;
pub use projection::{FoldProjection, ProjectionError, VisibleLine};
#[cfg(feature = "editor-parser")]
mod syntax;
#[cfg(all(feature = "editor-parser", any(test, feature = "test-support")))]
pub mod syntax_contracts;
#[cfg(feature = "editor-parser")]
mod syntax_injections;
#[cfg(feature = "editor-parser")]
mod syntax_providers;
#[cfg(feature = "editor-parser")]
pub use syntax::{
    MAX_ANALYSIS_MESSAGE_BYTES, MAX_SYNTAX_DOCUMENTS, MAX_SYNTAX_REQUEST_BYTES,
    MAX_SYNTAX_SOURCE_BYTES, SYNTAX_PROTOCOL_VERSION, SyntaxAnalysis, SyntaxAnalysisData,
    SyntaxDocument, SyntaxPreparations, SyntaxReply, SyntaxRequest, SyntaxSource, SyntaxStatus,
    preparation_exceeds_limits,
};
#[cfg(feature = "editor-parser")]
pub use syntax_providers::{
    ContextSelector, HighlightSelector, InjectionSelector, SYNTAX_PROVIDERS, SyntaxContextKind,
    SyntaxProvider, syntax_provider,
};
mod comments;
pub use comments::{block_comment, line_comment};
mod coordinates;
mod index;
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
    ConfigSource, EditorFont, EditorPreferences, EditorRules, LineEnding, load_rules, resolve_rules,
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
pub struct Edit<T = String> {
    pub range: Range<usize>,
    pub text: T,
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
    StaleContext,
    OverlappingEdits,
    InvalidSelection,
    TooManySelections,
    OutputTooLarge,
    Capacity(EditorLimit),
    CompositionActive,
    UnsupportedNativeInput,
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Self::Capacity(limit) = self {
            return std::fmt::Display::fmt(limit, f);
        }
        f.write_str(match self {
            Self::InvalidRange => "Edit is outside the document or splits a Unicode character",
            Self::StaleContext => "Editing context no longer matches this document",
            Self::OverlappingEdits => "Edits overlap",
            Self::InvalidSelection => {
                "Selection is outside the document or splits a Unicode character"
            }
            Self::TooManySelections => "The editor supports up to 512 selections",
            Self::Capacity(_) => unreachable!(),
            Self::OutputTooLarge => "Edit would exceed the 32 MiB editing limit",
            Self::CompositionActive => {
                "Finish the input composition before running an editor command"
            }
            Self::UnsupportedNativeInput => {
                "This native input cannot be applied to multiple selections"
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
    // Clones share a text version; edits and replacements create a new identity.
    identity: std::sync::Arc<()>,
    text: String,
    editor_limits: bool,
    line_index: index::LineIndex,
    projection: ProjectionCache,
    saved: String,
    selections: Vec<Selection>,
    history: Vec<HistoryStep>,
    history_cursor: usize,
    history_bytes: usize,
    revision: u64,
    folds: FoldState,
    selection_history: Vec<Vec<Selection>>,
    composition: Option<Box<native::Composition>>,
    motion_columns: Option<visual_motion::MotionColumns>,
}

// Derived presentation allocations are outside document identity/history. Clones
// rebuild lazily, while previously returned projections stay immutable.
#[derive(Debug, Default)]
struct ProjectionCache(std::sync::OnceLock<FoldProjection>);
impl Clone for ProjectionCache {
    fn clone(&self) -> Self {
        Self::default()
    }
}
impl PartialEq for ProjectionCache {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}
impl Eq for ProjectionCache {}

const HISTORY_BYTES: usize = 16 * 1024 * 1024;
const HISTORY_STEPS: usize = 1_000;
const EDIT_RESERVE_BYTES: usize = 64 * 1024;
pub const MAX_DOCUMENT_BYTES: usize = 32 * 1024 * 1024;

impl Document {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            identity: std::sync::Arc::new(()),
            saved: text.clone(),
            editor_limits: false,
            line_index: index::LineIndex::new(&text),
            projection: ProjectionCache::default(),
            text,
            selections: vec![Selection::caret(0)],
            history: Vec::new(),
            history_cursor: 0,
            history_bytes: 0,
            revision: 0,
            folds: FoldState::default(),
            selection_history: Vec::new(),
            composition: None,
            motion_columns: None,
        }
    }

    /// Admit an interactive document before allocating per-line metadata.
    pub fn for_editor(text: impl Into<String>) -> Result<Self, EditError> {
        let text = text.into();
        if let Some(limit) = editor_limit(&text) {
            return Err(EditError::Capacity(limit));
        }
        let mut document = Self::new(text);
        document.enforce_editor_limits();
        Ok(document)
    }

    /// Interactive hosts apply the same admission limits to every transaction,
    /// including commands, multi-cursor replication and composition previews.
    pub fn enforce_editor_limits(&mut self) {
        self.editor_limits = true;
    }

    pub fn fold_state(&self) -> &FoldState {
        &self.folds
    }
    /// Refresh fold providers without invalidating an unchanged projection.
    pub fn set_fold_ranges(&mut self, ranges: Vec<FoldRange>) -> bool {
        let before = self.folds.clone();
        self.folds.set_ranges(ranges, self.line_index.rows.len());
        let changed = before != self.folds;
        if changed {
            let before_collapsed = before
                .ranges()
                .iter()
                .filter_map(|range| before.collapsed_at(range.start_line));
            let after_collapsed = self
                .folds
                .ranges()
                .iter()
                .filter_map(|range| self.folds.collapsed_at(range.start_line));
            if !before_collapsed.eq(after_collapsed) {
                self.projection.0.take();
                self.motion_columns = None;
            }
        }
        changed
    }
    pub fn fold_state_mut(&mut self) -> &mut FoldState {
        self.motion_columns = None;
        self.projection.0.take();
        &mut self.folds
    }
    pub fn projection(&self) -> FoldProjection {
        self.projection
            .0
            .get_or_init(|| FoldProjection::indexed(&self.text, &self.folds, &self.line_index))
            .clone()
    }

    /// Browser surrounding text is bounded independently of document selections.
    /// Clipboard, commands and visual layout continue to use full source offsets.
    pub fn input_context(&self, max_bytes: usize) -> Result<FoldProjection, ProjectionError> {
        self.projection()
            .input_context(self.selections[0], max_bytes)
    }

    pub fn reveal_selection(&mut self) -> bool {
        let rows = &self.line_index.rows;
        let mut changed = false;
        for selected in lines::selected_rows(rows, &self.selections) {
            changed |= self
                .folds
                .reveal_lines(selected.start, selected.end.saturating_sub(1));
        }
        if changed {
            self.projection.0.take();
        }
        changed
    }

    pub fn fold_command(&mut self, command: FoldCommand) -> bool {
        if self.is_composing() {
            return false;
        }
        self.motion_columns = None;
        let before = self.selections.clone();
        let previous_folds = self.folds.clone();
        let line = lines::row_at(&self.line_index.rows, self.selections[0].head);
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
        let changed = previous_folds != self.folds;
        if changed {
            self.projection.0.take();
        }
        let projection = self.projection();
        for selection in &mut self.selections {
            if let Ok(visible) = projection.visible_selection(*selection)
                && let Ok(source) = projection.source_selection(visible)
            {
                *selection = source;
            }
        }
        self.selections = normalize_selections(&self.text, self.selections.clone())
            .expect("fold projections preserve source selection boundaries");
        if self.selections != before {
            self.selection_history.clear();
            self.motion_columns = None;
        }
        changed
    }

    pub fn byte_to_textarea(&self, offset: usize) -> Result<usize, EditError> {
        self.line_index.byte_to_textarea(&self.text, offset)
    }
    pub fn textarea_to_byte(&self, offset: usize) -> usize {
        self.line_index.textarea_to_byte(&self.text, offset)
    }
    pub fn native_selection(&self, selection: Selection) -> Selection {
        self.line_index.native_selection(&self.text, selection)
    }
    pub fn line_column(&self, offset: usize) -> (usize, usize) {
        self.line_index.line_column(&self.text, offset)
    }
    pub fn line_count(&self) -> usize {
        self.line_index.rows.len()
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
        if let Some(composition) = &mut self.composition {
            composition.mark_saved_version(&self.text);
        }
    }
    /// A write can finish after another edit. Record the version actually written
    /// without treating the newer in-memory document as saved.
    pub fn mark_saved_version(&mut self, text: &str) {
        self.saved.clear();
        self.saved.push_str(text);
        if let Some(composition) = &mut self.composition {
            composition.mark_saved_version(text);
        }
    }
    pub const fn can_undo(&self) -> bool {
        self.history_cursor > 0
    }
    pub fn can_redo(&self) -> bool {
        self.history_cursor < self.history.len()
    }

    pub fn set_selections(&mut self, selections: Vec<Selection>) -> Result<(), EditError> {
        if self.is_composing() {
            return Err(EditError::CompositionActive);
        }
        let selections = normalize_selections(&self.text, selections)?;
        if selections != self.selections {
            self.selection_history.clear();
            self.motion_columns = None;
        }
        self.selections = selections;
        Ok(())
    }

    fn validate_editor_edits<T: AsRef<str>>(&self, edits: &[Edit<T>]) -> Result<(), EditError> {
        if self.editor_limits
            && let Some(limit) = capacity::editor_limit_edits(&self.text, &self.line_index, edits)
        {
            return Err(EditError::Capacity(limit));
        }
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
        if self.is_composing() {
            return Err(EditError::CompositionActive);
        }
        edits.sort_by_key(|edit| (edit.range.start, edit.range.end));
        validate_edits(&self.text, &edits)?;
        let output = edits
            .iter()
            .try_fold(self.text.len(), |size, edit| {
                size.checked_sub(edit.range.len())
                    .and_then(|size| size.checked_add(edit.text.len()))
            })
            .ok_or(EditError::OutputTooLarge)?;
        if output > MAX_DOCUMENT_BYTES {
            return Err(EditError::OutputTooLarge);
        }
        let parts = edit_parts(&self.text, &edits);
        self.validate_editor_edits(&edits)?;
        let after = selections::normalize_selection_positions(after, |offset| {
            let mut start = 0;
            for part in &parts {
                let end = start + part.len();
                if offset <= end {
                    return part.is_char_boundary(offset - start);
                }
                start = end;
            }
            false
        })?;
        if edits
            .iter()
            .all(|edit| self.text[edit.range.clone()] == edit.text)
        {
            if self.selections != after {
                self.selection_history.clear();
                self.motion_columns = None;
            }
            self.selections = after;
            return Ok(false);
        }
        let inverse = inverse_edits(&self.text, &edits);
        drop(parts);
        replace_indexed_text(
            &mut self.text,
            &mut self.line_index,
            &mut self.folds,
            &mut self.projection,
            &edits,
        );
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
        self.selections = after;
        self.selection_history.clear();
        self.motion_columns = None;
        self.identity = std::sync::Arc::new(());
        self.revision = self.revision.wrapping_add(1);
        Ok(true)
    }

    pub fn undo(&mut self) -> bool {
        if self.is_composing() || !self.can_undo() {
            return false;
        }
        let step = &self.history[self.history_cursor - 1];
        for transaction in step.transactions.iter().rev() {
            replace_indexed_text(
                &mut self.text,
                &mut self.line_index,
                &mut self.folds,
                &mut self.projection,
                &transaction.inverse,
            );
            self.selections.clone_from(&transaction.before);
        }
        self.history_cursor -= 1;
        self.selection_history.clear();
        self.motion_columns = None;
        self.identity = std::sync::Arc::new(());
        self.revision = self.revision.wrapping_add(1);
        true
    }

    pub fn redo(&mut self) -> bool {
        if self.is_composing() || !self.can_redo() {
            return false;
        }
        let step = &self.history[self.history_cursor];
        for transaction in &step.transactions {
            replace_indexed_text(
                &mut self.text,
                &mut self.line_index,
                &mut self.folds,
                &mut self.projection,
                &transaction.forward,
            );
            self.selections.clone_from(&transaction.after);
        }
        self.history_cursor += 1;
        self.selection_history.clear();
        self.motion_columns = None;
        self.identity = std::sync::Arc::new(());
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

fn edit_parts<'a, T: AsRef<str>>(text: &'a str, edits: &'a [Edit<T>]) -> Vec<&'a str> {
    let mut parts = Vec::with_capacity(edits.len() * 2 + 1);
    let mut source = 0;
    for edit in edits {
        parts.push(&text[source..edit.range.start]);
        parts.push(edit.text.as_ref());
        source = edit.range.end;
    }
    parts.push(&text[source..]);
    parts
}

fn inverse_edits(text: &str, edits: &[Edit]) -> Vec<Edit> {
    let mut inverse = Vec::with_capacity(edits.len());
    let mut source = 0;
    let mut output = 0;
    for edit in edits {
        output += edit.range.start - source;
        inverse.push(Edit::replace(
            output..output + edit.text.len(),
            &text[edit.range.clone()],
        ));
        output += edit.text.len();
        source = edit.range.end;
    }
    inverse
}

fn replace_indexed_text(
    text: &mut String,
    index: &mut index::LineIndex,
    folds: &mut FoldState,
    projection: &mut ProjectionCache,
    edits: &[Edit],
) {
    // No-op ranges must not open folds or discard unchanged row coordinates.
    let edits: Vec<_> = edits
        .iter()
        .filter_map(|edit| {
            let change = text_change(&text[edit.range.clone()], &edit.text)?;
            Some(Edit {
                range: edit.range.start + change.range.start..edit.range.start + change.range.end,
                text: &edit.text[change.range.start..change.new_end],
            })
        })
        .collect();
    if edits.is_empty() {
        return;
    }
    let boundaries = folds.prepare_rebase(&index.rows, &edits);
    // Merge edits whose row contexts overlap. Multiple cursors in one long row
    // rebuild it once, while distant edits retain unchanged interior indexes.
    let mut batches: Vec<(std::ops::Range<usize>, usize)> = Vec::new();
    for (position, edit) in edits.iter().enumerate() {
        let first = lines::row_at(&index.rows, edit.range.start).saturating_sub(1);
        let last = (lines::row_at(&index.rows, edit.range.end) + 1).min(index.rows.len());
        if let Some((range, end_row)) = batches.last_mut()
            && first <= *end_row
        {
            range.end = position + 1;
            *end_row = (*end_row).max(last);
        } else {
            batches.push((position..position + 1, last));
        }
    }
    let mut size = text.len();
    let mut peak = size;
    for edit in edits.iter().rev() {
        size = size - edit.range.len() + edit.text.len();
        peak = peak.max(size);
    }
    if peak > text.capacity() {
        // Avoid doubling an admitted multi-megabyte buffer for one keystroke.
        text.reserve_exact(peak + EDIT_RESERVE_BYTES - text.len());
    }
    for (batch, _) in batches.into_iter().rev() {
        let edits = &edits[batch];
        let changed = edits[0].range.start..edits.last().unwrap().range.end;
        let old_len = text.len();
        for edit in edits.iter().rev() {
            text.replace_range(edit.range.clone(), edit.text);
        }
        let new_end = text.len() - (old_len - changed.end);
        index.update(old_len, text, changed, new_end);
    }
    projection.0.take();
    folds.finish_rebase(boundaries, &index.rows);
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
    textarea_chars_to_byte(text.chars(), offset)
}

fn textarea_chars_to_byte(chars: impl Iterator<Item = char>, offset: usize) -> usize {
    let mut units = 0;
    let mut byte = 0;
    let mut chars = chars.peekable();
    while let Some(ch) = chars.next() {
        if units == offset {
            return byte;
        }
        if ch == '\r' && chars.peek().is_some_and(|next| *next == '\n') {
            byte += ch.len_utf8();
            continue;
        }
        if units + ch.len_utf16() > offset {
            return byte;
        }
        units += ch.len_utf16();
        byte += ch.len_utf8();
    }
    byte
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
    proptest::proptest! {
        #[test]
        fn disjoint_unicode_transactions_match_full_replacement_and_history(
            characters in proptest::collection::vec(proptest::sample::select(vec!['a', '文', '😀', '\r', '\n', '\u{301}']), 0..100),
            requested in proptest::collection::vec((0_usize..100, 0_usize..100, 0_usize..5), 0..10),
        ) {
            let source: String = characters.into_iter().collect();
            let positions: Vec<_> = source.char_indices().map(|(at, _)| at).chain([source.len()]).collect();
            let inserted = ["", "文😀", "\r", "\n", "a\r\nb"];
            let mut requested: Vec<_> = requested.into_iter().map(|(start, end, text)| {
                let a = positions[start % positions.len()];
                let b = positions[end % positions.len()];
                Edit::replace(a.min(b)..a.max(b), inserted[text])
            }).collect();
            requested.sort_by_key(|edit| (edit.range.start, edit.range.end));
            let mut edits: Vec<Edit> = Vec::new();
            for edit in requested {
                if edits.last().is_none_or(|previous| previous.range.end <= edit.range.start && previous.range.start != edit.range.start) {
                    edits.push(edit);
                }
            }
            let expected = replace_edits(&source, &edits).0;
            let mut document = Document::new(source.as_str());
            document.apply(edits, vec![Selection::caret(expected.len()), Selection::caret(0)], None).unwrap();
            proptest::prop_assert_eq!(document.text(), expected.as_str());
            proptest::prop_assert_eq!(&document.line_index, &index::LineIndex::new(&expected));
            if expected != source {
                proptest::prop_assert!(document.undo());
                proptest::prop_assert_eq!(document.text(), source.as_str());
                proptest::prop_assert_eq!(&document.line_index, &index::LineIndex::new(&source));
                proptest::prop_assert!(document.redo());
                proptest::prop_assert_eq!(document.text(), expected.as_str());
                proptest::prop_assert_eq!(&document.line_index, &index::LineIndex::new(&expected));
            }
        }
    }

    #[test]
    fn large_buffer_growth_keeps_bounded_spare_capacity_for_typing() {
        let source = ("x".repeat(127) + "\n").repeat(4096);
        let mut document = Document::new(source.as_str());
        document
            .apply(
                vec![Edit::replace(0..0, "文")],
                vec![Selection::caret(3)],
                Some(1),
            )
            .unwrap();
        assert!(document.text.capacity() - document.text.len() <= EDIT_RESERVE_BYTES);
        let allocation = document.text.as_ptr();
        for _ in 0..100 {
            document
                .apply(
                    vec![Edit::replace(0..0, "😀")],
                    vec![Selection::caret(4)],
                    Some(1),
                )
                .unwrap();
            assert_eq!(document.text.as_ptr(), allocation);
        }
        assert!(document.undo());
        assert_eq!(document.text(), source);
        assert_eq!(document.text.as_ptr(), allocation);
        assert!(document.redo());
        assert_eq!(document.text.as_ptr(), allocation);
    }

    #[test]
    fn indexed_edits_reuse_the_buffer_and_unchanged_interior_coordinates() {
        use super::*;
        let middle = "文😀e\u{301} words ".repeat(7000);
        let source = format!("head\nspacer\n{middle}\nspacer\nlast");
        let mut document = Document::new(source.as_str());
        document.text.reserve(64);
        let allocation = document.text.as_ptr();
        let retained = document.line_index.coordinates[2].visual().unwrap();
        let last = source.rfind("last").unwrap();
        document
            .apply(
                vec![
                    Edit::replace(0..4, "header"),
                    Edit::replace(last..source.len(), "ending"),
                ],
                vec![Selection::caret(0)],
                None,
            )
            .unwrap();
        assert_eq!(document.text.as_ptr(), allocation);
        assert!(
            document.line_index.coordinates[2]
                .visual()
                .unwrap()
                .shared_with(&retained)
        );
        assert_eq!(document.line_index, index::LineIndex::new(document.text()));
        assert!(document.undo());
        assert_eq!(document.text(), source);
        assert_eq!(document.text.as_ptr(), allocation);
        assert!(
            document.line_index.coordinates[2]
                .visual()
                .unwrap()
                .shared_with(&retained)
        );
        assert!(document.redo());
        assert_eq!(document.text.as_ptr(), allocation);
        assert!(
            document.line_index.coordinates[2]
                .visual()
                .unwrap()
                .shared_with(&retained)
        );
    }

    #[test]
    fn provider_metadata_preserves_native_projection_until_visible_folds_change() {
        let mut document = Document::new("header\none\ntwo\nthree\nfour\n");
        let full = document.projection();
        assert!(document.set_fold_ranges(vec![FoldRange {
            start_line: 0,
            end_line: 2
        }]));
        assert!(std::ptr::eq(
            full.text().as_ptr(),
            document.projection().text().as_ptr()
        ));
        assert!(document.fold_command(FoldCommand::Toggle(0)));
        let collapsed = document.projection();
        assert_ne!(full.text(), collapsed.text());
        assert!(document.set_fold_ranges(vec![
            FoldRange {
                start_line: 0,
                end_line: 2
            },
            FoldRange {
                start_line: 3,
                end_line: 4
            },
        ]));
        assert!(std::ptr::eq(
            collapsed.text().as_ptr(),
            document.projection().text().as_ptr()
        ));
        assert!(document.set_fold_ranges(vec![FoldRange {
            start_line: 0,
            end_line: 3
        }]));
        assert_ne!(collapsed.text(), document.projection().text());
        assert_eq!(document.projection().text(), "header\nfour\n");
    }

    #[test]
    fn precise_edits_preserve_folds_between_disjoint_changes_and_inside_unchanged_replacements() {
        use super::*;
        let source = "head\nfn middle {\n body\n}\nlast\n";
        let mut document = Document::new(source);
        document.set_fold_ranges(vec![FoldRange {
            start_line: 1,
            end_line: 3,
        }]);
        document.fold_command(FoldCommand::CollapseAll);
        let last = source.find("last").unwrap();
        let body = source.find("body").unwrap();
        document
            .apply(
                vec![
                    Edit::replace(0..4, "new\nhead"),
                    Edit::replace(body..body + 4, "body"),
                    Edit::replace(last..last + 4, "tail"),
                ],
                vec![Selection::caret(0)],
                None,
            )
            .unwrap();
        document.set_fold_ranges(vec![FoldRange {
            start_line: 2,
            end_line: 4,
        }]);
        assert!(document.fold_state().collapsed_at(2).is_some());
        assert!(document.undo());
        document.set_fold_ranges(vec![FoldRange {
            start_line: 1,
            end_line: 3,
        }]);
        assert!(document.fold_state().collapsed_at(1).is_some());
        assert!(document.redo());
        document.set_fold_ranges(vec![FoldRange {
            start_line: 2,
            end_line: 4,
        }]);
        assert!(document.fold_state().collapsed_at(2).is_some());
        let source = document.text().to_string();
        let replacement = source.replacen("tail", "ending", 1);
        document
            .apply(
                vec![Edit::replace(0..source.len(), replacement)],
                vec![Selection::caret(0)],
                None,
            )
            .unwrap();
        document.set_fold_ranges(vec![FoldRange {
            start_line: 2,
            end_line: 4,
        }]);
        assert!(document.fold_state().collapsed_at(2).is_some());
    }

    #[test]
    fn virtual_transaction_validation_matches_full_strings_and_keeps_failures_atomic() {
        use super::*;
        for source in ["", "abc", "文😀e\u{301}\r\nx\n", "a\r\nb\rc"] {
            let boundaries: Vec<_> = source
                .char_indices()
                .map(|(at, _)| at)
                .chain([source.len()])
                .collect();
            for &start in &boundaries {
                for &end in boundaries.iter().filter(|&&at| at >= start) {
                    for inserted in ["", "文😀", "\r", "\n", "x\r\ny"] {
                        let mut document = Document::new(source);
                        let edits = vec![Edit::replace(start..end, inserted)];
                        let expected = replace_edits(source, &edits).0;
                        document
                            .apply(
                                edits,
                                vec![Selection {
                                    anchor: expected.len(),
                                    head: 0,
                                }],
                                Some(1),
                            )
                            .unwrap();
                        assert_eq!(document.text(), expected);
                        assert_eq!(document.line_index, index::LineIndex::new(&expected));
                        let before = document.clone();
                        assert!(
                            document
                                .apply(
                                    vec![Edit::replace(0..0, "😀")],
                                    vec![Selection::caret(1)],
                                    None
                                )
                                .is_err()
                        );
                        assert_eq!(document, before);
                        if expected != source {
                            assert!(document.undo());
                            assert_eq!(document.text(), source);
                            assert_eq!(document.line_index, index::LineIndex::new(source));
                            assert!(document.redo());
                            assert_eq!(document.text(), expected);
                            assert_eq!(document.line_index, index::LineIndex::new(&expected));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn edit_preparation_reveals_selected_headers_but_keeps_disjoint_folds() {
        use super::*;
        let source = "first {\r\n body\r\n}\r\nsecond {\r\n body\r\n}\r\n";
        let mut document = Document::new(source);
        document.fold_state_mut().set_ranges(
            vec![
                FoldRange {
                    start_line: 0,
                    end_line: 2,
                },
                FoldRange {
                    start_line: 3,
                    end_line: 5,
                },
            ],
            7,
        );
        document.fold_command(FoldCommand::CollapseAll);
        document
            .set_selections(vec![Selection {
                anchor: source.find("second").unwrap(),
                head: 0,
            }])
            .unwrap();
        assert!(document.reveal_selection());
        assert!(document.fold_state().collapsed_at(0).is_none());
        assert!(document.fold_state().collapsed_at(3).is_some());
        assert!(!document.reveal_selection());
        assert_eq!(document.text(), source);
        assert!(!document.is_dirty());
    }

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

mod paint_cache;
pub use paint_cache::PaintCache;

mod row_geometry;
pub use row_geometry::{
    GlyphRectangle, HorizontalGeometry, MAX_ROW_GEOMETRY_ANCHORS, MeasuredRowGeometry,
    WrappedGeometry,
};
