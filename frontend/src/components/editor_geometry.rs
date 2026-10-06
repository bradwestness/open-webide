//! Browser text measurements for shared visual-row cursor movement.
use super::editor::current_editor_target;
use crate::state_actions::editor::EditorActions;
use leptos::prelude::*;
use openwebide_core::editor::{MAX_VISUAL_CARETS, VisualCaret, VisualLayout, visual_line_offsets};
use std::collections::{BTreeMap, BTreeSet};

/// DOM values are rounded first and bounded well inside exact integer precision.
#[allow(
    clippy::cast_possible_truncation,
    reason = "The finite rounded DOM value is range checked before its integer conversion"
)]
fn measured_integer(value: f64) -> Option<i64> {
    if !value.is_finite() || value.abs() > 1_000_000_000_000.0 {
        return None;
    }
    Some(value as i64)
}
fn measured_row(value: f64) -> Option<usize> {
    usize::try_from(measured_integer(value)?).ok()
}

struct TextNodes(Vec<(web_sys::Node, u32, u32)>);
impl TextNodes {
    fn new(row: &web_sys::Element) -> Option<Self> {
        let mut pending = vec![web_sys::Node::from(row.clone())];
        let mut result = Vec::new();
        let mut offset = 0_u32;
        let mut visited = 0;
        while let Some(node) = pending.pop() {
            visited += 1;
            if visited > MAX_VISUAL_CARETS {
                return None;
            }
            if node.node_type() == web_sys::Node::TEXT_NODE {
                let length = u32::try_from(node.node_value()?.encode_utf16().count()).ok()?;
                result.push((node, offset, length));
                offset = offset.checked_add(length)?;
            } else {
                let children = node.child_nodes();
                pending.extend(
                    (0..children.length())
                        .rev()
                        .filter_map(|index| children.item(index)),
                );
            }
        }
        Some(Self(result))
    }
    fn position(&self, offset: u32) -> Option<(&web_sys::Node, u32)> {
        let index = self
            .0
            .partition_point(|(_, start, length)| start + length <= offset);
        let (node, start, length) = self.0.get(index).or_else(|| self.0.last())?;
        let at = offset.checked_sub(*start)?;
        (at <= *length).then_some((node, at))
    }
}

struct Glyphs {
    offsets: Vec<(usize, usize)>,
    nodes: TextNodes,
    range: web_sys::Range,
    measured: BTreeMap<usize, web_sys::DomRect>,
}
impl Glyphs {
    fn new(row: &web_sys::Element, body: &str) -> Option<Self> {
        Some(Self {
            offsets: visual_line_offsets(body).ok()?,
            nodes: TextNodes::new(row)?,
            range: document().create_range().ok()?,
            measured: BTreeMap::new(),
        })
    }
    fn rect(&mut self, index: usize) -> Option<web_sys::DomRect> {
        if let Some(rect) = self.measured.get(&index) {
            return Some(rect.clone());
        }
        if self.measured.len() >= MAX_VISUAL_CARETS {
            return None;
        }
        let (start_node, start_at) = self
            .nodes
            .position(u32::try_from(self.offsets.get(index)?.1).ok()?)?;
        let (end_node, end_at) = self
            .nodes
            .position(u32::try_from(self.offsets.get(index + 1)?.1).ok()?)?;
        self.range.set_start(start_node, start_at).ok()?;
        self.range.set_end(end_node, end_at).ok()?;
        let rects = self.range.get_client_rects()?;
        let rect = (0..rects.length())
            .filter_map(|index| rects.item(index))
            .find(|rect| rect.height() > 0.0)?;
        self.measured.insert(index, rect.clone());
        Some(rect)
    }
    fn row(&mut self, index: usize, metrics: &VisualMetrics) -> Option<usize> {
        measured_row(((self.rect(index)?.top() - metrics.top) / metrics.line_height).floor())
    }
    // Visual rows increase in source order even when a row contains bidi text.
    fn first_on_row(&mut self, minimum: usize, metrics: &VisualMetrics) -> Option<usize> {
        let mut start = 0;
        let mut end = self.offsets.len().checked_sub(1)?;
        while start < end {
            let middle = start + (end - start) / 2;
            if self.row(middle, metrics)? < minimum {
                start = middle + 1;
            } else {
                end = middle;
            }
        }
        Some(start)
    }
}

pub(super) struct VisualMetrics {
    pub identity: String,
    pub left: f64,
    pub top: f64,
    pub line_height: f64,
    pub caret_height: f64,
    pub caret_inset: f64,
    pub rows: usize,
}

