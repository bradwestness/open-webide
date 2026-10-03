use leptos::prelude::*;
use openwebide_frontend::components::highlight_count;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;

use super::support::{Mounted, editor_view, mount_test, settle};

fn mount_editor(content: String) -> Mounted {
    mount_test(move |state| {
        state.seed_project();
        state.workspace.open_file.set(Some("fixture.rs".into()));
        state.workspace.content.set(content);
        editor_view(state)
    })
}

fn assert_scroll_aligned(mounted: &Mounted, textarea: &web_sys::HtmlTextAreaElement) {
    let overlay = mounted.element(".editor-highlight");
    assert_eq!(
        overlay
            .style()
            .get_property_value("--editor-scroll-y")
            .unwrap(),
        format!("{}px", -textarea.scroll_top())
    );
    assert_eq!(
        overlay
            .style()
            .get_property_value("--editor-scroll-x")
            .unwrap(),
        format!("{}px", -textarea.scroll_left())
    );
    assert!(overlay.scroll_top().abs() < f64::EPSILON);
    assert!(overlay.scroll_left().abs() < f64::EPSILON);
}

fn input(mounted: &Mounted, value: &str) -> web_sys::HtmlTextAreaElement {
    let textarea: web_sys::HtmlTextAreaElement =
        mounted.element(".editor-textarea").unchecked_into();
    textarea.set_value(value);
    textarea
        .dispatch_event(&web_sys::Event::new("input").unwrap())
        .unwrap();
    textarea
}

async fn frame() {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        leptos::leptos_dom::helpers::request_animation_frame(move || {
            resolve.call0(&wasm_bindgen::JsValue::NULL).unwrap();
        });
    });
    JsFuture::from(promise).await.unwrap();
    settle().await;
}

fn now() -> f64 {
    js_sys::Function::new_no_args("return performance.now()")
        .call0(&wasm_bindgen::JsValue::NULL)
        .unwrap()
        .as_f64()
        .unwrap()
}

#[wasm_bindgen_test]
async fn measure_highlight_bursts() {
    for lines in [1_000, 10_000] {
        let source = "fn example() { let value = 42; }\n".repeat(lines);
        let mounted = mount_editor(source.clone());
        settle().await;
        frame().await;
        let mut latency = Vec::new();
        let mut frames = Vec::new();
        let mut counts = Vec::new();
        for run in 0..5 {
            let before = highlight_count();
            let start = now();
            for event in 0..10 {
                let text = format!("{source}// burst {run} input {event}\n");
                let start = now();
                input(&mounted, &text);
                settle().await;
                latency.push(now() - start);
            }
            frame().await;
            frames.push(now() - start);
            counts.push(highlight_count() - before);
        }
        assert!(counts.iter().all(|count| *count == 1));
        latency.sort_by(f64::total_cmp);
        frames.sort_by(f64::total_cmp);
        console_log!(
            "editor {lines} lines: executions/10 inputs={counts:?}, input+microtasks median={:.3}ms p95={:.3}ms, burst-to-frame median={:.3}ms p95={:.3}ms",
            latency[25],
            latency[47],
            frames[2],
            frames[4]
        );
    }
}

#[wasm_bindgen_test]
async fn inputs_coalesce_without_delaying_edits_or_save() {
    use openwebide_frontend::testing::fake_backend::Call;

    let mounted = mount_editor("fn original() {}\n".into());
    settle().await;
    frame().await;
    let textarea: web_sys::HtmlTextAreaElement =
        mounted.element(".editor-textarea").unchecked_into();
    textarea.focus().unwrap();
    let before = highlight_count();
    for text in ["fn first() {}", "fn final_name() { /* café <>& */ }\n"] {
        input(&mounted, text);
        assert_eq!(mounted.state.workspace.content.get_untracked(), text);
        assert!(mounted.state.workspace.dirty.get_untracked());
        settle().await;
    }
    textarea.set_selection_range(3, 8).unwrap();
    assert_eq!(highlight_count(), before);
    mounted.click_text("Save");
    settle().await;
    assert!(mounted.state.fake.calls.borrow().iter().any(|call| {
        matches!(call, Call::WriteFile { path, content }
            if path == "fixture.rs" && content == "fn final_name() { /* café <>& */ }\n")
    }));
    assert_eq!(highlight_count(), before);
    frame().await;
    assert_eq!(highlight_count(), before + 1);
    assert_eq!(
        mounted.element(".editor-highlight").text_content().unwrap(),
        textarea.value()
    );
    assert!(
        mounted
            .element(".editor-highlight")
            .inner_html()
            .contains("&lt;&gt;&amp;")
    );
    assert!(textarea.is_same_node(Some(&mounted.element(".editor-textarea"))));
    assert!(
        textarea.is_same_node(
            web_sys::window()
                .unwrap()
                .document()
                .unwrap()
                .active_element()
                .as_ref()
                .map(AsRef::as_ref)
        )
    );
    assert_eq!(textarea.selection_start().unwrap(), Some(3));
    assert_eq!(textarea.selection_end().unwrap(), Some(8));
}

#[wasm_bindgen_test]
async fn file_switch_reads_latest_path_and_mode_exit_cancels() {
    let mounted = mount_editor("fn original() {}".into());
    settle().await;
    frame().await;
    let before = highlight_count();
    input(&mounted, "fn stale() {}");
    settle().await;
    mounted
        .state
        .workspace
        .open_file
        .set(Some("plain.txt".into()));
    mounted.state.workspace.content.set("fn plain <>&\n".into());
    settle().await;
    frame().await;
    assert_eq!(highlight_count(), before + 1);
    assert_eq!(
        mounted.element(".editor-highlight-content").inner_html(),
        "fn plain <span class=\"tok-operator\">&lt;&gt;&amp;</span>\n"
    );
    let before = highlight_count();
    input(&mounted, "new text");
    settle().await;
    mounted.click_text("Preview");
    settle().await;
    frame().await;
    assert_eq!(highlight_count(), before);
    assert!(
        mounted
            .root
            .query_selector(".editor-highlight")
            .unwrap()
            .is_none()
    );
    mounted.click_text("Edit");
    settle().await;
    frame().await;
    assert_eq!(
        mounted.element(".editor-highlight").text_content().unwrap(),
        "new text"
    );
}

