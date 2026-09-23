use leptos::prelude::*;
use openwebide_core::{
    DiffChunk, FileDiff, FileKind, diff_inline_detailed, diff_side_by_side_detailed,
    highlight::{Language, TokenKind, highlight_lines, language_from_path},
};
use web_sys::wasm_bindgen::JsCast;

use crate::components::chat_pane::render_markdown;
use crate::components::ui::{Button, ButtonSize, ButtonVariant, SegmentOption, SegmentedControl};

/// How the open file is displayed in the editor pane.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    /// Editable syntax-highlighted code editor.
    Code,
    /// Rendered preview (Markdown HTML, Image asset, or Binary placeholder).
    Preview,
    /// Agent edit inline diff.
    InlineDiff,
    /// Agent edit side-by-side split diff.
    SideBySide,
    /// Agent edit full updated content view.
    Content,
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

/// Render the highlighted source as an HTML string for the overlay.
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

/// Render a list of intra-line diff chunks with word-level highlights.
fn render_diff_chunks(chunks: Vec<DiffChunk>) -> impl IntoView {
    chunks
        .into_iter()
        .map(|chunk| match chunk {
            DiffChunk::Unchanged(text) => view! {
                <span>{text}</span>
            }
            .into_any(),
            DiffChunk::Deleted(text) => view! {
                <span class="diff-word-del">{text}</span>
            }
            .into_any(),
            DiffChunk::Inserted(text) => view! {
                <span class="diff-word-add">{text}</span>
            }
            .into_any(),
        })
        .collect::<Vec<_>>()
}

/// Render the changed middle of a file edit as inline removed/added lines with intra-line word diffs.
fn render_inline_diff(diff: FileDiff) -> impl IntoView {
    let lines = diff_inline_detailed(&diff);
    view! {
        <div class="editor-diff editor-diff-inline">
            {lines
                .into_iter()
                .map(|dl| {
                    let mark = dl.marker;
                    let line_class = if mark == '+' {
                        "diff-line add"
                    } else if mark == '-' {
                        "diff-line del"
                    } else {
                        "diff-line"
                    };
                    view! {
                        <div class=line_class>
                            <span class="diff-line-marker">{mark} " "</span>
                            {render_diff_chunks(dl.chunks)}
                        </div>
                    }
                })
                .collect::<Vec<_>>()}
        </div>
    }
}