pub(super) fn visual_metrics(input: &web_sys::HtmlTextAreaElement) -> Option<VisualMetrics> {
    let parent = input.parent_element()?;
    let first = parent.query_selector(".editor-source-line").ok()??;
    let last = parent
        .query_selector(".editor-source-line:last-child")
        .ok()??;
    let first_rect = first.get_bounding_client_rect();
    let last_rect = last.get_bounding_client_rect();
    let style = window().get_computed_style(input).ok()??;
    let line_height: f64 = style
        .get_property_value("line-height")
        .ok()?
        .trim_end_matches("px")
        .parse()
        .ok()?;
    if !line_height.is_finite() || line_height <= 0.0 {
        return None;
    }
    let identity = format!(
        "{}:{}:{}:{}",
        first_rect.width(),
        style.get_property_value("font").ok()?,
        line_height,
        style.get_property_value("tab-size").ok()?
    );
    let caret = super::editor::caret_rect(&first, 0).filter(|rect| rect.height() > 0.0);
    Some(VisualMetrics {
        identity,
        left: first_rect.left(),
        top: first_rect.top(),
        line_height,
        caret_height: caret.as_ref().map_or(line_height, web_sys::DomRect::height),
        caret_inset: caret
            .as_ref()
            .map_or(0.0, |caret| caret.top() - first_rect.top()),
        rows: measured_row(((last_rect.bottom() - first_rect.top()) / line_height).round())?,
    })
}

pub(super) fn visual_layout(
    actions: EditorActions,
    input: &web_sys::HtmlTextAreaElement,
) -> Option<VisualLayout> {
    if !current_editor_target(actions, input) {
        return None;
    }
    let source = actions.source();
    let projection = actions.projection()?;
    let parent = input.parent_element()?;
    let paint = parent.query_selector(".editor-highlight-content").ok()??;
    if !parent.class_list().contains("highlight-ready")
        || paint.text_content()? != projection.textarea_text()
    {
        return None;
    }
    let metrics = visual_metrics(input)?;
    if metrics.rows == 0 {
        return None;
    }
    let row_element = |index: usize| {
        parent
            .query_selector(&format!(
                ".editor-source-line[data-line='{}']",
                projection.lines().get(index)?.source_line + 1
            ))
            .ok()
            .flatten()
    };
    let selections = actions.selections(&source);
    let mut selected_rows = BTreeSet::new();
    for selection in &selections {
        let visible = projection.visible_selection(*selection).ok()?;
        let index = projection
            .lines()
            .partition_point(|line| line.visible_start <= visible.head)
            .saturating_sub(1);
        selected_rows
            .extend(index.saturating_sub(1)..=(index + 1).min(projection.lines().len() - 1));
    }
    let mut prepared = BTreeMap::new();
    for index in selected_rows {
        let line = &projection.lines()[index];
        let row = row_element(index)?;
        let raw = &source[line.source.clone()];
        let body = raw
            .strip_suffix("\r\n")
            .or_else(|| raw.strip_suffix('\n'))
            .unwrap_or(raw);
        let glyphs = if body.is_empty() {
            None
        } else {
            Some(Glyphs::new(&row, body)?)
        };
        prepared.insert(index, (row, glyphs));
    }
    let mut wanted = BTreeSet::new();
    for (caret, selection) in selections.iter().enumerate() {
        let visible = projection.visible_selection(*selection).ok()?;
        let index = projection
            .lines()
            .partition_point(|line| line.visible_start <= visible.head)
            .saturating_sub(1);
        let line = &projection.lines()[index];
        let (row, glyphs) = prepared.get_mut(&index)?;
        let current = if let Some(caret) = actions.visual_caret(&source, caret, &metrics.identity) {
            caret.row
        } else if glyphs.is_none() {
            measured_row(
                ((row.get_bounding_client_rect().top() - metrics.top) / metrics.line_height)
                    .round(),
            )?
        } else {
            let glyphs = glyphs.as_mut()?;
            let byte = visible
                .head
                .saturating_sub(line.visible_start)
                .min(glyphs.offsets.last()?.0);
            let glyph = glyphs
                .offsets
                .partition_point(|(offset, _)| *offset <= byte)
                .saturating_sub(1)
                .min(glyphs.offsets.len() - 2);
            glyphs.row(glyph, &metrics)?
        };
        wanted.extend(current.saturating_sub(1)..=(current + 1).min(metrics.rows - 1));
    }
    let mut carets = Vec::new();
    for (index, (row, glyphs)) in prepared {
        let line = &projection.lines()[index];
        let Some(mut glyphs) = glyphs else {
            let bounds = row.get_bounding_client_rect();
            carets.push(VisualCaret {
                offset: line.visible_start,
                column: 0,
                row: measured_row(((bounds.top() - metrics.top) / metrics.line_height).round())?,
            });
            continue;
        };
        for &wanted in &wanted {
            let start = glyphs.first_on_row(wanted, &metrics)?;
            let end = glyphs.first_on_row(wanted + 1, &metrics)?;
            if carets.len() + (end - start) * 2 > MAX_VISUAL_CARETS {
                return None;
            }
            for index in start..end {
                let rect = glyphs.rect(index)?;
                for (offset, x) in [
                    (glyphs.offsets[index].0, rect.left()),
                    (glyphs.offsets[index + 1].0, rect.right()),
                ] {
                    carets.push(VisualCaret {
                        offset: line.visible_start + offset,
                        column: measured_integer(((x - metrics.left) * 64.0).round())?,
                        row: wanted,
                    });
                }
            }
        }
    }
    VisualLayout::new(&source, projection, metrics.identity, metrics.rows, carets).ok()
}