#[wasm_bindgen_test]
async fn unmount_cancels_pending_highlighting() {
    let mounted = mount_editor("fn original() {}".into());
    settle().await;
    frame().await;
    let before = highlight_count();
    input(&mounted, "fn cancelled() {}");
    settle().await;
    drop(mounted);
    frame().await;
    assert_eq!(highlight_count(), before);
}

#[wasm_bindgen_test]
async fn growing_paste_preserves_scroll_after_frame() {
    let source = (0..100)
        .map(|_| "let value = 42;")
        .collect::<Vec<_>>()
        .join("\n");
    let mounted = mount_editor(source.clone());
    settle().await;
    frame().await;
    for element in [
        mounted.element(".editor-textarea"),
        mounted.element(".editor-highlight"),
    ] {
        element
            .set_attribute(
                "style",
                "display:block;box-sizing:content-box;width:80px;height:100px;padding:0;border:0;overflow:scroll;white-space:pre;font:16px/20px monospace",
            )
            .unwrap();
    }
    let textarea: web_sys::HtmlTextAreaElement =
        mounted.element(".editor-textarea").unchecked_into();
    textarea.set_attribute("wrap", "off").unwrap();
    textarea.set_scroll_top(f64::from(textarea.scroll_height()));
    textarea
        .dispatch_event(&web_sys::Event::new("scroll").unwrap())
        .unwrap();
    let overlay = mounted.element(".editor-highlight");

    input(&mounted, &format!("{source}\n{source}{}", "x".repeat(200)));
    settle().await;
    textarea.set_scroll_top(f64::from(textarea.scroll_height()));
    textarea.set_scroll_left(500.0);
    textarea
        .dispatch_event(&web_sys::Event::new("scroll").unwrap())
        .unwrap();
    assert!(textarea.scroll_top() > overlay.scroll_top());
    assert!(textarea.scroll_left() > overlay.scroll_left());
    frame().await;
    assert_scroll_aligned(&mounted, &textarea);
    assert!(
        mounted
            .element(".editor-code")
            .class_list()
            .contains("highlight-ready")
    );
}

#[wasm_bindgen_test]
async fn overlay_preserves_empty_unicode_long_lines_and_scroll_mirror() {
    let mounted = mount_editor(String::new());
    settle().await;
    frame().await;
    assert_eq!(
        mounted.element(".editor-highlight").text_content().unwrap(),
        ""
    );
    let text = format!("/*\n café <>&\n*/\n\t{}\n", "a".repeat(10_001));
    let textarea = input(&mounted, &text);
    settle().await;
    frame().await;
    assert_eq!(
        mounted.element(".editor-highlight").text_content().unwrap(),
        text
    );
    assert!(
        mounted
            .element(".editor-highlight")
            .inner_html()
            .contains("tok-comment")
    );
    // Give both layers real scroll extents without depending on application CSS.
    for element in [
        mounted.element(".editor-textarea"),
        mounted.element(".editor-highlight"),
    ] {
        element
            .set_attribute(
                "style",
                "display:block;width:80px;height:20px;overflow:scroll;white-space:pre",
            )
            .unwrap();
    }
    textarea.set_scroll_top(10.0);
    textarea.set_scroll_left(20.0);
    textarea
        .dispatch_event(&web_sys::Event::new("scroll").unwrap())
        .unwrap();
    assert_scroll_aligned(&mounted, &textarea);
}

#[wasm_bindgen_test]
async fn rapid_bottom_and_reverse_scrolling_uses_one_scroll_source() {
    for trailing_newline in [false, true] {
        let source = format!(
            "{}{}",
            "fn example() { let value = 42; }\n".repeat(100),
            if trailing_newline {
                "\n"
            } else {
                "fn last() {}"
            }
        );
        let mounted = mount_editor(source);
        let style = document().create_element("style").unwrap();
        let css = include_str!("../../styles.css");
        let start = css.find(".editor-code {").unwrap();
        let end = css[start..].find("/* syntax highlight").unwrap() + start;
        style.set_text_content(Some(&format!("{}\n.editor-code {{ width: 180px; height: 120px; flex: none; --mono: monospace; --text: black; }} .editor-code * {{ box-sizing: border-box; }}", &css[start..end])));
        mounted.root.append_child(&style).unwrap();
        settle().await;
        frame().await;
        let textarea: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        assert!(textarea.scroll_height() > textarea.client_height());
        for offset in [1e6, 0.0, 600.0, 1e6, 240.0, 1e6, 0.0] {
            textarea.set_scroll_top(offset);
            textarea
                .dispatch_event(&web_sys::Event::new("scroll").unwrap())
                .unwrap();
            assert_scroll_aligned(&mounted, &textarea);
        }
        frame().await;
        assert_scroll_aligned(&mounted, &textarea);
        let computed = window().get_computed_style(&textarea).unwrap().unwrap();
        assert_eq!(
            computed.get_property_value("color").unwrap(),
            "rgba(0, 0, 0, 0)"
        );
        assert!(
            mounted
                .element(".editor-code")
                .class_list()
                .contains("highlight-ready")
        );
    }
}
