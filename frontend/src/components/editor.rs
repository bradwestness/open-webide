use leptos::prelude::*;
use openwebide_core::{
    FileDiff, diff_inline_lines, diff_side_by_side,
    highlight::{Language, TokenKind, highlight_lines, language_from_path},
};
use web_sys::wasm_bindgen::JsCast;

use crate::components::chat_pane::render_markdown;

/// How the open file is displayed. When the file has a pending agent edit the
/// editor defaults to the inline diff; the user can switch between the four
/// modes. Without a pending edit the file is shown as an editable textarea.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    InlineDiff,
    SideBySide,
    Content,
    Preview,
}

fn mode_class(view_mode: ReadSignal<ViewMode>, target: ViewMode) -> String {
    if view_mode.get() == target {
        "mode-opt active".to_string()
    } else {
        "mode-opt".to_string()
    }
}

/// The CSS class for a token category.
fn token_class(kind: TokenKind) -> &'static str {
    match kind {
        TokenKind::Plain => "tok-plain",
        TokenKind::Keyword => "tok-keyword",
        TokenKind::Type => "tok-type",
        TokenKind::String => "tok-string",
        TokenKind::Char => "tok-char",
        TokenKind::Comment => "tok-comment",
        TokenKind::Number => "tok-number",
        TokenKind::Function => "tok-function",
        TokenKind::Operator => "tok-operator",
        TokenKind::Punct => "tok-punct",
        TokenKind::Attribute => "tok-attribute",
        TokenKind::Macro => "tok-macro",
        TokenKind::Lifetime => "tok-lifetime",
        TokenKind::Boolean => "tok-boolean",
    }
}

/// Escape the few characters that are special in HTML element content.
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Render the highlighted source as an HTML string for the overlay: one line
/// per source line (newline-separated), each token wrapped in a colored span.
/// Plain tokens are emitted bare so they don't add span overhead.
fn highlight_html(source: &str, language: Language) -> String {
    let lines = highlight_lines(source, language);
    let mut html = String::new();
    for (idx, line) in lines.iter().enumerate() {
        if idx > 0 {
            html.push('\n');
        }
        for tok in line {
            match tok.kind {
                TokenKind::Plain => html.push_str(&escape_html(&tok.text)),
                kind => {
                    html.push_str("<span class=\"");
                    html.push_str(token_class(kind));
                    html.push_str("\">");
                    html.push_str(&escape_html(&tok.text));
                    html.push_str("</span>");
                }
            }
        }
    }
    html
}

/// Render the changed middle of a file edit as inline removed/added lines.
fn render_inline_diff(diff: FileDiff) -> impl IntoView {
    let lines = diff_inline_lines(&diff);
    view! {
        <div class="editor-diff editor-diff-inline">
            {lines
                .into_iter()
                .map(|(mark, line)| {
                    let text = format!("{mark} {line}");
                    let line_class = if mark == '+' {
                        "diff-line add".to_string()
                    } else {
                        "diff-line del".to_string()
                    };
                    view! {
                        <div class=line_class>{text}</div>
                    }
                })
                .collect::<Vec<_>>()}
        </div>
    }
}

/// Render the changed middle of a file edit as aligned side-by-side rows.
fn render_side_by_side(diff: FileDiff) -> impl IntoView {
    let rows = diff_side_by_side(&diff);
    view! {
        <div class="editor-diff editor-diff-side">
            {rows
                .into_iter()
                .map(|(left, right)| {
                    let (left_class, right_class) = match (&left, &right) {
                        (Some(o), Some(n)) if o == n => ("sbs-cell", "sbs-cell"),
                        (Some(_), Some(_)) => ("sbs-cell sbs-del", "sbs-cell sbs-add"),
                        (Some(_), None) => ("sbs-cell sbs-del", "sbs-cell sbs-empty"),
                        (None, Some(_)) => ("sbs-cell sbs-empty", "sbs-cell sbs-add"),
                        (None, None) => ("sbs-cell sbs-empty", "sbs-cell sbs-empty"),
                    };
                    let left_text = left.unwrap_or_default();
                    let right_text = right.unwrap_or_default();
                    view! {
                        <div class="sbs-row">
                            <div class=left_class>{left_text}</div>
                            <div class=right_class>{right_text}</div>
                        </div>
                    }
                })
                .collect::<Vec<_>>()}
        </div>
    }
}

/// Render the full updated contents as plain text.
fn render_content_view(content: String) -> impl IntoView {
    view! {
        <pre class="editor-content-view">{content}</pre>
    }
}

/// Render the updated contents as markdown (images and links included).
fn render_preview_view(content: String) -> impl IntoView {
    let html = render_markdown(&content);
    view! {
        <div class="editor-preview markdown" inner_html=html />
    }
}