/// Render the changed middle of a file edit as aligned side-by-side rows with intra-line word diffs.
fn render_side_by_side(diff: FileDiff) -> impl IntoView {
    let rows = diff_side_by_side_detailed(&diff);
    view! {
        <div class="editor-diff editor-diff-side">
            {rows
                .into_iter()
                .map(|(left, right)| {
                    let (left_class, right_class) = match (&left, &right) {
                        (Some(l), Some(r)) if l.content == r.content => ("sbs-cell", "sbs-cell"),
                        (Some(_), Some(_)) => ("sbs-cell sbs-del", "sbs-cell sbs-add"),
                        (Some(_), None) => ("sbs-cell sbs-del", "sbs-cell sbs-empty"),
                        (None, Some(_)) => ("sbs-cell sbs-empty", "sbs-cell sbs-add"),
                        (None, None) => ("sbs-cell sbs-empty", "sbs-cell sbs-empty"),
                    };
                    view! {
                        <div class="sbs-row">
                            <div class=left_class>
                                {left.map(|l| render_diff_chunks(l.chunks).into_any()).unwrap_or_else(|| ().into_any())}
                            </div>
                            <div class=right_class>
                                {right.map(|r| render_diff_chunks(r.chunks).into_any()).unwrap_or_else(|| ().into_any())}
                            </div>
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

/// Render a friendly placeholder for binary or non-previewable files.
fn render_placeholder_view(
    path: &str,
    kind: FileKind,
    set_view_mode: WriteSignal<ViewMode>,
    on_open_lossy: Callback<()>,
) -> impl IntoView {
    let file_name = path.rsplit('/').next().unwrap_or(path).to_string();
    let glyph = kind.glyph(path);
    let desc = kind.description(path);
    let path_clone = path.to_string();
    let copied = RwSignal::new(false);

    let on_copy = Callback::new(move |_| {
        let p = path_clone.clone();
        if let Some(clipboard) = web_sys::window().map(|w| w.navigator().clipboard()) {
            let _ = clipboard.write_text(&p);
            copied.set(true);
        }
    });

    view! {
        <div class="editor-placeholder-view">
            <div class="placeholder-icon">{glyph}</div>
            <div class="placeholder-title">{file_name}</div>
            <div class="placeholder-desc">{desc}</div>
            <p class="placeholder-hint">
                "This file is binary or not directly previewable in the text editor."
            </p>
            <div class="placeholder-actions">
                <Button
                    variant=ButtonVariant::Default
                    size=ButtonSize::Sm
                    on_click=on_copy
                >
                    {move || if copied.get() { "✓ Copied" } else { "Copy Path" }}
                </Button>
                <Button
                    variant=ButtonVariant::Ghost
                    size=ButtonSize::Sm
                    on_click=Callback::new(move |_| {
                        set_view_mode.set(ViewMode::Code);
                        on_open_lossy.run(());
                    })
                >
                    "Open as Text anyway"
                </Button>
            </div>
        </div>
    }
}

/// Render the preview view for a file: image asset, markdown formatted HTML,
/// or binary placeholder.
fn render_preview_view(
    path: &str,
    content: &str,
    media_url: Option<String>,
    set_view_mode: WriteSignal<ViewMode>,
    on_open_lossy: Callback<()>,
) -> impl IntoView {
    let kind = FileKind::from_path(path);
    let file_name = path.rsplit('/').next().unwrap_or(path).to_string();

    match kind {
        FileKind::Image => {
            if let Some(url) = media_url {
                view! {
                    <div class="editor-media-preview">
                        <div class="editor-image-frame">
                            <img src=url alt=file_name.clone() class="editor-preview-image" />
                        </div>
                        <div class="editor-media-info">
                            <span class="media-filename">{file_name}</span>
                            <span class="media-type-pill">"Image Asset"</span>
                        </div>
                    </div>
                }
                .into_any()
            } else if path.ends_with(".svg") && !content.is_empty() {
                let src = openwebide_frontend::markdown::svg_data_url(content);
                view! {
                    <div class="editor-media-preview">
                        <div class="editor-image-frame"><img src=src alt=file_name.clone() class="editor-preview-image"/></div>
                        <div class="editor-media-info">
                            <span class="media-filename">{file_name}</span>
                            <span class="media-type-pill">"SVG Vector"</span>
                        </div>
                    </div>
                }
                .into_any()
            } else {
                render_placeholder_view(path, kind, set_view_mode, on_open_lossy).into_any()
            }
        }
        FileKind::Markdown => {
            let html = render_markdown(content);
            view! {
                <div class="editor-preview markdown" inner_html=html />
            }
            .into_any()
        }
        FileKind::Binary | FileKind::Media => {
            render_placeholder_view(path, kind, set_view_mode, on_open_lossy).into_any()
        }
        FileKind::Text => {
            let html = render_markdown(content);
            view! {
                <div class="editor-preview markdown" inner_html=html />
            }
            .into_any()
        }
    }
}

/// The code editor: a full-height textarea bound to the open file's content,
/// with a header showing path, dirty indicator, view mode toggles, and Save button.
///
/// Automatically defaults to Preview mode for non-text files (images, binaries)
/// with informative placeholders for non-previewable files.
#[component]
pub fn Editor(
    open_file: ReadSignal<Option<String>>,
    content: ReadSignal<String>,
    set_content: WriteSignal<String>,
    dirty: ReadSignal<bool>,
    set_dirty: WriteSignal<bool>,
    pending_diff: ReadSignal<Option<FileDiff>>,
    #[prop(default = None.into())] media_url: Signal<Option<String>>,
    #[prop(default = Signal::derive(|| None))] git_head_diff: Signal<Option<FileDiff>>,
    #[prop(optional)] on_load_git_diff: Option<Callback<()>>,
    #[prop(optional)] on_discard_git_diff: Option<Callback<()>>,
    #[prop(into, default = Signal::derive(|| false))] can_revert: Signal<bool>,
    read_only: Signal<bool>,
    on_open_lossy: Callback<()>,
    on_save: Callback<()>,
    on_accept: Callback<()>,
    on_reject: Callback<()>,
) -> impl IntoView {
    let ta = NodeRef::<leptos::html::Textarea>::new();
    let hl = NodeRef::<leptos::html::Div>::new();
    let view_mode = RwSignal::new(ViewMode::Code);

    // Default to Preview for non-text files; default to Code for source files.
    Effect::new(move || {
        if let Some(path) = open_file.get() {
            let kind = FileKind::from_path(&path);
            if kind.is_non_text() {
                view_mode.set(ViewMode::Preview);
            } else if pending_diff.get().is_none() && kind != FileKind::Markdown {
                view_mode.set(ViewMode::Code);
            }
        }
    });

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
                        let on_load = on_load_git_diff;
                        let on_discard = on_discard_git_diff;
                        view! {
                            <div class="editor-header-actions">
                                <Show
                                    when=move || view_mode.get() == ViewMode::InlineDiff || view_mode.get() == ViewMode::SideBySide || view_mode.get() == ViewMode::Content
                                    fallback={
                                        move || {
                                            let on_load = on_load;
                                            view! {
                                                <SegmentedControl
                                                    options=vec![
                                                        SegmentOption::new("Edit", ViewMode::Code),
                                                        SegmentOption::new("Diff HEAD", ViewMode::InlineDiff),
                                                        SegmentOption::new("Preview", ViewMode::Preview),
                                                    ]
                                                    value=view_mode.read_only().into()
                                                    on_change=Callback::new(move |mode| {
                                                        if mode == ViewMode::InlineDiff
                                                            && let Some(cb) = &on_load {
                                                                cb.run(());
                                                            }
                                                        view_mode.set(mode);
                                                    })
                                                />
                                                <Show when=move || read_only.get() fallback=|| ()>
                                                    <span class="form-hint" style="margin-left: 12px; align-self: center;">
                                                        "Not valid UTF-8 — shown read-only"
                                                    </span>
                                                </Show>
                                                <Button
                                                    variant=if dirty.get() { ButtonVariant::Primary } else { ButtonVariant::Default }
                                                    size=ButtonSize::Sm
                                                    disabled=Signal::derive(move || read_only.get() || !dirty.get())
                                                    on_click=Callback::new(move |_| on_save.run(()))
                                                >
                                                    "Save"
                                                </Button>
                                            }
                                        }
                                    }
                                >
                                    <SegmentedControl
                                        options=vec![
                                            SegmentOption::new("Edit", ViewMode::Code),
                                            SegmentOption::new("Inline", ViewMode::InlineDiff),
                                            SegmentOption::new("Split", ViewMode::SideBySide),
                                            SegmentOption::new("Content", ViewMode::Content),
                                            SegmentOption::new("Preview", ViewMode::Preview),
                                        ]
                                        value=view_mode.read_only().into()
                                        on_change=Callback::new(move |mode| {
                                            view_mode.set(mode);
                                        })
                                    />
                                    <Show when=move || can_revert.get() fallback=|| ()>
                                        <Button
                                            variant=ButtonVariant::Danger
                                            size=ButtonSize::Sm
                                            on_click={
                                                let on_discard = on_discard;
                                                Callback::new(move |_| {
                                                    if let Some(cb) = &on_discard {
                                                        cb.run(());
                                                    }
                                                    view_mode.set(ViewMode::Code);
                                                })
                                            }
                                        >
                                            "↺ Revert to HEAD"
                                        </Button>
                                    </Show>
                                    <Button
                                        variant=if dirty.get() { ButtonVariant::Primary } else { ButtonVariant::Default }
                                        size=ButtonSize::Sm
                                        disabled=Signal::derive(move || !dirty.get())
                                        on_click=Callback::new(move |_| on_save.run(()))
                                    >
                                        "Save"
                                    </Button>
                                </Show>
                            </div>
                        }
                    }
                >
                    <div class="editor-diff-actions">
                        <Show when=move || {
                            if let Some(d) = pending_diff.get() {
                                d.old.is_none() || d.old_unavailable
                            } else { false }
                        }>
                            <span style="color: var(--warn); margin-right: 12px; font-size: 0.9em; flex: 1;">
                                {move || {
                                    if let Some(d) = pending_diff.get() {
                                        if d.old_unavailable {
                                            "Warning: Unreadable file was modified. The old contents were not sent to the agent and could not be included in this preview. Rejecting will restore the file from backup."
                                        } else {
                                            "New file created."
                                        }
                                    } else { "" }
                                }}
                            </span>
                        </Show>
                        <SegmentedControl
                            options=vec![
                                SegmentOption::new("Inline", ViewMode::InlineDiff),
                                SegmentOption::new("Split", ViewMode::SideBySide),
                                SegmentOption::new("Content", ViewMode::Content),
                                SegmentOption::new("Preview", ViewMode::Preview),
                            ]
                            value=view_mode.read_only().into()
                            on_change=Callback::new(move |mode| view_mode.set(mode))
                        />
                        <Button
                            variant=ButtonVariant::Success
                            size=ButtonSize::Sm
                            on_click=Callback::new(move |_| on_accept.run(()))
                        >
                            "✓ Accept"
                        </Button>
                        <Button
                            variant=ButtonVariant::Danger
                            size=ButtonSize::Sm
                            on_click=Callback::new(move |_| on_reject.run(()))
                        >
                            "✕ Reject"
                        </Button>
                    </div>
                </Show>
            </div>
            <Show
                when=move || open_file.get().is_some()
                fallback=move || {
                    view! {
                        <p class="empty editor-empty">"Select a file to edit, or create a new one in the explorer."</p>
                    }
                }
            >
                {move || {
                    let mode = view_mode.get();
                    if let Some(diff) = pending_diff.get() {
                        match mode {
                            ViewMode::InlineDiff => render_inline_diff(diff).into_any(),
                            ViewMode::SideBySide => render_side_by_side(diff).into_any(),
                            ViewMode::Content => render_content_view(diff.new).into_any(),
                            ViewMode::Preview => {
                                let path = open_file.get().unwrap_or_default();
                                render_preview_view(
                                    &path,
                                    &diff.new,
                                    media_url.get(),
                                    view_mode.write_only(),
                                    on_open_lossy,
                                )
                                .into_any()
                            }
                            ViewMode::Code => render_content_view(diff.new).into_any(),
                        }
                    } else if (mode == ViewMode::InlineDiff || mode == ViewMode::SideBySide || mode == ViewMode::Content) && git_head_diff.get().is_some() {
                        let diff = git_head_diff.get().unwrap();
                        match mode {
                            ViewMode::InlineDiff => render_inline_diff(diff).into_any(),
                            ViewMode::SideBySide => render_side_by_side(diff).into_any(),
                            ViewMode::Content => render_content_view(diff.new).into_any(),
                            _ => render_content_view(diff.new).into_any(),
                        }
                    } else {
                        match mode {
                            ViewMode::Preview => {
                                let path = open_file.get().unwrap_or_default();
                                let text = content.get();
                                render_preview_view(
                                    &path,
                                    &text,
                                    media_url.get(),
                                    view_mode.write_only(),
                                    on_open_lossy,
                                )
                                .into_any()
                            }
                            _ => {
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
                                            readonly=read_only
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
                                }.into_any()
                            }
                        }
                    }
                }}
            </Show>
        </div>
    }
}
