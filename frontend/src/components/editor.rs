use leptos::prelude::*;
use openwebide_core::{
    DiffChunk, FileDiff, FileKind, diff_inline_full, diff_side_by_side_detailed,
    highlight::{Language, TokenKind, highlight_lines, language_from_path},
};
use web_sys::wasm_bindgen::JsCast;

use crate::components::chat_pane::render_markdown;
use crate::components::ui::{
    Button, ButtonSize, ButtonVariant, Icon, IconButton, IconName, PanelSearchRow, SegmentOption,
    SegmentedControl,
};
use crate::state::{git::GitState, projects::ProjectsState, workspace::WorkspaceState};
use crate::state_actions::editor::{EditorActions, EditorCommand};

fn current_editor_target(actions: EditorActions, textarea: &web_sys::HtmlTextAreaElement) -> bool {
    textarea
        .get_attribute("data-editor-project")
        .and_then(|project| project.parse().ok())
        .zip(textarea.get_attribute("data-editor-path"))
        .is_some_and(|(project, path)| actions.is_current(project, &path))
}

fn editor_selection(textarea: &web_sys::HtmlTextAreaElement) -> openwebide_core::editor::Selection {
    use openwebide_core::editor::{Selection, utf16_to_byte};
    let text = textarea.value();
    let start = utf16_to_byte(
        &text,
        textarea.selection_start().ok().flatten().unwrap_or(0) as usize,
    );
    let end = utf16_to_byte(
        &text,
        textarea.selection_end().ok().flatten().unwrap_or(0) as usize,
    );
    if textarea.selection_direction().ok().flatten().as_deref() == Some("backward") {
        Selection {
            anchor: end,
            head: start,
        }
    } else {
        Selection {
            anchor: start,
            head: end,
        }
    }
}

fn document_selection(
    textarea: &web_sys::HtmlTextAreaElement,
    text: &str,
) -> openwebide_core::editor::Selection {
    use openwebide_core::editor::{Selection, textarea_to_byte};
    let start = textarea_to_byte(
        text,
        textarea.selection_start().ok().flatten().unwrap_or(0) as usize,
    );
    let end = textarea_to_byte(
        text,
        textarea.selection_end().ok().flatten().unwrap_or(0) as usize,
    );
    if textarea.selection_direction().ok().flatten().as_deref() == Some("backward") {
        Selection {
            anchor: end,
            head: start,
        }
    } else {
        Selection {
            anchor: start,
            head: end,
        }
    }
}

fn restore_editor_selection(
    textarea: &web_sys::HtmlTextAreaElement,
    text: &str,
    selection: openwebide_core::editor::Selection,
) {
    use openwebide_core::editor::byte_to_textarea;
    let range = selection.range();
    if let (Ok(start), Ok(end)) = (
        byte_to_textarea(text, range.start),
        byte_to_textarea(text, range.end),
    ) && let (Ok(start), Ok(end)) = (u32::try_from(start), u32::try_from(end))
    {
        let _ = textarea.set_selection_range_with_direction(
            start,
            end,
            if selection.anchor > selection.head {
                "backward"
            } else {
                "forward"
            },
        );
    }
}

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
}

fn text_position(node: &web_sys::Node, offset: &mut u32) -> Option<(web_sys::Node, u32)> {
    if node.node_type() == web_sys::Node::TEXT_NODE {
        let length =
            u32::try_from(node.node_value().unwrap_or_default().encode_utf16().count()).ok()?;
        if *offset <= length {
            return Some((node.clone(), *offset));
        }
        *offset -= length;
    } else {
        let children = node.child_nodes();
        for index in 0..children.length() {
            if let Some(child) = children.item(index)
                && let Some(position) = text_position(&child, offset)
            {
                return Some(position);
            }
        }
    }
    None
}

/// Reveal the actual match column, including tabs, Unicode and highlighted tokens.
fn reveal_match_column(
    text: &web_sys::Element,
    body: &web_sys::HtmlElement,
    mut offset: u32,
    gutter: f64,
) {
    let Some((node, offset)) = text_position(text.as_ref(), &mut offset) else {
        return;
    };
    let Ok(range) = document().create_range() else {
        return;
    };
    if range.set_start(&node, offset).is_err() || range.set_end(&node, offset).is_err() {
        return;
    }
    let caret = range.get_bounding_client_rect();
    let viewport = body.get_bounding_client_rect();
    let left = viewport.left() + gutter;
    if caret.left() < left || caret.left() > viewport.right() - 16.0 {
        body.set_scroll_left((body.scroll_left() + caret.left() - left).max(0.0));
    }
}

fn clear_find_marks(root: &web_sys::HtmlElement) {
    if let Ok(nodes) = root.query_selector_all(".editor-find-match") {
        for index in 0..nodes.length() {
            if let Some(node) = nodes
                .item(index)
                .and_then(|node| node.dyn_into::<web_sys::Element>().ok())
            {
                let _ = node.class_list().remove_1("editor-find-match");
            }
        }
    }
}