/// The code editor: a full-height textarea bound to the open file's content,
/// with a header showing the path, a dirty indicator, and a Save button. When
/// the open file has a pending agent edit, the header switches to the diff
/// controls (display-mode toggle + Accept/Reject) and the body shows the edit
/// in the selected display mode instead of the editable textarea.
#[component]
pub fn Editor(
    open_file: ReadSignal<Option<String>>,
    content: ReadSignal<String>,
    set_content: WriteSignal<String>,
    dirty: ReadSignal<bool>,
    set_dirty: WriteSignal<bool>,
    pending_diff: ReadSignal<Option<FileDiff>>,
    on_save: Callback<()>,
    on_accept: Callback<()>,
    on_reject: Callback<()>,
    error: ReadSignal<Option<String>>,
) -> impl IntoView {
    let ta = NodeRef::<leptos::html::Textarea>::new();
    let hl = NodeRef::<leptos::html::Div>::new();
    let view_mode = RwSignal::new(ViewMode::InlineDiff);

    // When a pending edit appears, default to the inline diff view.
    Effect::new(move || {
        if pending_diff.get().is_some() {
            view_mode.set(ViewMode::InlineDiff);
        }
    });

    // Mirror the loaded content into the textarea without clobbering the
    // user's in-progress edits (same pattern as the chat composer).
    Effect::new(move || {
        let value = content.get();
        if let Some(el) = ta.get()
            && el.value() != value
        {
            el.set_value(&value);
        }
    });

    view! {
        <div class="editor">
            <div class="editor-header">
                <span class="editor-path" title="Open file">
                    {move || match open_file.get() {
                        Some(p) => {
                            if dirty.get() {
                                format!("{p} ●")
                            } else {
                                p
                            }
                        }
                        None => "No file open".to_string(),
                    }}
                </span>
                <Show
                    when=move || pending_diff.get().is_some()
                    fallback=move || {
                        view! {
                            <button
                                class="btn editor-save"
                                disabled=move || !dirty.get()
                                on:click=move |_| on_save.run(())
                            >
                                "Save"
                            </button>
                        }
                    }
                >
                    <div class="editor-diff-actions">
                        <div class="editor-mode-toggle">
                            <button
                                class=move || mode_class(view_mode.read_only(), ViewMode::InlineDiff)
                                on:click=move |_| view_mode.set(ViewMode::InlineDiff)
                            >
                                "Inline"
                            </button>
                            <button
                                class=move || mode_class(view_mode.read_only(), ViewMode::SideBySide)
                                on:click=move |_| view_mode.set(ViewMode::SideBySide)
                            >
                                "Split"
                            </button>
                            <button
                                class=move || mode_class(view_mode.read_only(), ViewMode::Content)
                                on:click=move |_| view_mode.set(ViewMode::Content)
                            >
                                "Content"
                            </button>
                            <button
                                class=move || mode_class(view_mode.read_only(), ViewMode::Preview)
                                on:click=move |_| view_mode.set(ViewMode::Preview)
                            >
                                "Preview"
                            </button>
                        </div>
                        <button class="btn editor-accept" on:click=move |_| on_accept.run(())>
                            "Accept"
                        </button>
                        <button class="btn editor-reject" on:click=move |_| on_reject.run(())>
                            "Reject"
                        </button>
                    </div>
                </Show>
            </div>
            <Show when=move || error.get().is_some() fallback=|| ()>
                <div class="editor-error">{move || error.get().unwrap_or_default()}</div>
            </Show>
            <Show
                when=move || open_file.get().is_some()
                fallback=move || {
                    view! {
                        <p class="empty editor-empty">"Select a file to edit, or create a new one in the explorer."</p>
                    }
                }
            >
                <Show
                    when=move || pending_diff.get().is_some()
                    fallback=move || {
                        view! {
                            <div class="editor-code">
                                <div
                                    class="editor-highlight"
                                    node_ref=hl
                                    inner_html=move || {
                                        let lang = open_file
                                            .get()
                                            .as_deref()
                                            .map(language_from_path)
                                            .unwrap_or(Language::Plain);
                                        highlight_html(&content.get(), lang)
                                    }
                                />
                                <textarea
                                    class="editor-textarea"
                                    spellcheck="false"
                                    node_ref=ta
                                    on:input=move |e: web_sys::Event| {
                                        if let Some(target) = e.target()
                                            && let Some(textarea) = target.dyn_ref::<web_sys::HtmlTextAreaElement>()
                                        {
                                            set_content.set(textarea.value());
                                            set_dirty.set(true);
                                        }
                                    }
                                    on:scroll=move |_| {
                                        if let (Some(ta_el), Some(hl_el)) = (ta.get(), hl.get()) {
                                            hl_el.set_scroll_top(ta_el.scroll_top());
                                            hl_el.set_scroll_left(ta_el.scroll_left());
                                        }
                                    }
                                />
                            </div>
                        }
                    }
                >
                    <Show when=move || view_mode.get() == ViewMode::InlineDiff fallback=|| ()>
                        {move || {
                            let diff = pending_diff.get().unwrap();
                            render_inline_diff(diff)
                        }}
                    </Show>
                    <Show when=move || view_mode.get() == ViewMode::SideBySide fallback=|| ()>
                        {move || {
                            let diff = pending_diff.get().unwrap();
                            render_side_by_side(diff)
                        }}
                    </Show>
                    <Show when=move || view_mode.get() == ViewMode::Content fallback=|| ()>
                        {move || {
                            let diff = pending_diff.get().unwrap();
                            render_content_view(diff.new)
                        }}
                    </Show>
                    <Show when=move || view_mode.get() == ViewMode::Preview fallback=|| ()>
                        {move || {
                            let diff = pending_diff.get().unwrap();
                            render_preview_view(diff.new)
                        }}
                    </Show>
                </Show>
            </Show>
        </div>
    }
}
