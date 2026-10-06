//! DOM positions use the shared coverage map, including unpainted source gaps.
use super::editor::raw_text_position;
use leptos::prelude::*;
use openwebide_core::editor::PaintCoverage;
use std::ops::Range;
use wasm_bindgen::JsCast;

struct PaintNodes {
    coverage: PaintCoverage,
    nodes: Vec<web_sys::Node>,
}
impl PaintNodes {
    fn read(row: &web_sys::Element) -> Option<Self> {
        let fragments = row
            .query_selector_all(":scope > .editor-source-fragment")
            .ok()?;
        let length = row.get_attribute("data-paint-length")?.parse().ok()?;
        let mut nodes = Vec::new();
        let mut ranges = Vec::new();
        for index in 0..fragments.length() {
            let element = fragments.item(index)?.dyn_into::<web_sys::Element>().ok()?;
            let start: usize = element.get_attribute("data-paint-start")?.parse().ok()?;
            let end: usize = element.get_attribute("data-paint-end")?.parse().ok()?;
            if end.checked_sub(start)? != native_length(element.as_ref())? {
                return None;
            }
            ranges.push(start..end);
            nodes.push(element.into());
        }
        Some(Self {
            coverage: PaintCoverage::new(length, ranges).ok()?,
            nodes,
        })
    }
}

fn native_length(node: &web_sys::Node) -> Option<usize> {
    if let Some(text) = node.dyn_ref::<web_sys::Text>() {
        return usize::try_from(text.length()).ok();
    }
    let children = node.child_nodes();
    (0..children.length()).try_fold(0_usize, |length, index| {
        length.checked_add(native_length(&children.item(index)?)?)
    })
}

pub(super) fn position(node: &web_sys::Node, offset: u32) -> Option<(web_sys::Node, u32)> {
    if let Some(row) = node.dyn_ref::<web_sys::Element>()
        && row.has_attribute("data-paint-length")
    {
        let paint = PaintNodes::read(row)?;
        let at = paint.coverage.caret(offset as usize)?;
        return raw_text_position(
            &paint.nodes[at.fragment],
            &mut u32::try_from(at.offset).ok()?,
        );
    }
    raw_text_position(node, &mut { offset })
}

pub(super) fn covers(row: &web_sys::Element, offset: u32) -> bool {
    !row.has_attribute("data-paint-length")
        || PaintNodes::read(row)
            .is_some_and(|paint| paint.coverage.caret(offset as usize).is_some())
}

pub(super) fn ranges(row: &web_sys::Element, selected: Range<u32>) -> Vec<web_sys::Range> {
    if !row.has_attribute("data-paint-length") {
        return dom_range(row.as_ref(), selected).into_iter().collect();
    }
    let Some(paint) = PaintNodes::read(row) else {
        return Vec::new();
    };
    let Ok(selected) = paint
        .coverage
        .selection(selected.start as usize..selected.end as usize)
    else {
        return Vec::new();
    };
    selected
        .into_iter()
        .filter_map(|selected| {
            let node = &paint.nodes[selected.fragment];
            dom_range(
                node,
                u32::try_from(selected.range.start).ok()?
                    ..u32::try_from(selected.range.end).ok()?,
            )
        })
        .collect()
}

fn dom_range(node: &web_sys::Node, selected: Range<u32>) -> Option<web_sys::Range> {
    let mut first = selected.start;
    let mut last = selected.end;
    let (start, at) = raw_text_position(node, &mut first)?;
    let (end, to) = raw_text_position(node, &mut last)?;
    let range = document().create_range().ok()?;
    range.set_start(&start, at).ok()?;
    range.set_end(&end, to).ok()?;
    Some(range)
}