/// Literal, case-sensitive matches with browser selection offsets and logical lines.
fn find_matches(text: &str, query: &str) -> Vec<(u32, u32, usize)> {
    if query.is_empty() {
        return Vec::new();
    }
    let mut byte = 0;
    let mut utf16 = 0;
    let mut line = 1;
    text.match_indices(query)
        .map(|(start, matched)| {
            let gap = &text[byte..start];
            utf16 += gap.encode_utf16().count();
            line += gap.bytes().filter(|value| *value == b'\n').count();
            let begin = utf16;
            let match_line = line;
            utf16 += matched.encode_utf16().count();
            line += matched.bytes().filter(|value| *value == b'\n').count();
            byte = start + matched.len();
            (
                u32::try_from(begin).unwrap_or(u32::MAX),
                u32::try_from(utf16).unwrap_or(u32::MAX),
                match_line,
            )
        })
        .collect()
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

use crate::text::escape_html;

#[cfg(feature = "test-support")]
thread_local! {
    static HIGHLIGHT_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Number of overlay generations, for browser performance regressions.
#[cfg(feature = "test-support")]
pub fn highlight_count() -> usize {
    HIGHLIGHT_COUNT.get()
}

/// Render the highlighted source as an HTML string for the overlay.
fn highlight_html(source: &str, language: Language) -> String {
    #[cfg(feature = "test-support")]
    HIGHLIGHT_COUNT.set(HIGHLIGHT_COUNT.get() + 1);
    let lines = highlight_lines(source, language);
    let mut html = String::new();
    for (idx, line) in lines.iter().enumerate() {
        html.push_str(&format!(
            "<span class=\"editor-source-line\" data-line=\"{}\">",
            idx + 1
        ));
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
        if idx + 1 < lines.len() {
            html.push('\n');
        }
        html.push_str("</span>");
    }
    html
}

/// The textarea owns scrolling; translate paint instead of copying clamped offsets.
fn sync_highlight_scroll(textarea: &web_sys::HtmlTextAreaElement, overlay: &web_sys::HtmlElement) {
    let _ = overlay.style().set_property(
        "--editor-scroll-x",
        &format!("{}px", -textarea.scroll_left()),
    );
    let _ = overlay.style().set_property(
        "--editor-scroll-y",
        &format!("{}px", -textarea.scroll_top()),
    );
    overlay.set_scroll_top(0.0);
    overlay.set_scroll_left(0.0);
}

#[component]
fn HighlightOverlay(
    content: ReadSignal<String>,
    open_file: ReadSignal<Option<String>>,
    node_ref: NodeRef<leptos::html::Div>,
    textarea_ref: NodeRef<leptos::html::Textarea>,
    ready: RwSignal<bool>,
) -> impl IntoView {
    use wasm_bindgen::closure::Closure;

    let viewport_observer = StoredValue::new_local(None::<wasm_bindgen::JsValue>);
    Effect::new(move || {
        if let (Some(input), Some(overlay)) = (textarea_ref.get(), node_ref.get())
            && viewport_observer.get_value().is_none()
        {
            viewport_observer.set_value(Some(crate::viewport::observe_editor_viewport(
                &input, &overlay,
            )));
        }
    });
    on_cleanup(move || {
        viewport_observer.with_value(|stop| {
            if let Some(stop) = stop {
                let _ = stop
                    .unchecked_ref::<js_sys::Function>()
                    .call0(&wasm_bindgen::JsValue::NULL);
            }
        });
    });
    let rendered = RwSignal::new(String::new());
    let request = StoredValue::new(None::<i32>);
    let generation = StoredValue::new(0_u64);
    let queued_generation = StoredValue::new(0_u64);
    let path = StoredValue::new(None::<String>);
    let callback = StoredValue::new_local(Closure::<dyn FnMut()>::new(move || {
        request.set_value(None);
        if generation.get_value() != queued_generation.get_value()
            || open_file.get_untracked() != path.get_value()
            || node_ref.get_untracked().is_none()
        {
            return;
        }
        let language = open_file
            .with_untracked(|path| path.as_deref().map(language_from_path))
            .unwrap_or(Language::Plain);
        rendered.set(content.with_untracked(|text| highlight_html(text, language)));
        ready.set(true);
        let published_generation = generation.get_value();
        // Re-align after the highlighted HTML reaches the DOM.
        leptos::leptos_dom::helpers::queue_microtask(move || {
            if generation.try_get_value() != Some(published_generation)
                || open_file.try_get_untracked() != path.try_get_value()
            {
                return;
            }
            if let (Some(Some(textarea)), Some(Some(overlay))) = (
                textarea_ref.try_get_untracked(),
                node_ref.try_get_untracked(),
            ) {
                sync_highlight_scroll(&textarea, &overlay);
            }
        });
    }));

    Effect::new(move || {
        content.track();
        ready.set(false);
        let current_path = open_file.get();
        let mounted = node_ref.get().is_some();
        if current_path != path.get_value() || !mounted {
            generation.update_value(|value| *value += 1);
            if let Some(id) = request.get_value() {
                let _ = window().cancel_animation_frame(id);
                request.set_value(None);
            }
            path.set_value(current_path);
            rendered.set(String::new());
        }
        if mounted && request.get_value().is_none() {
            queued_generation.set_value(generation.get_value());
            let id = callback.with_value(|callback| {
                window().request_animation_frame(callback.as_ref().unchecked_ref())
            });
            if let Ok(id) = id {
                request.set_value(Some(id));
            }
        }
    });
    // Keep the JS closure owned here so cancellation also releases its captures.
    on_cleanup(move || {
        if let Some(id) = request.get_value() {
            let _ = window().cancel_animation_frame(id);
        }
    });

    view! { <div class="editor-highlight" node_ref=node_ref><div class="editor-highlight-content" inner_html=move || rendered.get() /></div> }
}

/// Render a list of intra-line diff chunks with word-level highlights.
pub(crate) fn render_diff_chunks(chunks: Vec<DiffChunk>) -> impl IntoView {
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

/// Render a full file edit as inline removed/added lines with intra-line word diffs.
pub(super) fn render_inline_diff(diff: FileDiff) -> impl IntoView {
    let digits = diff
        .old
        .as_deref()
        .unwrap_or_default()
        .lines()
        .count()
        .max(diff.new.lines().count())
        .max(1)
        .to_string()
        .len();
    let mut old_line = 0;
    let mut new_line = 0;
    let body = diff_inline_full(&diff)
        .into_iter()
        .map(|dl| {
            let mark = dl.marker;
            let old_number = if mark != '+' {
                old_line += 1;
                Some(old_line)
            } else {
                None
            };
            let new_number = if mark != '-' {
                new_line += 1;
                Some(new_line)
            } else {
                None
            };
            let line_class = match mark {
                '+' => "diff-line add",
                '-' => "diff-line del",
                _ => "diff-line",
            };
            view! {
                <div class=line_class data-line=new_number>
                    <span class="editor-line-gutter">
                        <span class="editor-line-number">{old_number}</span>
                        <span class="editor-line-number">{new_number}</span>
                        <span class="diff-line-marker">{mark}</span>
                    </span>
                    <span class="editor-line-text">{render_diff_chunks(dl.chunks)}
                        {dl.ending_note.map(|note| view! { <span class="form-hint">{note}</span> })}
                    </span>
                </div>
            }
        })
        .collect_view();
    view! {
        <div class="editor-diff editor-diff-inline" style=format!("--editor-number-width: {digits}ch")>
            <div class="inline-track">{body}</div>
        </div>
    }
}

/// Scroll aligned split panes together, including equal space for their longest line.
fn sync_split_scroll(source: NodeRef<leptos::html::Div>, target: NodeRef<leptos::html::Div>) {
    if let (Some(source), Some(target)) = (source.get_untracked(), target.get_untracked()) {
        if (source.scroll_left() - target.scroll_left()).abs() > 0.5 {
            target.set_scroll_left(source.scroll_left());
        }
        if (source.scroll_top() - target.scroll_top()).abs() > 0.5 {
            target.set_scroll_top(source.scroll_top());
        }
    }
}

/// Full aligned rows with a shared scroll extent and compact, pinned gutters.
fn render_side_by_side(diff: FileDiff) -> impl IntoView {
    let digits = diff
        .old
        .as_deref()
        .unwrap_or_default()
        .lines()
        .count()
        .max(diff.new.lines().count())
        .max(1)
        .to_string()
        .len();
    let rows = diff_side_by_side_detailed(&diff);
    let left_pane = NodeRef::<leptos::html::Div>::new();
    let right_pane = NodeRef::<leptos::html::Div>::new();
    let left_track = NodeRef::<leptos::html::Div>::new();
    let right_track = NodeRef::<leptos::html::Div>::new();
    Effect::new(move || {
        let (Some(left), Some(right)) = (left_track.get(), right_track.get()) else {
            return;
        };
        let extent = |track: &web_sys::HtmlElement| {
            let gutter = track
                .query_selector(".editor-line-gutter")
                .ok()
                .flatten()
                .map_or(0, |node| {
                    node.unchecked_into::<web_sys::HtmlElement>().offset_width()
                });
            let text = track
                .query_selector_all(".editor-line-text")
                .ok()
                .map_or(0, |nodes| {
                    (0..nodes.length())
                        .filter_map(|index| nodes.item(index))
                        .map(|node| node.unchecked_into::<web_sys::HtmlElement>().scroll_width())
                        .max()
                        .unwrap_or(0)
                });
            gutter + text + 20
        };
        let width = extent(&left).max(extent(&right));
        for track in [left, right] {
            let _ = track
                .unchecked_ref::<web_sys::HtmlElement>()
                .style()
                .set_property("width", &format!("{width}px"));
        }
    });
    let mut old_line = 0;
    let mut new_line = 0;
    let (left, right): (Vec<_>, Vec<_>) = rows.into_iter().map(|(left, right)| {
        let changed = !matches!((&left, &right), (Some(l), Some(r)) if l.marker == ' ' && r.marker == ' ');
        let old_number = left.as_ref().map(|_| { old_line += 1; old_line });
        let new_number = right.as_ref().map(|_| { new_line += 1; new_line });
        let left_class = if left.is_none() { "sbs-cell sbs-empty" } else if changed { "sbs-cell sbs-del" } else { "sbs-cell" };
        let right_class = if right.is_none() { "sbs-cell sbs-empty" } else if changed { "sbs-cell sbs-add" } else { "sbs-cell" };
        let cell = |class, number, line: Option<openwebide_core::DiffLine>| view! {
            <div class=class data-line=number>
                <span class="editor-line-gutter"><span class="editor-line-number">{number}</span></span>
                <span class="editor-line-text">{line.map(|line| view! {
                    {render_diff_chunks(line.chunks)}
                    {line.ending_note.map(|note| view! { <span class="form-hint">{note}</span> })}
                })}</span>
            </div>
        };
        (cell(left_class, old_number, left), cell(right_class, new_number, right))
    }).unzip();
    view! {
        <div class="editor-diff editor-diff-side" style=format!("--editor-number-width: {digits}ch")>
            <div class="sbs-pane" node_ref=left_pane on:scroll=move |_| sync_split_scroll(left_pane, right_pane)><div class="sbs-track" node_ref=left_track>{left}</div></div>
            <div class="sbs-pane" node_ref=right_pane on:scroll=move |_| sync_split_scroll(right_pane, left_pane)><div class="sbs-track" node_ref=right_track>{right}</div></div>
        </div>
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

    if openwebide_core::file_type::extension(path).as_deref() == Some("pdf") {
        return media_url.map_or_else(
            || render_placeholder_view(path, kind, set_view_mode, on_open_lossy).into_any(),
            |url| view! { <iframe class="editor-pdf-preview" src=url title=format!("PDF preview: {file_name}") /> }.into_any(),
        );
    }
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
            } else if std::path::Path::new(&path)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("svg"))
                && !content.is_empty()
            {
                let src = crate::markdown::svg_data_url(content);
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
            if FileKind::is_plain_document(path) {
                view! { <pre class="editor-preview editor-document-preview">{content.to_string()}</pre> }.into_any()
            } else {
                render_placeholder_view(path, kind, set_view_mode, on_open_lossy).into_any()
            }
        }
    }
}

fn render_markdown_diff(diff: &FileDiff) -> impl IntoView {
    let html = if diff.old_unavailable {
        render_markdown(&diff.new)
    } else {
        crate::markdown::render_diff(diff.old.as_deref().unwrap_or_default(), &diff.new)
    };
    view! { <div class="editor-preview markdown rich-preview" inner_html=html /> }
}

/// The code editor: a full-height textarea bound to the open file's content,
/// with a header showing path, dirty indicator, view mode toggles, and Save button.
///
/// Automatically defaults to Preview mode for non-text files (images, binaries)
/// with informative placeholders for non-previewable files.
fn pair_character(text: &str) -> Option<char> {
    let mut chars = text.chars();
    let ch = chars.next()?;
    (chars.next().is_none() && matches!(ch, '(' | ')' | '[' | ']' | '{' | '}' | '\'' | '"' | '`'))
        .then_some(ch)
}

fn apply_editor_command(
    actions: EditorActions,
    command: EditorCommand,
    textarea: &web_sys::HtmlTextAreaElement,
    text: &str,
) -> bool {
    if let Ok(Some((text, selection))) = actions.command(
        command,
        document_selection(textarea, text),
        actions.rules_untracked().indentation,
    ) {
        textarea.set_value(&text);
        restore_editor_selection(textarea, &text, selection);
        true
    } else {
        false
    }
}

#[component]
pub fn Editor(
    #[prop(optional)] on_load_git_diff: Option<Callback<()>>,
    #[prop(optional)] on_discard_git_diff: Option<Callback<()>>,
    read_only: Signal<bool>,
    on_open_lossy: Callback<()>,
    on_save: Callback<()>,
    on_accept: Callback<()>,
    on_reject: Callback<()>,
) -> impl IntoView {
    let workspace = expect_context::<WorkspaceState>();
    let editor_actions = EditorActions::new(workspace);
    let tab_moves_focus = RwSignal::new(false);
    let paste_matches_indentation = RwSignal::new(false);
    Effect::new(move |_| {
        workspace.active_project.track();
        workspace.open_file.track();
        paste_matches_indentation.set(false);
    });
    let file_tree_actions = use_context::<crate::state_actions::file_tree::FileTreeActions>();
    let read_only = Signal::derive(move || {
        read_only.get()
            || workspace.is_resolving()
            || file_tree_actions.is_some_and(|actions| actions.busy.get())
    });
    let projects = expect_context::<ProjectsState>();
    let git = expect_context::<GitState>();

    let open_file = workspace.open_file.read_only();
    let content = workspace.content.read_only();
    let dirty = workspace.dirty.read_only();
    let pending_diff: Signal<Option<FileDiff>> = Signal::from(workspace.pending_diff);
    let media_url = workspace.media_url.read_only();
    let preview_disabled = Signal::derive(move || {
        workspace.open_file.with(|path| {
            path.as_ref()
                .is_none_or(|path| !FileKind::supports_preview(path))
        })
    });
    let git_head_diff = Signal::derive(move || {
        let open_file = workspace.open_file.get()?;
        let project_id = projects.active_project.get();
        let head = git.head_content.get()?;
        if head.project_id != project_id || head.path != open_file || head.content.is_err() {
            return None;
        }
        let old = head.content.ok();
        let new = workspace.content.get();
        if old.is_none() && new.is_empty() {
            return None;
        }
        Some(FileDiff {
            path: open_file,
            old,
            new,
            old_unavailable: false,
            backup_path: None,
        })
    });
    let binary_head = Signal::derive(move || {
        git.head_content.get().is_some_and(|head| {
            head.project_id == projects.active_project.get()
                && Some(head.path) == workspace.open_file.get()
                && head
                    .content
                    .as_ref()
                    .err()
                    .is_some_and(|error| error == "binary file")
        })
    });
    let can_revert = Signal::derive(move || {
        let Some(open_file) = workspace.open_file.get() else {
            return false;
        };
        let project_id = projects.active_project.get();
        git.head_content.get().is_some_and(|head| {
            head.project_id == project_id && head.path == open_file && head.content.is_ok()
        })
    });
    let ta = NodeRef::<leptos::html::Textarea>::new();
    let hl = NodeRef::<leptos::html::Div>::new();
    let highlight_ready = RwSignal::new(false);
    let view_mode = RwSignal::new(ViewMode::Code);

    let root = NodeRef::<leptos::html::Div>::new();
    let find_input = NodeRef::<leptos::html::Input>::new();
    let find_open = RwSignal::new(false);
    let query = RwSignal::new(String::new());
    let match_index = RwSignal::new(0_usize);
    let find_source = Memo::new(move |_| {
        pending_diff
            .get()
            .map_or_else(|| content.get(), |diff| diff.new)
    });
    let matches = Memo::new(move |_| find_matches(&find_source.get(), &query.get()));
    let navigate = Callback::new(move |forward: bool| {
        let count = matches.get_untracked().len();
        if count > 0 {
            match_index.update(|index| {
                *index = if forward {
                    (*index + 1) % count
                } else {
                    (*index + count - 1) % count
                }
            });
        }
    });
    Effect::new(move || {
        query.track();
        open_file.track();
        projects.active_project.track();
        match_index.set(0);
    });
    Effect::new(move || {
        if find_open.get()
            && let Some(input) = find_input.get()
        {
            let _ = input.focus();
        }
    });
    Effect::new(move || {
        let active = find_open.get() && view_mode.get() != ViewMode::Preview;
        if !active {
            if let Some(root) = root.get() {
                clear_find_marks(&root);
            }
            return;
        }
        highlight_ready.track();
        let results = matches.get();
        let selected = results
            .get(match_index.get() % results.len().max(1))
            .copied();
        let mode = view_mode.get();
        let path = open_file.get();
        let project = projects.active_project.get();
        let source = find_source.get();
        let search = query.get();
        let index = match_index.get();
        leptos::leptos_dom::helpers::queue_microtask(move || {
            if open_file.try_get_untracked() != Some(path)
                || projects.active_project.try_get_untracked() != Some(project)
                || find_source.try_with_untracked(|current| current == &source) != Some(true)
                || query.try_get_untracked() != Some(search)
                || match_index.try_get_untracked() != Some(index)
                || view_mode.try_get_untracked() != Some(mode)
                || find_open.try_get_untracked() != Some(active)
            {
                return;
            }
            let Some(Some(root)) = root.try_get_untracked() else {
                return;
            };
            clear_find_marks(&root);
            let Some((start, end, line)) = selected.filter(|_| active) else {
                return;
            };
            let line_start = source
                .split_inclusive('\n')
                .take(line.saturating_sub(1))
                .map(|part| part.encode_utf16().count())
                .sum::<usize>();
            let column = start.saturating_sub(u32::try_from(line_start).unwrap_or(u32::MAX));
            if mode == ViewMode::Code
                && let Some(Some(textarea)) = ta.try_get_untracked()
            {
                let _ = textarea.set_selection_range(start, end);
                if let Ok(Some(row)) =
                    root.query_selector(&format!(".editor-source-line[data-line='{line}']"))
                {
                    let row: web_sys::HtmlElement = row.unchecked_into();
                    textarea.set_scroll_top(f64::from(row.offset_top().saturating_sub(12)));
                    if let Some(Some(overlay)) = hl.try_get_untracked() {
                        sync_highlight_scroll(&textarea, &overlay);
                        let gutter = window().get_computed_style(&textarea).ok().flatten().and_then(|style| style.get_property_value("padding-left").ok()).and_then(|padding| padding.trim_end_matches("px").parse::<f64>().ok()).unwrap_or(40.0);
                        reveal_match_column(&row, &textarea, column, gutter);
                        sync_highlight_scroll(&textarea, &overlay);
                    }
                }
            } else if let Ok(Some(row)) =
                root.query_selector(&format!(".editor-diff-inline [data-line='{line}'], .sbs-pane:last-child [data-line='{line}']"))
            {
                let _ = row.class_list().add_1("editor-find-match");
                if let Ok(Some(body)) = row.closest(".sbs-pane, .editor-diff") {
                    let body: web_sys::HtmlElement = body.unchecked_into();
                    body.set_scroll_top(
                        body.scroll_top() + row.get_bounding_client_rect().top()
                            - body.get_bounding_client_rect().top(),
                    );
                    if let Ok(Some(text)) = row.query_selector(".editor-line-text") {
                        let gutter = row.query_selector(".editor-line-gutter").ok().flatten().map_or(0.0, |gutter| gutter.get_bounding_client_rect().width()) + 8.0;
                        reveal_match_column(&text, &body, column, gutter);
                    }
                }
            }
        });
    });

    // Default to Preview for non-text files; default to Code for source files.
    Effect::new(move || {
        if let Some(path) = open_file.get() {
            let kind = FileKind::from_path(&path);
            if kind.is_non_text() && FileKind::supports_preview(&path) {
                view_mode.set(ViewMode::Preview);
            } else if pending_diff.get().is_none() && kind != FileKind::Markdown {
                view_mode.set(ViewMode::Code);
            }
        }
    });

    // Keep the selected view while hunk decisions update the same file.
    let pending_path = Memo::new(move |_| pending_diff.get().map(|diff| diff.path));
    Effect::new(move || {
        if pending_path.get().is_some() {
            view_mode.set(ViewMode::InlineDiff);
        }
    });

    // Mirror the loaded content into the textarea without clobbering the
    // user's in-progress edits (same pattern as the chat composer).
    Effect::new(move || {
        let value = content.get();
        if let Some(el) = ta.get()
            && el.value() != value.replace("\r\n", "\n").replace('\r', "\n")
        {
            let scroll = (el.scroll_top(), el.scroll_left());
            el.set_value(&value);
            if current_editor_target(editor_actions, &el)
                && let Some(key) = workspace
                    .active_project
                    .get_untracked()
                    .zip(open_file.get_untracked())
                && let Some(selection) = workspace.editor_documents.with_untracked(|documents| {
                    documents
                        .get(&key)
                        .filter(|document| document.text() == value)
                        .and_then(|document| document.selections().first().copied())
                })
            {
                restore_editor_selection(&el, &value, selection);
                el.set_scroll_top(scroll.0);
                el.set_scroll_left(scroll.1);
            }
        }
    });

    view! {
        <div class="editor" node_ref=root style=move || format!("--editor-tab-width: {}", editor_actions.rules().indentation.tab_width()) on:keydown=move |event: web_sys::KeyboardEvent| {
            if (event.ctrl_key() || event.meta_key()) && event.key().eq_ignore_ascii_case("f") && view_mode.get_untracked() != ViewMode::Preview {
                event.prevent_default(); event.stop_propagation(); find_open.set(true);
                if let Some(input) = find_input.get_untracked() { let _ = input.focus(); input.select(); }
            } else if event.key() == "Escape" && find_open.get_untracked() {
                event.prevent_default(); event.stop_propagation(); find_open.set(false);
                if let Some(textarea) = ta.get_untracked() { let _ = textarea.focus(); }
            }
        }>
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
                                <Show when=move || view_mode.get() == ViewMode::Code && open_file.with(|path| path.as_ref().is_some_and(|path| !FileKind::from_path(path).is_non_text()))>
                                    <super::dropdown::ActionMenu aria_label="Editing commands">
                                        {[
                                            ("Move lines up", EditorCommand::Line(openwebide_core::editor::LineCommand::MoveUp), "Alt+Up"),
                                            ("Move lines down", EditorCommand::Line(openwebide_core::editor::LineCommand::MoveDown), "Alt+Down"),
                                            ("Duplicate lines above", EditorCommand::Line(openwebide_core::editor::LineCommand::DuplicateAbove), "Alt+Shift+Up"),
                                            ("Duplicate lines below", EditorCommand::Line(openwebide_core::editor::LineCommand::Duplicate), "Alt+Shift+Down"),
                                            ("Duplicate selection", EditorCommand::DuplicateSelection, "Ctrl/Cmd+Shift+D"),
                                            ("Delete lines", EditorCommand::Line(openwebide_core::editor::LineCommand::Delete), "Ctrl/Cmd+Shift+K"),
                                            ("Insert line above", EditorCommand::Line(openwebide_core::editor::LineCommand::InsertAbove), "Ctrl/Cmd+Shift+Enter"),
                                            ("Insert line below", EditorCommand::Line(openwebide_core::editor::LineCommand::InsertBelow), "Ctrl/Cmd+Enter"),
                                            ("Toggle line comment", EditorCommand::LineComment, "Ctrl/Cmd+/"),
                                            ("Toggle block comment", EditorCommand::BlockComment, "Ctrl/Cmd+Shift+/"),
                                            ("Reindent selected lines", EditorCommand::Reindent, ""),
                                        ].into_iter().map(move |(label, command, shortcut)| view! {
                                            <button type="button" role="menuitem" class="ui-dropdown-item recent-item" title=shortcut disabled=move || read_only.get() || {
                                                let language = openwebide_core::highlight::language_from_path(&open_file.get().unwrap_or_default());
                                                match command {
                                                    EditorCommand::LineComment => openwebide_core::editor::line_comment(language).is_none() && openwebide_core::editor::block_comment(language).is_none(),
                                                    EditorCommand::BlockComment => openwebide_core::editor::block_comment(language).is_none(),
                                                    EditorCommand::Reindent => !openwebide_core::editor::supports_brackets(language),
                                                    _ => false,
                                                }
                                            } on:click=move |_| {
                                                if let Some(textarea) = ta.get() && !read_only.get_untracked() && current_editor_target(editor_actions, &textarea) { apply_editor_command(editor_actions, command, &textarea, &workspace.content.get_untracked()); let _ = textarea.focus(); }
                                            }>{label}</button>
                                        }).collect_view()}
                                    </super::dropdown::ActionMenu>
                                </Show>
                                <Show
                                    when=move || view_mode.get() == ViewMode::InlineDiff || view_mode.get() == ViewMode::SideBySide
                                    fallback={
                                        move || {
                                            let on_load = on_load;
                                            view! {
                                                <SegmentedControl
                                                    options=vec![
                                                        SegmentOption::new("Edit", ViewMode::Code),
                                                        SegmentOption::new("Diff HEAD", ViewMode::InlineDiff),
                                                        SegmentOption::new("Preview", ViewMode::Preview).disabled_when(preview_disabled),
                                                    ]
                                                    value=view_mode.read_only().into()
                                                    on_change=Callback::new(move |mode| {
                                                        if (mode == ViewMode::InlineDiff || (mode == ViewMode::Preview && open_file.with(|path| path.as_ref().is_some_and(|path| FileKind::from_path(path) == FileKind::Markdown))))
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
                                            SegmentOption::new("Preview", ViewMode::Preview).disabled_when(preview_disabled),
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
                                            <super::ui::Icon name=super::ui::IconName::Undo2 />"Revert to HEAD"
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
                                            "Warning: Unreadable file was modified. The old contents were not sent to the agent and could not be included in this preview. Rejecting will restore the previous contents."
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
                                SegmentOption::new("Preview", ViewMode::Preview).disabled_when(preview_disabled),
                            ]
                            value=view_mode.read_only().into()
                            on_change=Callback::new(move |mode| view_mode.set(mode))
                        />
                        <Button
                            variant=ButtonVariant::Success
                            size=ButtonSize::Sm
                            disabled=Signal::derive(move || workspace.is_resolving())
                            on_click=Callback::new(move |_| on_accept.run(()))
                        >
                            <super::ui::Icon name=super::ui::IconName::Check />"Accept"
                        </Button>
                        <Button
                            variant=ButtonVariant::Danger
                            size=ButtonSize::Sm
                            disabled=Signal::derive(move || workspace.is_resolving())
                            on_click=Callback::new(move |_| on_reject.run(()))
                        >
                            <super::ui::Icon name=super::ui::IconName::X />"Reject"
                        </Button>
                    </div>
                </Show>
                <IconButton label="Find in file (Ctrl/⌘F)" disabled=Signal::derive(move || open_file.get().is_none() || view_mode.get() == ViewMode::Preview) on_click=Callback::new(move |_| find_open.set(!find_open.get_untracked()))><Icon name=IconName::Search /></IconButton>
            </div>
            <Show when=move || find_open.get() && open_file.get().is_some() && view_mode.get() != ViewMode::Preview>
                <PanelSearchRow class="editor-find">
                    <input class="form-input panel-search-input" type="search" node_ref=find_input placeholder="Find in file" aria-label="Find in file" title="Literal, case-sensitive search" prop:value=move || query.get() on:input=move |event| query.set(event_target_value(&event)) on:keydown=move |event: web_sys::KeyboardEvent| {
                        if event.key() == "Enter" { event.prevent_default(); navigate.run(!event.shift_key()); }
                    } />
                    <span class="form-hint" aria-live="polite">{move || { let count = matches.get().len(); format!("{} / {count}", if count == 0 { 0 } else { match_index.get() % count + 1 }) }}</span>
                    <IconButton label="Previous match" disabled=Signal::derive(move || matches.get().is_empty()) on_click=Callback::new(move |_| navigate.run(false))><Icon name=IconName::ChevronUp /></IconButton>
                    <IconButton label="Next match" disabled=Signal::derive(move || matches.get().is_empty()) on_click=Callback::new(move |_| navigate.run(true))><Icon name=IconName::ChevronDown /></IconButton>
                    <IconButton label="Close find" on_click=Callback::new(move |_| find_open.set(false))><Icon name=IconName::X /></IconButton>
                </PanelSearchRow>
            </Show>
            <Show
                when=move || open_file.get().is_some()
                fallback=move || {
                    view! {
                        <p class="empty editor-empty">"Select a file to edit, or create a new one in the explorer."</p>
                    }
                }
            >
                {move || {
                    let editor_project = workspace.active_project.get();
                    let mode = view_mode.get();
                    if let Some(diff) = pending_diff.get() {
                        let metadata = workspace.open_file.get().and_then(|path| workspace.persisted_edits.with(|edits| edits.get(&path).and_then(|edit| edit.file.clone())));
                        if metadata.as_ref().is_some_and(|file| file.deleted) {
                            view! { <p class="empty editor-empty">"File deleted. Reject the file changes to restore it."</p> }.into_any()
                        } else if metadata.as_ref().is_some_and(|file| file.binary_after.is_some()) {
                            view! { <p class="empty editor-empty">"Binary contents changed. Use Accept or Reject to review this file."</p> }.into_any()
                        } else { match mode {
                            ViewMode::InlineDiff => render_inline_diff(diff).into_any(),
                            ViewMode::SideBySide => render_side_by_side(diff).into_any(),
                            ViewMode::Preview => {
                                let path = open_file.get().unwrap_or_default();
                                if FileKind::from_path(&path) == FileKind::Markdown {
                                    return render_markdown_diff(&diff).into_any();
                                }
                                render_preview_view(
                                    &path,
                                    &diff.new,
                                    media_url.get(),
                                    view_mode.write_only(),
                                    on_open_lossy,
                                )
                                .into_any()
                            }
                            ViewMode::Code => render_inline_diff(diff).into_any(),
                        }
                        }
                    } else if (mode == ViewMode::InlineDiff || mode == ViewMode::SideBySide) && binary_head.get() {
                        view! { <p class="empty editor-empty">"Binary file — no text diff"</p> }.into_any()
                    } else if (mode == ViewMode::InlineDiff || mode == ViewMode::SideBySide) && git_head_diff.get().is_some() {
                        let diff = git_head_diff.get().unwrap();
                        match mode {
                            ViewMode::InlineDiff => render_inline_diff(diff).into_any(),
                            ViewMode::SideBySide => render_side_by_side(diff).into_any(),
                            _ => ().into_any(),
                        }
                    } else {
                        match mode {
                            ViewMode::Preview => {
                                let path = open_file.get().unwrap_or_default();
                                if FileKind::from_path(&path) == FileKind::Markdown && let Some(diff) = git_head_diff.get() {
                                    return render_markdown_diff(&diff).into_any();
                                }
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
                            _ if open_file.with(|path| path.as_ref().is_some_and(|path| FileKind::from_path(path).is_non_text())) && !read_only.get() => {
                                let path = open_file.get().unwrap_or_default();
                                render_placeholder_view(&path, FileKind::from_path(&path), view_mode.write_only(), on_open_lossy).into_any()
                            }
                            _ => {
                                view! {
                                    <div class="editor-code" style=move || content.with(|text| format!("--editor-gutter-width: calc({}ch + 24px); --editor-tab-width: {}", text.split('\n').count().to_string().len(), editor_actions.rules().indentation.tab_width())) class:highlight-ready=move || highlight_ready.get()>
                                        <HighlightOverlay content=content open_file=open_file node_ref=hl textarea_ref=ta ready=highlight_ready />
                                        <textarea
                                            data-editor-project=editor_project.map(|project| project.to_string())
                                            data-editor-path=open_file.get()
                                            class="editor-textarea"
                                            wrap="off"
                                            spellcheck="false"
                                            readonly=read_only
                                            title="Tab indents; Ctrl+M toggles Tab moving focus"
                                            node_ref=ta
                                            on:select=move |event: web_sys::Event| {
                                                if let Some(textarea) = event.target().and_then(|target| target.dyn_into::<web_sys::HtmlTextAreaElement>().ok()) && current_editor_target(editor_actions, &textarea) { let _ = editor_actions.record_selection(document_selection(&textarea, &workspace.content.get_untracked())); }
                                            }
                                            on:blur=move |event: web_sys::FocusEvent| {
                                                paste_matches_indentation.set(false);
                                                if let Some(textarea) = event.target().and_then(|target| target.dyn_into::<web_sys::HtmlTextAreaElement>().ok()) && current_editor_target(editor_actions, &textarea) { let _ = editor_actions.record_selection(document_selection(&textarea, &workspace.content.get_untracked())); }
                                            }
                                            on:keyup=move |event: web_sys::KeyboardEvent| { if event.key().eq_ignore_ascii_case("v") { paste_matches_indentation.set(false); } }
                                            on:paste=move |event: web_sys::ClipboardEvent| {
                                                let matching = paste_matches_indentation.get_untracked(); paste_matches_indentation.set(false);
                                                if !matching || read_only.get_untracked() { return; }
                                                let Some(textarea) = event.target().and_then(|target| target.dyn_into::<web_sys::HtmlTextAreaElement>().ok()).filter(|textarea| current_editor_target(editor_actions, textarea)) else { return; };
                                                let Some(clipboard) = event.clipboard_data() else { return; };
                                                let Ok(pasted) = clipboard.get_data("text/plain") else { return; };
                                                if pasted.is_empty() { return; }
                                                if let Ok(Some((text, selection))) = editor_actions.paste_with_indentation(&pasted, document_selection(&textarea, &workspace.content.get_untracked())) {
                                                    event.prevent_default(); textarea.set_value(&text); restore_editor_selection(&textarea, &text, selection);
                                                }
                                            }
                                            on:beforeinput=move |event: web_sys::InputEvent| {
                                                if let Some(textarea) = event.target().and_then(|target| target.dyn_into::<web_sys::HtmlTextAreaElement>().ok()) && current_editor_target(editor_actions, &textarea) {
                                                    let selection = document_selection(&textarea, &workspace.content.get_untracked());
                                                    let _ = editor_actions.record_selection(selection);
                                                    if read_only.get_untracked() || event.is_composing() || !event.cancelable() { return; }
                                                    let command = match event.input_type().as_str() {
                                                        "insertLineBreak" | "insertParagraph" => Some(EditorCommand::Newline),
                                                        "deleteContentBackward" => Some(EditorCommand::DeletePair),
                                                        "insertText" => event.data().and_then(|text| pair_character(&text)).map(EditorCommand::TypeCharacter),
                                                        _ => None,
                                                    };
                                                    if let Some(command) = command && apply_editor_command(editor_actions, command, &textarea, &workspace.content.get_untracked()) { event.prevent_default(); }
                                                }
                                            }
                                            on:compositionstart=move |event: web_sys::CompositionEvent| {
                                                if let Some(textarea) = event.target().and_then(|target| target.dyn_into::<web_sys::HtmlTextAreaElement>().ok()) && current_editor_target(editor_actions, &textarea) {
                                                    let _ = editor_actions.record_selection(document_selection(&textarea, &workspace.content.get_untracked())); editor_actions.begin_composition();
                                                }
                                            }
                                            on:compositionend=move |event: web_sys::CompositionEvent| {
                                                if let Some(textarea) = event.target().and_then(|target| target.dyn_into::<web_sys::HtmlTextAreaElement>().ok()) && current_editor_target(editor_actions, &textarea) { editor_actions.end_composition(); }
                                            }
                                            on:keydown=move |event: web_sys::KeyboardEvent| {
                                                if event.is_composing() { return; }
                                                let Some(textarea) = event.target().and_then(|target| target.dyn_into::<web_sys::HtmlTextAreaElement>().ok()).filter(|textarea| current_editor_target(editor_actions, textarea)) else { return; };
                                                let modified = event.ctrl_key() || event.meta_key();
                                                paste_matches_indentation.set(false);
                                                if modified && event.shift_key() && !event.alt_key() && event.key().eq_ignore_ascii_case("v") && !read_only.get_untracked() { paste_matches_indentation.set(true); return; }

                                                if modified && event.key().eq_ignore_ascii_case("m") {
                                                    event.prevent_default(); event.stop_propagation(); tab_moves_focus.update(|value| *value = !*value); return;
                                                }
                                                if read_only.get_untracked() { return; }
                                                let command = match event.key().as_str() {
                                                    "Tab" if !tab_moves_focus.get_untracked() && !modified && !event.alt_key() => Some(if event.shift_key() { EditorCommand::Outdent } else { EditorCommand::Tab }),
                                                    "ArrowUp" if event.alt_key() && !modified => Some(EditorCommand::Line(if event.shift_key() { openwebide_core::editor::LineCommand::DuplicateAbove } else { openwebide_core::editor::LineCommand::MoveUp })),
                                                    "ArrowDown" if event.alt_key() && !modified => Some(EditorCommand::Line(if event.shift_key() { openwebide_core::editor::LineCommand::Duplicate } else { openwebide_core::editor::LineCommand::MoveDown })),
                                                    "Enter" if modified && !event.alt_key() => Some(EditorCommand::Line(if event.shift_key() { openwebide_core::editor::LineCommand::InsertAbove } else { openwebide_core::editor::LineCommand::InsertBelow })),
                                                    "Enter" if !modified && !event.alt_key() => Some(EditorCommand::Newline),
                                                    "/" | "?" if modified && !event.alt_key() => Some(if event.shift_key() { EditorCommand::BlockComment } else { EditorCommand::LineComment }),
                                                    "K" | "k" if modified && event.shift_key() && !event.alt_key() => Some(EditorCommand::Line(openwebide_core::editor::LineCommand::Delete)),
                                                    "D" | "d" if modified && event.shift_key() && !event.alt_key() => Some(EditorCommand::DuplicateSelection),
                                                    key if modified && key.eq_ignore_ascii_case("z") => Some(if event.shift_key() { EditorCommand::Redo } else { EditorCommand::Undo }),
                                                    key if modified && key.eq_ignore_ascii_case("y") => Some(EditorCommand::Redo),
                                                    "Backspace" if !modified && !event.alt_key() => Some(EditorCommand::DeletePair),
                                                    key if !modified && !event.alt_key() => pair_character(key).map(EditorCommand::TypeCharacter),
                                                    _ => None,
                                                };
                                                if let Some(command) = command && apply_editor_command(editor_actions, command, &textarea, &workspace.content.get_untracked()) {
                                                    event.prevent_default(); event.stop_propagation();
                                                }
                                            }
                                            on:input=move |e: web_sys::Event| {
                                                if read_only.get_untracked() { return; }
                                                if let Some(target) = e.target()
                                                    && let Some(textarea) = target.dyn_ref::<web_sys::HtmlTextAreaElement>()
                                                    && current_editor_target(editor_actions, textarea)
                                                {
                                                    let input_type = e.dyn_ref::<web_sys::InputEvent>().map_or_else(String::new, web_sys::InputEvent::input_type);
                                                    let _ = editor_actions.native_input(textarea.value(), editor_selection(textarea), &input_type, e.time_stamp());
                                                }
                                            }
                                            on:scroll=move |_| {
                                                if let (Some(ta_el), Some(hl_el)) = (ta.get(), hl.get()) {
                                                    sync_highlight_scroll(&ta_el, &hl_el);
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
            <Show when=move || open_file.get().is_some() && view_mode.get() == ViewMode::Code && open_file.with(|path| path.as_ref().is_some_and(|path| !FileKind::from_path(path).is_non_text()))>
                <div class="editor-footer">
                    <super::editor_options::IndentationControls above=true value=Signal::derive(move || editor_actions.rules().indentation) disabled=read_only on_change=Callback::new(move |indentation| editor_actions.set_indentation(indentation)) />
                    <Button size=ButtonSize::Sm variant=ButtonVariant::Ghost disabled=read_only on_click=Callback::new(move |_| {
                        if let Some(textarea) = ta.get_untracked() && current_editor_target(editor_actions, &textarea)
                            && let Ok(Some((text, selection))) = editor_actions.command(EditorCommand::ConvertIndentation, document_selection(&textarea, &workspace.content.get_untracked()), editor_actions.rules_untracked().indentation) {
                                textarea.set_value(&text); restore_editor_selection(&textarea, &text, selection); let _ = textarea.focus();
                        }
                    })>"Convert indentation"</Button>
                    <span class="editor-rules-source" title=move || editor_actions.rules().source.unwrap_or_else(|| "Detected from this file, with editor defaults as fallback".into())>{move || if editor_actions.rules().source.is_some() { "EditorConfig" } else { "Detected / defaults" }}</span>
                </div>
            </Show>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::find_matches;

    #[wasm_bindgen_test::wasm_bindgen_test]
    fn find_uses_utf16_offsets_and_logical_lines() {
        assert_eq!(
            find_matches("😀 café\n😀 café", "café"),
            vec![(3, 7, 1), (11, 15, 2)]
        );
        assert!(find_matches("text", "").is_empty());
        assert!(find_matches("Text", "text").is_empty());
        assert_eq!(
            find_matches("a\nb\na\nb", "a\nb"),
            vec![(0, 3, 1), (4, 7, 3)]
        );
    }
}