/// DOM prefix measurement is a browser primitive; interval mapping belongs to
/// PaintCoverage. Never count omitted fragments as if their text were present.
pub(crate) fn native_offset(
    paint: &web_sys::Element,
    node: &web_sys::Node,
    offset: u32,
) -> Option<u32> {
    if !paint.contains(Some(node)) {
        return None;
    }
    let element = node
        .dyn_ref::<web_sys::Element>()
        .cloned()
        .or_else(|| node.parent_element())?;
    if let Some(row) = element
        .closest(".editor-source-line[data-paint-length]")
        .ok()?
    {
        let mapping = PaintNodes::read(&row)?;
        let index = mapping
            .nodes
            .iter()
            .position(|fragment| fragment.contains(Some(node)))?;
        let range = document().create_range().ok()?;
        range.select_node_contents(&mapping.nodes[index]).ok()?;
        range.set_end(node, offset).ok()?;
        let within = usize::try_from(range.to_string().length()).ok()?;
        let column = mapping.coverage.source_offset(index, within)?;
        let start: usize = row.get_attribute("data-textarea-start")?.parse().ok()?;
        return u32::try_from(start.checked_add(column)?).ok();
    }
    let range = document().create_range().ok()?;
    range.select_node_contents(paint).ok()?;
    range.set_end(node, offset).ok()?;
    let start: u32 = paint
        .get_attribute("data-textarea-start")
        .as_deref()
        .unwrap_or("0")
        .parse()
        .ok()?;
    start.checked_add(range.to_string().length())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    #[wasm_bindgen_test]
    fn painted_fragments_preserve_utf16_coordinates_and_selection_gaps() {
        let row = document().create_element("span").unwrap();
        row.set_class_name("editor-source-line");
        row.set_attribute("data-paint-length", "16").unwrap();
        row.set_inner_html(concat!(
            "<span class='editor-source-fragment' data-paint-start='2' data-paint-end='7'>",
            "<span>日🦀</span><span>ab</span></span>",
            "<span class='editor-source-fragment' data-paint-start='10' data-paint-end='16'>",
            "<span>cd</span><span>🌍ef</span></span>"
        ));
        let paint = document().create_element("div").unwrap();
        row.set_attribute("data-textarea-start", "100").unwrap();
        paint.append_child(&row).unwrap();
        let node: web_sys::Node = row.clone().into();
        for offset in [0, 1, 7, 8, 9, 17] {
            assert!(!covers(&row, offset));
            assert!(position(&node, offset).is_none());
        }
        let (text, at) = position(&node, 5).unwrap();
        assert_eq!(text.node_value().as_deref(), Some("ab"));
        assert_eq!(at, 0);
        let (text, at) = position(&node, 16).unwrap();
        assert_eq!(text.node_value().as_deref(), Some("🌍ef"));
        assert_eq!(at, 4);
        assert_eq!(native_offset(&paint, &text, at), Some(116));
        let second = row
            .query_selector(".editor-source-fragment:last-child span:first-child")
            .unwrap()
            .unwrap()
            .first_child()
            .unwrap();
        assert_eq!(native_offset(&paint, &second, 1), Some(111));
        let first = row
            .query_selector(".editor-source-fragment:first-child span:last-child")
            .unwrap()
            .unwrap()
            .first_child()
            .unwrap();
        assert_eq!(native_offset(&paint, &first, 2), Some(107));
        assert_eq!(native_offset(&paint, &first, 3), None);
        let outside = document().create_text_node("outside");
        assert_eq!(native_offset(&paint, outside.as_ref(), 0), None);
        let strings = |selected| {
            ranges(&row, selected)
                .into_iter()
                .map(|range| range.to_string().as_string().unwrap())
                .collect::<Vec<_>>()
        };
        assert_eq!(strings(0..16), ["日🦀ab", "cd🌍ef"]);
        assert_eq!(strings(3..14), ["🦀ab", "cd🌍"]);
        assert!(strings(7..10).is_empty());
        assert!(strings(8..8).is_empty());
        assert!(strings(0..17).is_empty());
        row.query_selector(".editor-source-fragment")
            .unwrap()
            .unwrap()
            .set_attribute("data-paint-end", "6")
            .unwrap();
        assert!(position(&node, 5).is_none());
        assert_eq!(native_offset(&paint, &first, 0), None);
        assert!(strings(0..16).is_empty());
    }

    #[wasm_bindgen_test]
    fn complete_legacy_rows_keep_token_boundaries_and_eof() {
        let row = document().create_element("span").unwrap();
        row.set_inner_html("<span>日🦀</span><span>ab</span>\n");
        let node: web_sys::Node = row.clone().into();
        let (text, at) = position(&node, 3).unwrap();
        assert_eq!(text.node_value().as_deref(), Some("ab"));
        assert_eq!(at, 0);
        let (text, at) = position(&node, 6).unwrap();
        assert_eq!(text.node_value().as_deref(), Some("\n"));
        assert_eq!(at, 1);
        assert!(covers(&row, 6));
        assert_eq!(
            ranges(&row, 1..5)[0].to_string().as_string().unwrap(),
            "🦀ab"
        );
        assert!(ranges(&row, 0..7).is_empty());
    }
}
