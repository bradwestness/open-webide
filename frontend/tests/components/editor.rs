use leptos::prelude::*;
use openwebide_frontend::components::highlight_count;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;

use super::support::{Mounted, editor_view, mount_test, settle, wait_until};

fn mount_editor(content: String) -> Mounted {
    mount_test(move |state| {
        state.seed_project();
        state.workspace.open_file.set(Some("fixture.rs".into()));
        state.workspace.content.set(content);
        editor_view(state)
    })
}

fn editor_key(
    textarea: &web_sys::HtmlTextAreaElement,
    key: &str,
    control: bool,
    shift: bool,
) -> web_sys::KeyboardEvent {
    let init = web_sys::KeyboardEventInit::new();
    init.set_key(key);
    init.set_ctrl_key(control);
    init.set_shift_key(shift);
    init.set_bubbles(true);
    init.set_cancelable(true);
    let event =
        web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    textarea.dispatch_event(&event).unwrap();
    event
}

#[wasm_bindgen_test]
async fn parser_reindent_preserves_literals_and_embedded_boundaries_in_both_modes() {
    use openwebide_core::{
        WorkspaceMode,
        editor::{Indentation, Selection},
    };
    use openwebide_frontend::state_actions::editor::{EditorActions, EditorCommand};
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        for (path, source, expected) in [
            (
                "fixture.html",
                "<script>\nfunction first() {\nx();\n</script>\n<style>\na {\ncolor: red;\n}\n</style>\n<script>\nfunction second() {\ny();\n}\n</script>",
                "<script>\nfunction first() {\n    x();\n</script>\n<style>\na {\n    color: red;\n}\n</style>\n<script>\nfunction second() {\n    y();\n}\n</script>",
            ),
            (
                "fixture.cs",
                "class C {\nvoid F() {\nvar text = @\"文\n  keep 🦀\nlast\";\nCall();\n}\n}",
                "class C {\n    void F() {\n        var text = @\"文\n  keep 🦀\nlast\";\n        Call();\n    }\n}",
            ),
            (
                "fixture.py",
                "values = [\nf\"\"\"first\n  {call()}\nlast\"\"\"\n]",
                "values = [\n    f\"\"\"first\n  {call()}\nlast\"\"\"\n]",
            ),
        ] {
            let mounted = mount_test(move |state| {
                state.seed_project();
                state
                    .projects
                    .projects
                    .update(|projects| projects[0].mode = mode);
                state.workspace.open_file.set(Some(path.into()));
                state.workspace.content.set(source.into());
                editor_view(state)
            });
            let actions = EditorActions::new(mounted.state.workspace);
            let selection = Selection {
                anchor: source.len(),
                head: 0,
            };
            actions
                .command(EditorCommand::Reindent, selection, Indentation::default())
                .unwrap()
                .unwrap();
            assert_eq!(mounted.state.workspace.content.get_untracked(), expected);
            let after = actions.selections(expected)[0];
            actions
                .command(EditorCommand::Undo, after, Indentation::default())
                .unwrap()
                .unwrap();
            assert_eq!(mounted.state.workspace.content.get_untracked(), source);
            // Reusing the parser cache after undo must revalidate its source.
            actions
                .command(EditorCommand::Reindent, selection, Indentation::default())
                .unwrap()
                .unwrap();
            assert_eq!(mounted.state.workspace.content.get_untracked(), expected);
            drop(mounted);
            settle().await;
        }
    }
}

#[wasm_bindgen_test]
async fn parser_block_comments_share_modes_embedded_cursors_and_atomic_undo() {
    use openwebide_core::{
        WorkspaceMode,
        editor::{Document, Indentation, Selection},
    };
    use openwebide_frontend::state_actions::editor::{EditorActions, EditorCommand};
    let source = "<script>call();</script><style>a { color: red; }</style>";
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("comments.html".into()));
            state.workspace.content.set(source.into());
            editor_view(state)
        });
        let actions = EditorActions::new(mounted.state.workspace);
        let primary = Selection::caret(source.find("call").unwrap());
        mounted
            .state
            .workspace
            .editor_documents
            .update(|documents| {
                let mut document = Document::new(source);
                document
                    .set_selections(vec![
                        primary,
                        Selection {
                            anchor: source.find("red").unwrap() + 3,
                            head: source.find("red").unwrap(),
                        },
                    ])
                    .unwrap();
                documents.insert((1, "comments.html".into()), document);
            });
        actions
            .command(EditorCommand::BlockComment, primary, Indentation::default())
            .unwrap()
            .unwrap();
        let edited = "<script>/* call(); */</script><style>a { color: /* red */; }</style>";
        assert_eq!(mounted.state.workspace.content.get_untracked(), edited);
        assert_eq!(actions.selections(edited).len(), 2);
        actions
            .command(
                EditorCommand::BlockComment,
                actions.selections(edited)[0],
                Indentation::default(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(mounted.state.workspace.content.get_untracked(), source);
        actions
            .command(
                EditorCommand::Undo,
                actions.selections(source)[0],
                Indentation::default(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(mounted.state.workspace.content.get_untracked(), edited);
        drop(mounted);
        settle().await;
    }
}

#[wasm_bindgen_test]
async fn parser_line_comments_share_modes_mixed_syntax_and_one_undo_step() {
    use openwebide_core::{
        WorkspaceMode,
        editor::{Document, Indentation, Selection},
    };
    use openwebide_frontend::state_actions::editor::{EditorActions, EditorCommand};
    let source = "<script>\r\ncall();\r\n</script><style>a { color: red; }</style>";
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("comments.html".into()));
            state.workspace.content.set(source.into());
            editor_view(state)
        });
        let actions = EditorActions::new(mounted.state.workspace);
        let primary = Selection::caret(source.find("call").unwrap());
        mounted
            .state
            .workspace
            .editor_documents
            .update(|documents| {
                let mut document = Document::new(source);
                document
                    .set_selections(vec![
                        primary,
                        Selection {
                            anchor: source.find("red").unwrap() + 3,
                            head: source.find("red").unwrap(),
                        },
                    ])
                    .unwrap();
                documents.insert((1, "comments.html".into()), document);
            });
        actions
            .command(EditorCommand::LineComment, primary, Indentation::default())
            .unwrap()
            .unwrap();
        let edited = "<script>\r\n// call();\r\n</script><style>a { color: /* red */; }</style>";
        assert_eq!(mounted.state.workspace.content.get_untracked(), edited);
        let selections = actions.selections(edited);
        assert_eq!(selections.len(), 2);
        assert!(selections[1].anchor > selections[1].head);
        assert_eq!(&edited[selections[1].range()], "red");
        actions
            .command(EditorCommand::Undo, selections[0], Indentation::default())
            .unwrap()
            .unwrap();
        assert_eq!(mounted.state.workspace.content.get_untracked(), source);
        actions
            .command(
                EditorCommand::Redo,
                actions.selections(source)[0],
                Indentation::default(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(mounted.state.workspace.content.get_untracked(), edited);
        actions
            .command(
                EditorCommand::LineComment,
                actions.selections(edited)[0],
                Indentation::default(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(mounted.state.workspace.content.get_untracked(), source);
        drop(mounted);
        settle().await;
    }
}

#[wasm_bindgen_test]
async fn parser_selection_and_navigation_share_modes_and_reject_old_file_events() {
    use openwebide_core::{
        WorkspaceMode,
        editor::{Selection, SelectionCommand},
    };
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let source = "def f():\r\n    value = call(foo)\r\n    return value\r\noutside()";
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("fixture.py".into()));
            state.workspace.content.set(source.into());
            editor_view(state)
        });
        let actions = EditorActions::new(mounted.state.workspace);
        let start = source.find("foo").unwrap();
        actions
            .record_selection(Selection {
                anchor: start + 2,
                head: start + 1,
            })
            .unwrap();
        for expected in [
            "foo",
            "(foo)",
            "call(foo)",
            "value = call(foo)",
            "value = call(foo)\r\n    return value",
            "def f():\r\n    value = call(foo)\r\n    return value",
        ] {
            let selections = actions
                .selection_command(1, "fixture.py", source, SelectionCommand::Expand)
                .unwrap()
                .unwrap();
            assert_eq!(&source[selections[0].range()], expected);
            assert!(selections[0].anchor > selections[0].head);
        }
        let selections = actions
            .selection_command(1, "fixture.py", source, SelectionCommand::Shrink)
            .unwrap()
            .unwrap();
        assert_eq!(
            &source[selections[0].range()],
            "value = call(foo)\r\n    return value"
        );
        assert_eq!(mounted.state.workspace.content.get_untracked(), source);
        let embedded = "<script>const t = `text ${call(foo)} tail`;</script><style>a {}</style>";
        mounted
            .state
            .workspace
            .open_file
            .set(Some("fixture.html".into()));
        mounted.state.workspace.content.set(embedded.into());
        let open = embedded.find("(foo)").unwrap();
        assert_eq!(actions.matching_bracket(open), Some((open, open + 4)));
        assert!(
            actions
                .selection_command(1, "fixture.py", source, SelectionCommand::Expand)
                .unwrap()
                .is_none()
        );
        assert_eq!(mounted.state.workspace.content.get_untracked(), embedded);
        let context = actions.syntax_structure(|| true).unwrap();
        assert!(context.is_code(open));
        assert!(!context.is_code(embedded.find(" tail").unwrap()));
        // Selecting bracket mates goes through the existing source-coordinate facade.
        actions.navigate(open + 4).unwrap();
        assert_eq!(
            actions.selections(embedded),
            vec![Selection::caret(open + 4)]
        );
        assert_eq!(actions.matching_bracket(open + 4), Some((open + 4, open)));
        drop(mounted);
        settle().await;
    }
}

#[wasm_bindgen_test]
async fn grammar_highlights_paint_embedded_code_and_preserve_crlf_overlay_in_both_modes() {
    use openwebide_core::WorkspaceMode;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let source = "<script>\r\nconst t = `文 ${call(42)} tail`;\r\n</script><style>a { color: red; }</style>";
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("paint.html".into()));
            state.workspace.content.set(source.into());
            editor_view(state)
        });
        wait_until("grammar colors", || {
            mounted
                .root
                .query_selector(".tok-function")
                .unwrap()
                .is_some()
        })
        .await;
        assert_eq!(
            mounted.element(".tok-function").text_content().unwrap(),
            "call"
        );
        assert_eq!(
            mounted.element(".tok-attribute").text_content().unwrap(),
            "color"
        );
        assert_eq!(
            mounted
                .element(".editor-highlight-content")
                .text_content()
                .unwrap(),
            source.replace("\r\n", "\n")
        );
        assert_eq!(mounted.state.workspace.content.get_untracked(), source);
        drop(mounted);
        settle().await;
    }
}

#[wasm_bindgen_test]
async fn selection_policy_and_multi_commands_share_both_modes_and_reject_stale_targets() {
    use openwebide_core::{
        WorkspaceMode,
        editor::{Indentation, Selection, SelectionCommand},
    };
    use openwebide_frontend::state_actions::editor::{EditorActions, EditorCommand};
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let source = "文 foo\r\nfoo";
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("fixture.rs".into()));
            state.workspace.content.set(source.into());
            editor_view(state)
        });
        let actions = EditorActions::new(mounted.state.workspace);
        let project = mounted
            .state
            .workspace
            .active_project
            .get_untracked()
            .unwrap();
        actions
            .record_selection(Selection { anchor: 4, head: 7 })
            .unwrap();
        let selections = actions
            .selection_command(
                project,
                "fixture.rs",
                source,
                SelectionCommand::AllOccurrences,
            )
            .unwrap()
            .unwrap();
        assert_eq!(selections.len(), 2);
        assert_eq!(actions.selections(source), selections);
        assert!(!mounted.state.workspace.dirty.get_untracked());
        for (project, path, source) in [
            (project + 1, "fixture.rs", source),
            (project, "other.rs", source),
            (project, "fixture.rs", "stale"),
        ] {
            assert!(
                actions
                    .selection_command(project, path, source, SelectionCommand::Single)
                    .unwrap()
                    .is_none()
            );
        }
        assert_eq!(actions.selections(source).len(), 2);
        let (paired, primary) = actions
            .command(
                EditorCommand::TypeCharacter('('),
                selections[0],
                Indentation::default(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(paired, "文 (foo)\r\n(foo)");
        assert_eq!(actions.selections(&paired).len(), 2);
        let (pasted, primary) = actions
            .paste_with_indentation("😀", primary)
            .unwrap()
            .unwrap();
        assert_eq!(pasted, "文 (😀)\r\n(😀)");
        assert_eq!(actions.selections(&pasted).len(), 2);
        let (restored, primary) = actions
            .command(EditorCommand::Undo, primary, Indentation::default())
            .unwrap()
            .unwrap();
        assert_eq!(restored, paired);
        assert_eq!(actions.selections(&restored).len(), 2);
        let (restored, _) = actions
            .command(EditorCommand::Undo, primary, Indentation::default())
            .unwrap()
            .unwrap();
        assert_eq!(restored, source);
        assert_eq!(actions.selections(&restored), selections);
        mounted
            .state
            .workspace
            .open_file
            .set(Some("other.rs".into()));
        assert!(
            actions
                .selection_command(project, "fixture.rs", source, SelectionCommand::Single)
                .unwrap()
                .is_none()
        );
        assert!(actions.selections(source).is_empty());
        assert_eq!(mounted.state.workspace.content.get_untracked(), source);
    }
}

#[wasm_bindgen_test]
async fn multiline_clipboard_fragments_round_trip_in_both_modes_and_reject_bad_metadata() {
    use openwebide_core::{
        WorkspaceMode,
        editor::{CLIPBOARD_SELECTIONS_MIME, Selection, byte_to_textarea},
    };
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let source = "文\r\na\n---\n😀\nb";
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("fragments.rs".into()));
            state.workspace.content.set(source.into());
            editor_view(state)
        });
        settle().await;
        frame().await;
        let actions = EditorActions::new(mounted.state.workspace);
        let textarea: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        actions
            .record_selection(Selection {
                anchor: source.len(),
                head: 11,
            })
            .unwrap();
        // Seed two disjoint, multiline ranges in primary-first order.
        mounted
            .state
            .workspace
            .editor_documents
            .update(|documents| {
                documents
                    .get_mut(&(1, "fragments.rs".into()))
                    .unwrap()
                    .set_selections(vec![
                        Selection {
                            anchor: source.len(),
                            head: 11,
                        },
                        Selection { anchor: 0, head: 7 },
                    ])
                    .unwrap();
            });
        textarea
            .set_selection_range_with_direction(
                u32::try_from(byte_to_textarea(source, 11).unwrap()).unwrap(),
                u32::try_from(byte_to_textarea(source, source.len()).unwrap()).unwrap(),
                "backward",
            )
            .unwrap();
        let cut = editorClipboardCut(&textarea);
        let data = cut
            .unchecked_ref::<web_sys::ClipboardEvent>()
            .clipboard_data()
            .unwrap();
        assert_eq!(data.get_data("text/plain").unwrap(), "😀\nb\n文\r\na\n");
        assert!(!data.get_data(CLIPBOARD_SELECTIONS_MIME).unwrap().is_empty());
        assert_eq!(actions.source(), "---\n");
        assert!(editorClipboardPasteData(&textarea, &cut).default_prevented());
        assert_eq!(actions.source(), source);
        editor_key(&textarea, "z", true, false);
        assert_eq!(actions.source(), "---\n");
        editor_key(&textarea, "z", true, false);
        assert_eq!(actions.source(), source);
        assert_eq!(
            actions.selections(source)[0],
            Selection {
                anchor: source.len(),
                head: 11
            }
        );
        editor_key(&textarea, "v", true, true);
        assert!(editorClipboardPasteData(&textarea, &cut).default_prevented());
        assert_eq!(actions.source(), "文\r\na\r\n---\n😀\r\nb");
        editor_key(&textarea, "z", true, false);
        assert_eq!(actions.source(), source);
        data.set_data(CLIPBOARD_SELECTIONS_MIME, "{broken").unwrap();
        data.set_data("text/plain", "a\nb").unwrap();
        assert!(editorClipboardPasteData(&textarea, &cut).default_prevented());
        assert_eq!(actions.source(), "b---\na");
        actions.begin_composition();
        let before = mounted.state.workspace.editor_documents.get_untracked();
        assert_eq!(
            actions.paste_clipboard_with_indentation("x", None, Selection::caret(0)),
            Err(openwebide_core::editor::EditError::CompositionActive)
        );
        assert_eq!(
            mounted.state.workspace.editor_documents.get_untracked(),
            before
        );
        actions.cancel_composition();
    }
}

#[wasm_bindgen_test]
async fn multi_cursor_pointer_and_column_gestures_share_both_modes() {
    use openwebide_core::{WorkspaceMode, editor::Selection};
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("columns.rs".into()));
            state.workspace.content.set("a\t文z\r\nxy\r\na\t文z".into());
            view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:600px;height:350px">{editor_view(state)}</div> }
        });
        settle().await;
        frame().await;
        let actions = EditorActions::new(mounted.state.workspace);
        let textarea: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        textarea.set_selection_range(0, 0).unwrap();
        actions.record_selection(Selection::caret(0)).unwrap();
        assert!(editorGesture(&textarea, 3, 3, false, false).default_prevented());
        assert_eq!(
            actions.selections(&actions.source()),
            vec![Selection::caret(17), Selection::caret(0)]
        );
        assert!(editorGesture(&textarea, 3, 3, false, false).default_prevented());
        assert_eq!(
            actions.selections(&actions.source()),
            vec![Selection::caret(0)]
        );
        textarea.set_selection_range(1, 1).unwrap();
        actions.record_selection(Selection::caret(1)).unwrap();
        assert!(editorGesture(&textarea, 3, 4, true, false).default_prevented());
        assert_eq!(actions.selections(&actions.source()).len(), 3);
        assert_eq!(editorClipboardCopy(&textarea), "\t文z\n\t文z\ny");
        assert!(editorGesture(&textarea, 3, 3, true, true).default_prevented());
        assert_eq!(editorClipboardCopy(&textarea), "\t文\n\t文\ny");
        let before = actions.selections(&actions.source());
        mounted.state.workspace.content.set("external".into());
        settle().await;
        editorGesture(&textarea, 1, 1, true, true);
        assert_eq!(actions.source(), "external");
        assert_ne!(
            actions.selections(&actions.source()),
            before,
            "stale drag cannot restore old selections"
        );
        mounted
            .state
            .workspace
            .content
            .set("fn foo() {}\r\nfn bar() {}".into());
        settle().await;
        frame().await;
        textarea.set_selection_range(0, 0).unwrap();
        actions.record_selection(Selection::caret(0)).unwrap();
        editor_key(&textarea, "d", true, false);
        editor_key(&textarea, "d", true, false);
        wait_until("keyword selection paint", || {
            mounted
                .root
                .query_selector(".editor-secondary-selection")
                .unwrap()
                .is_some()
        })
        .await;
        assert_eq!(
            mounted
                .root
                .query_selector_all(".editor-secondary-selection")
                .unwrap()
                .length(),
            1,
            "nested syntax tokens must not stack translucent selection rectangles"
        );
        assert_eq!(editorClipboardCopy(&textarea), "fn\nfn");
        assert!(!mounted.state.workspace.dirty.get_untracked());
    }
}

fn assert_editor_native_source(
    input: &web_sys::HtmlTextAreaElement,
    workspace: openwebide_frontend::state::workspace::WorkspaceState,
    source: &str,
) {
    let actions = openwebide_frontend::state_actions::editor::EditorActions::new(workspace);
    assert_eq!(actions.source(), source);
    let normalized = source.replace("\r\n", "\n").replace('\r', "\n");
    if let Some(context) = actions.bound_native_context() {
        assert_eq!(input.value(), context.projection().textarea_text());
        assert!(normalized.contains(&input.value()));
        assert!(input.value().len() <= 16 * 1024);
    } else {
        assert_eq!(input.value(), normalized);
    }
}

#[wasm_bindgen_test]
async fn prepared_source_extents_ignore_native_dimensions_and_reject_stale_scopes_in_both_modes() {
    use openwebide_core::WorkspaceMode;
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        for wrap in [false, true] {
            let wide = "wide 文😀 ".repeat(160);
            let source = (0..700)
                .map(|row| {
                    if row == 650 {
                        format!("{wide}\r\n")
                    } else {
                        format!("row {row}\t文😀\r\n")
                    }
                })
                .collect::<String>();
            let initial = source.clone();
            let actions_slot = std::rc::Rc::new(std::cell::Cell::new(None));
            let mounted_actions = actions_slot.clone();
            let mounted = mount_test(move |state| {
                state.seed_project();
                state
                    .projects
                    .projects
                    .update(|projects| projects[0].mode = mode);
                state
                    .settings
                    .editor_preferences
                    .update(|preferences| preferences.word_wrap = wrap);
                state.workspace.open_file.set(Some("extents.txt".into()));
                state.workspace.content.set(initial);
                mounted_actions.set(Some(EditorActions::new(state.workspace)));
                view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:420px;height:240px">{editor_view(state)}</div> }
            });
            frame().await;
            let actions = actions_slot.get().unwrap();
            let textarea: web_sys::HtmlTextAreaElement =
                mounted.element(".editor-textarea").unchecked_into();
            wait_until("prepared source dimensions", || {
                mounted
                    .element(".editor-scroll-extent")
                    .has_attribute("data-source-width")
            })
            .await;
            frame().await;
            let extent = mounted.element(".editor-scroll-extent");
            let scroll = openwebide_frontend::viewport::editor_scroll(&textarea);
            let full = fullNativeDimensions(&textarea, &source);
            let native_width = full[0];
            let native_height = full[1];
            assert!(
                (scroll.scroll_width() - (native_width + if wrap { 0 } else { 16 })).abs() <= 2,
                "source width must match full layout: mode={mode:?} wrap={wrap} source={} native={native_width}",
                scroll.scroll_width()
            );
            assert!(
                (scroll.scroll_height() - native_height).abs() <= 2,
                "source height must match full layout: mode={mode:?} wrap={wrap} source={} native={native_height}",
                scroll.scroll_height()
            );
            if !wrap {
                scroll.set_scroll_left(f64::from(scroll.scroll_width()));
                openwebide_frontend::viewport::sync_editor_scroll(&textarea, false);
                let source_end = scroll.scroll_left();
                assert!(
                    source_end > textarea.scroll_left(),
                    "source viewport retains trailing padding independently of native clamping"
                );
                openwebide_frontend::viewport::sync_editor_scroll(&textarea, true);
                assert!(
                    (scroll.scroll_left() - source_end).abs() < 0.1,
                    "native scroll echo must not rewind the source viewport"
                );
            }
            let counts = watchNativeExtentReads(&textarea);
            for _ in 0..3 {
                openwebide_frontend::viewport::refresh_editor_scroll(&textarea);
            }
            assert_eq!(
                counts.call0(&wasm_bindgen::JsValue::NULL).unwrap().as_f64(),
                Some(0.0),
                "prepared extents must not read the full input dimensions"
            );
            let scope = extent.get_attribute("data-editor-scope").unwrap();
            extent.set_attribute("data-editor-scope", "stale").unwrap();
            openwebide_frontend::viewport::refresh_editor_scroll(&textarea);
            assert_eq!(
                counts.call0(&wasm_bindgen::JsValue::NULL).unwrap().as_f64(),
                Some(if textarea.has_attribute("data-editor-native-bound") {
                    0.0
                } else {
                    2.0
                })
            );
            extent.set_attribute("data-editor-scope", &scope).unwrap();
            let account = extent.get_attribute("data-editor-account").unwrap();
            extent
                .set_attribute("data-editor-account", "stale")
                .unwrap();
            openwebide_frontend::viewport::refresh_editor_scroll(&textarea);
            assert_eq!(
                counts.call0(&wasm_bindgen::JsValue::NULL).unwrap().as_f64(),
                Some(if textarea.has_attribute("data-editor-native-bound") {
                    0.0
                } else {
                    4.0
                }),
                "stale account dimensions must never use a local native window as source geometry"
            );
            extent
                .set_attribute("data-editor-account", &account)
                .unwrap();
            restoreNativeExtentReads(&textarea);
            assert!(
                !openwebide_frontend::viewport::check_editor_extent(
                    1_000_000_000.0,
                    1_000_000_000.0
                ),
                "physical browser limits must remain a validation gate"
            );
            let replacement = source.replace(&wide, "tiny");
            mounted.state.workspace.content.set(replacement.clone());
            wait_until("shrunk source width/height", || {
                actions.measured_rows().is_some_and(|measured| {
                    measured.rows.width().is_some_and(|width| width < 500.0)
                        && mounted
                            .element(".editor-scroll-extent")
                            .get_attribute("data-editor-scope")
                            == textarea.get_attribute("data-editor-scope")
                })
            })
            .await;
            frame().await;
            assert!(scroll.scroll_width() < native_width || wrap);
            assert!(scroll.scroll_height() < native_height || !wrap);
            assert_eq!(actions.source(), replacement);
            mounted
                .state
                .auth
                .generation
                .update(|generation| *generation += 1);
            assert!(
                actions.measured_rows().is_none(),
                "another account cannot reuse previous measurements"
            );
            frame().await;
            if let Some(measured) = actions.measured_rows() {
                assert_eq!(
                    measured.account_generation,
                    mounted.state.auth.generation.get_untracked()
                );
                assert_ne!(
                    mounted
                        .element(".editor-scroll-extent")
                        .get_attribute("data-editor-account"),
                    Some(account)
                );
            }
        }
    }
}

#[wasm_bindgen_test]
async fn primary_pointer_units_and_drag_use_source_in_both_modes() {
    use openwebide_core::{WorkspaceMode, editor::Selection};
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        for wrap in [false, true] {
            let source = "one 文_foo\r\nnext\r\nlast";
            let mounted = mount_test(move |state| {
                state.seed_project();
                state
                    .projects
                    .projects
                    .update(|projects| projects[0].mode = mode);
                state
                    .settings
                    .editor_preferences
                    .update(|preferences| preferences.word_wrap = wrap);
                state.workspace.open_file.set(Some("pointer.txt".into()));
                state.workspace.content.set(source.into());
                view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:420px;height:240px">{editor_view(state)}</div> }
            });
            frame().await;
            let actions = EditorActions::new(mounted.state.workspace);
            let textarea: web_sys::HtmlTextAreaElement =
                mounted.element(".editor-textarea").unchecked_into();
            textarea.set_selection_range(0, 3).unwrap();
            assert_eq!(
                editorClipboardCopy(&textarea),
                "one",
                "copy must reconcile native selection before testing whether it is empty"
            );
            assert!(editorClipboardCut(&textarea).default_prevented());
            assert_eq!(actions.source(), &source[3..]);
            editor_key(&textarea, "z", true, false);
            assert_eq!(actions.source(), source);
            frame().await;
            assert!(
                editorPrimaryGesture(&textarea, 1, 2, 1, false, "mousedown").default_prevented(),
                "source pointer not ready in {mode:?}, wrap={wrap}; class={:?}, preparing={:?}",
                mounted.element(".editor-code").class_name(),
                mounted
                    .state
                    .workspace
                    .editor_row_preparation
                    .get_untracked(),
            );
            assert_eq!(actions.selection(source), Some(Selection::caret(2)));
            assert!(
                editorPrimaryGesture(&textarea, 2, 2, 1, false, "mousemove").default_prevented()
            );
            assert_eq!(
                actions.selection(source),
                Some(Selection {
                    anchor: 2,
                    head: 15
                })
            );
            assert_eq!(editorClipboardCopy(&textarea), &source[2..15]);
            editorPrimaryGesture(&textarea, 2, 2, 1, false, "mouseup");
            editorPrimaryGesture(&textarea, 3, 2, 1, false, "mousemove");
            assert_eq!(
                actions.selection(source),
                Some(Selection {
                    anchor: 2,
                    head: 15
                })
            );
            editorPrimaryGesture(&textarea, 1, 5, 2, false, "mousedown");
            assert_eq!(editorClipboardCopy(&textarea), "文_foo");
            editorPrimaryGesture(&textarea, 1, 0, 2, false, "mousemove");
            assert_eq!(
                actions.selection(source),
                Some(Selection {
                    anchor: 11,
                    head: 0
                })
            );
            assert!(editorClickContainerFallback(&textarea, 1, 2, false));
            assert_eq!(actions.selection(source), Some(Selection::caret(2)));
            assert!(editorClickContainerFallback(&textarea, 1, 9, true));
            assert_eq!(actions.selection(source), Some(Selection::caret(11)));
            assert!(editorClickPosition(&textarea, 1, 2, false));
            assert_eq!(actions.selection(source), Some(Selection::caret(2)));
            assert!(editorClickPosition(&textarea, 1, 9, true));
            assert_eq!(actions.selection(source), Some(Selection::caret(11)));
            editorPrimaryGesture(&textarea, 2, 2, 3, false, "mousedown");
            assert_eq!(editorClipboardCopy(&textarea), "next\r\n");
            editorPrimaryGesture(&textarea, 3, 2, 1, true, "mousedown");
            assert_eq!(
                actions.selection(source),
                Some(Selection {
                    anchor: 13,
                    head: 21
                })
            );
            for changed in 0..3 {
                frame().await;
                editorPrimaryGesture(&textarea, 3, 2, 1, false, "mousedown");
                let before = actions.selection(source);
                match changed {
                    0 => mounted
                        .state
                        .workspace
                        .editor_read_revision
                        .update(|revision| *revision += 1),
                    1 => mounted
                        .state
                        .auth
                        .generation
                        .update(|generation| *generation += 1),
                    _ => mounted
                        .state
                        .workspace
                        .pending_epoch
                        .update(|epoch| *epoch += 1),
                }
                editorPrimaryGesture(&textarea, 1, 1, 1, false, "mousemove");
                assert_eq!(
                    actions.selection(source),
                    before,
                    "stale read/account/project epoch must discard the drag"
                );
            }
            assert_eq!(actions.source(), source);
            assert!(!mounted.state.workspace.dirty.get_untracked());
        }
    }
}

#[wasm_bindgen_test]
async fn stationary_selection_drag_scrolls_and_stops_in_both_modes() {
    use openwebide_core::WorkspaceMode;
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let source = (0..100)
            .map(|line| format!("row {line} {}\n", "abcdef ".repeat(100)))
            .collect::<String>();
        let content = source.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state
                .settings
                .editor_preferences
                .update(|preferences| preferences.word_wrap = false);
            state.workspace.open_file.set(Some("drag.txt".into()));
            state.workspace.content.set(content);
            view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:420px;height:260px">{editor_view(state)}</div> }
        });
        frame().await;
        let actions = EditorActions::new(mounted.state.workspace);
        let input: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        let scroll = openwebide_frontend::viewport::editor_scroll(&input);
        assert!(editorPrimaryGesture(&input, 1, 2, 1, false, "mousedown").default_prevented());
        assert!(editorEdgeGesture(&input, false, true, "mousemove").default_prevented());
        wait_until("stationary drag scroll and source selection", || {
            scroll.scroll_top() > 80.0
                && actions
                    .selection(&source)
                    .is_some_and(|selection| selection.head > 2000)
        })
        .await;
        let first_top = scroll.scroll_top();
        let first_head = actions.selection(&source).unwrap().head;
        wait_until("stationary vertical drag keeps extending", || {
            scroll.scroll_top() > first_top + 50.0
                && actions
                    .selection(&source)
                    .is_some_and(|selection| selection.head > first_head)
        })
        .await;
        let anchor = actions.selection(&source).unwrap().anchor;
        editorEdgeGesture(&input, false, true, "mouseup");
        frame().await;
        let stopped = scroll.scroll_top();
        let selected = actions.selection(&source);
        for _ in 0..4 {
            frame().await;
        }
        assert!((scroll.scroll_top() - stopped).abs() < 0.1);
        assert_eq!(actions.selection(&source), selected);
        assert_eq!(selected.unwrap().anchor, anchor);
        openwebide_frontend::viewport::set_editor_scroll_top(&input, 0.0);
        frame().await;
        editorPrimaryGesture(&input, 1, 2, 1, false, "mousedown");
        editorEdgeGesture(&input, true, true, "mousemove");
        wait_until("horizontal stationary drag", || {
            scroll.scroll_left() > 80.0
                && actions
                    .selection(&source)
                    .is_some_and(|selection| selection.head > 40)
        })
        .await;
        let first_left = scroll.scroll_left();
        let first_head = actions.selection(&source).unwrap().head;
        wait_until("stationary horizontal drag keeps extending", || {
            scroll.scroll_left() > first_left + 50.0
                && actions
                    .selection(&source)
                    .is_some_and(|selection| selection.head > first_head + 4)
        })
        .await;
        let right = scroll.scroll_left();
        let right_head = actions.selection(&source).unwrap().head;
        editorEdgeGesture(&input, true, false, "mousemove");
        wait_until("reverse horizontal drag", || {
            scroll.scroll_left() < right - 20.0
                && actions
                    .selection(&source)
                    .is_some_and(|selection| selection.head < right_head)
        })
        .await;
        mounted
            .state
            .workspace
            .editor_read_revision
            .update(|revision| *revision += 1);
        frame().await;
        let stale = scroll.scroll_left();
        for _ in 0..4 {
            frame().await;
        }
        assert!((scroll.scroll_left() - stale).abs() < 0.1);
        assert_eq!(actions.source(), source);
        assert!(!mounted.state.workspace.dirty.get_untracked());
    }
}

#[wasm_bindgen_test]
async fn primary_caret_and_selection_follow_source_motion_and_scroll_in_both_modes() {
    use openwebide_core::{WorkspaceMode, editor::Selection};
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        for wrap in [false, true] {
            let source = format!("a\u{301}文😀\tvalue\r\n{}last", "row\r\n".repeat(80));
            let initial = source.clone();
            let mounted = mount_test(move |state| {
                state.seed_project();
                state
                    .projects
                    .projects
                    .update(|projects| projects[0].mode = mode);
                state
                    .settings
                    .editor_preferences
                    .update(|preferences| preferences.word_wrap = wrap);
                state.workspace.open_file.set(Some("primary.txt".into()));
                state.workspace.content.set(initial);
                view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:420px;height:240px">{editor_view(state)}</div> }
            });
            frame().await;
            let actions = EditorActions::new(mounted.state.workspace);
            let textarea: web_sys::HtmlTextAreaElement =
                mounted.element(".editor-textarea").unchecked_into();
            textarea.focus().unwrap();
            actions.record_selection(Selection::caret(3)).unwrap();
            textarea.set_selection_range(2, 2).unwrap();
            assert!(editor_key(&textarea, "ArrowRight", false, false).default_prevented());
            assert_eq!(actions.selection(&source), Some(Selection::caret(6)));
            frame().await;
            wait_until("primary source caret", || {
                mounted
                    .root
                    .query_selector(".editor-primary-caret")
                    .unwrap()
                    .is_some()
            })
            .await;
            let row = mounted.element(".editor-source-line[data-line='1']");
            let fragment = row
                .query_selector(":scope > .editor-source-fragment")
                .unwrap()
                .unwrap();
            let node = fragment.first_child().unwrap();
            let range = document().create_range().unwrap();
            range.set_start(&node, 3).unwrap();
            range.collapse_with_to_start(true);
            let expected = range.get_bounding_client_rect();
            let caret = mounted
                .element(".editor-primary-caret")
                .get_bounding_client_rect();
            assert!(
                (caret.left() - expected.left()).abs() < 0.5,
                "primary caret left={} expected={} mode={mode:?} wrap={wrap}",
                caret.left(),
                expected.left()
            );
            assert!(
                (caret.top() - expected.top()).abs() < 0.5,
                "primary caret top={} expected={} mode={mode:?} wrap={wrap}",
                caret.top(),
                expected.top()
            );
            assert!(textarea.class_list().contains("editor-visual-carets"));
            assert!(editor_key(&textarea, "ArrowRight", false, true).default_prevented());
            assert_eq!(
                actions.selection(&source),
                Some(Selection {
                    anchor: 6,
                    head: 10
                })
            );
            wait_until("primary source selection", || {
                mounted
                    .root
                    .query_selector(".editor-primary-selection")
                    .unwrap()
                    .is_some()
            })
            .await;
            assert!(textarea.class_list().contains("editor-source-selections"));
            assert_eq!(
                actions
                    .clipboard_content(Selection {
                        anchor: 6,
                        head: 10
                    })
                    .unwrap()
                    .unwrap()
                    .text,
                "😀"
            );
            assert!(editor_key(&textarea, "End", true, false).default_prevented());
            assert_eq!(
                actions.selection(&source),
                Some(Selection::caret(source.len()))
            );
            frame().await;
            wait_until("document end caret visible", || {
                let Some(caret) = mounted
                    .root
                    .query_selector(".editor-primary-caret")
                    .unwrap()
                else {
                    return false;
                };
                let bounds = caret.get_bounding_client_rect();
                let viewport = mounted.element(".editor-code").get_bounding_client_rect();
                bounds.top() >= viewport.top() && bounds.bottom() <= viewport.bottom() + 0.5
            })
            .await;
            assert!(openwebide_frontend::viewport::editor_scroll(&textarea).scroll_top() > 100.0);
            assert_eq!(actions.source(), source);
            assert!(!mounted.state.workspace.dirty.get_untracked());
        }
    }
}

#[wasm_bindgen_test]
async fn multi_cursor_shortcuts_motion_and_paint_share_both_modes() {
    use openwebide_core::WorkspaceMode;
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let source = format!("foo {}\r\nfoo", "x".repeat(500));
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("cursors.rs".into()));
            state.workspace.content.set(source.clone());
            view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:600px;height:350px">{editor_view(state)}</div> }
        });
        settle().await;
        frame().await;
        let actions = EditorActions::new(mounted.state.workspace);
        let textarea: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        textarea.set_selection_range(0, 0).unwrap();
        assert!(editor_key(&textarea, "d", true, false).default_prevented());
        assert_eq!(actions.selections(&actions.source()).len(), 1);
        assert!(editor_key(&textarea, "d", true, false).default_prevented());
        assert_eq!(actions.selections(&actions.source()).len(), 2);
        wait_until("painted secondary selection", || {
            mounted
                .root
                .query_selector(".editor-secondary-selection")
                .unwrap()
                .is_some()
        })
        .await;
        assert_eq!(editorClipboardCopy(&textarea), "foo\nfoo");
        assert!(editor_key(&textarea, "ArrowRight", false, false).default_prevented());
        wait_until("painted secondary caret", || {
            mounted
                .root
                .query_selector(".editor-secondary-caret")
                .unwrap()
                .is_some()
        })
        .await;
        assert_eq!(actions.selections(&actions.source()).len(), 2);
        let before = mounted
            .element(".editor-secondary-caret")
            .get_bounding_client_rect();
        // Next occurrence promotes the newly added range to primary; the first
        // source occurrence is the secondary caret after collapsing selections.
        let row = mounted.element(".editor-source-line[data-line='1']");
        let node = row
            .query_selector(":scope > .editor-source-fragment")
            .unwrap()
            .unwrap()
            .first_child()
            .unwrap();
        let range = document().create_range().unwrap();
        range.set_start(&node, 3).unwrap();
        range.set_end(&node, 3).unwrap();
        let expected = range.get_bounding_client_rect();
        assert!(
            (before.top() - expected.top()).abs() < 0.5,
            "secondary caret must align with the text baseline: actual={} expected={} style={:?} parent={}",
            before.top(),
            expected.top(),
            mounted
                .element(".editor-secondary-caret")
                .get_attribute("style"),
            mounted
                .element(".editor-code")
                .get_bounding_client_rect()
                .top()
        );
        assert!(
            (before.left() - expected.left()).abs() < 0.5,
            "secondary caret must align with its source column"
        );
        textarea.set_scroll_left(16.0);
        textarea
            .dispatch_event(&web_sys::Event::new("scroll").unwrap())
            .unwrap();
        settle().await;
        let after = mounted
            .element(".editor-secondary-caret")
            .get_bounding_client_rect();
        assert!((before.left() - after.left() - textarea.scroll_left()).abs() < 1.0);
        assert!(editor_key(&textarea, "ArrowLeft", false, true).default_prevented());
        assert_eq!(editorClipboardCopy(&textarea), "o\no");
        assert!(editor_key(&textarea, "Escape", false, false).default_prevented());
        settle().await;
        assert_eq!(actions.selections(&actions.source()).len(), 1);
        assert!(
            mounted
                .root
                .query_selector(".editor-secondary-caret, .editor-secondary-selection")
                .unwrap()
                .is_none()
        );
        assert!(
            mounted
                .element(".editor-code [role='status']")
                .text_content()
                .unwrap()
                .contains("1 editor cursor")
        );
        mounted.click("button[aria-label='Editing commands']");
        settle().await;
        mounted.click_text("Select all occurrences");
        settle().await;
        assert_eq!(actions.selections(&actions.source()).len(), 4);
        assert!(
            document()
                .active_element()
                .unwrap()
                .is_same_node(Some(&textarea))
        );
        mounted.click("button[aria-label='Editing commands']");
        settle().await;
        mounted.click_text("Keep primary cursor");
        settle().await;
        assert_eq!(actions.selections(&actions.source()).len(), 1);
        mounted
            .state
            .settings
            .editor_preferences
            .update(|preferences| preferences.word_wrap = true);
        let source = actions.source();
        actions
            .record_selection(openwebide_core::editor::Selection::caret(504))
            .unwrap();
        actions
            .toggle_cursor(1, "cursors.rs", &source, source.len())
            .unwrap();
        let end = u32::try_from(
            openwebide_core::editor::byte_to_textarea(&source, source.len()).unwrap(),
        )
        .unwrap();
        textarea.set_selection_range(end, end).unwrap();
        frame().await;
        settle().await;
        wait_until("wrapped secondary caret", || {
            mounted
                .root
                .query_selector(".editor-secondary-caret")
                .unwrap()
                .is_some()
        })
        .await;
        let wrapped = mounted
            .element(".editor-secondary-caret")
            .get_bounding_client_rect();
        let row = mounted.element(".editor-source-line[data-line='1']");
        let range = document().create_range().unwrap();
        let node = row
            .query_selector(":scope > .editor-source-fragment")
            .unwrap()
            .unwrap()
            .first_child()
            .unwrap();
        range.set_start(&node, 504).unwrap();
        range.collapse_with_to_start(true);
        let expected = range.get_bounding_client_rect();
        assert!((wrapped.top() - expected.top()).abs() < 0.5);
        assert!((wrapped.left() - expected.left()).abs() < 0.5);
        assert!(!mounted.state.workspace.dirty.get_untracked());
    }
}

#[wasm_bindgen_test]
async fn multi_selection_clipboard_uses_source_ranges_and_atomic_history_in_both_modes() {
    use openwebide_core::{
        WorkspaceMode,
        editor::{Selection, SelectionCommand},
    };
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("clipboard.rs".into()));
            state.workspace.content.set("foo\r\nfoo".into());
            editor_view(state)
        });
        settle().await;
        frame().await;
        let actions = EditorActions::new(mounted.state.workspace);
        let textarea: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        textarea.set_selection_range(0, 3).unwrap();
        actions
            .record_selection(Selection { anchor: 0, head: 3 })
            .unwrap();
        actions
            .selection_command(
                1,
                "clipboard.rs",
                "foo\r\nfoo",
                SelectionCommand::AllOccurrences,
            )
            .unwrap();
        assert_eq!(editorClipboardCopy(&textarea), "foo\nfoo");
        assert!(editorClipboardPaste(&textarea, "文\r\n😀").default_prevented());
        assert_eq!(actions.source(), "文\r\n😀");
        editor_key(&textarea, "z", true, false);
        assert_eq!(actions.source(), "foo\r\nfoo");
        assert_eq!(actions.selections(&actions.source()).len(), 2);
        assert!(editorClipboardPaste(&textarea, "first\nsecond\nthird").default_prevented());
        assert_eq!(
            actions.source(),
            "first\nsecond\nthird\r\nfirst\nsecond\nthird"
        );
        editor_key(&textarea, "z", true, false);
        let cut = editorClipboardCut(&textarea);
        assert!(cut.default_prevented());
        assert_eq!(
            cut.unchecked_ref::<web_sys::ClipboardEvent>()
                .clipboard_data()
                .unwrap()
                .get_data("text/plain")
                .unwrap(),
            "foo\nfoo"
        );
        assert_eq!(actions.source(), "\r\n");
        editor_key(&textarea, "z", true, false);
        assert_eq!(actions.source(), "foo\r\nfoo");
        assert_eq!(actions.selections(&actions.source()).len(), 2);
        let cut = web_sys::ClipboardEvent::new("cut").unwrap();
        textarea.dispatch_event(&cut).unwrap();
        assert_eq!(
            actions.source(),
            "foo\r\nfoo",
            "missing clipboard access must not cut the primary range"
        );
        settle().await;
        assert!(
            mounted
                .element(".editor-error[role='alert']")
                .text_content()
                .unwrap()
                .contains("Could not write the clipboard")
        );
        actions.begin_composition();
        assert!(actions.cut(Selection { anchor: 0, head: 3 }).is_err());
        assert_eq!(actions.source(), "foo\r\nfoo");
        actions.cancel_composition();
    }
}

#[wasm_bindgen_test]
async fn composition_scope_changes_and_failed_frames_preserve_documents_in_both_modes() {
    use openwebide_core::editor::{EditError, Selection};
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [
        openwebide_core::WorkspaceMode::Local,
        openwebide_core::WorkspaceMode::Remote,
    ] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("ime.rs".into()));
            state.workspace.content.set("before".into());
            editor_view(state)
        });
        let actions = EditorActions::new(mounted.state.workspace);
        actions.record_selection(Selection::caret(0)).unwrap();
        actions.begin_composition();
        actions.begin_composition();
        actions
            .native_input(
                "文before".into(),
                Selection::caret(3),
                "insertCompositionText",
                1.0,
            )
            .unwrap();
        assert!(
            actions
                .native_input(
                    "文before".into(),
                    Selection::caret(1),
                    "insertCompositionText",
                    2.0
                )
                .is_err()
        );
        assert_eq!(actions.source(), "before");
        assert!(!actions.is_composing());
        assert!(!mounted.state.workspace.dirty.get_untracked());
        actions.begin_composition();
        actions
            .native_input(
                "文before".into(),
                Selection::caret(3),
                "insertCompositionText",
                3.0,
            )
            .unwrap();
        mounted.state.workspace.save_active(1);
        mounted.state.workspace.active_project.set(Some(2));
        mounted
            .state
            .workspace
            .open_file
            .set(Some("other.rs".into()));
        mounted.state.workspace.content.set("different".into());
        settle().await;
        assert_eq!(actions.source(), "different");
        assert_eq!(
            mounted.state.workspace.snapshots.get_untracked()[&1].content,
            "before"
        );
        assert!(actions.end_composition().unwrap().is_none());
        assert_eq!(
            actions.native_input(
                "文before".into(),
                Selection::caret(3),
                "insertCompositionText",
                4.0
            ),
            Err(EditError::UnsupportedNativeInput)
        );
        assert_eq!(actions.source(), "different");
        mounted.state.workspace.active_project.set(Some(1));
        mounted.state.workspace.open_file.set(Some("ime.rs".into()));
        mounted.state.workspace.content.set("before".into());
        settle().await;
        actions.record_selection(Selection::caret(0)).unwrap();
        actions.begin_composition();
        actions
            .native_input(
                "文before".into(),
                Selection::caret(3),
                "insertCompositionText",
                5.0,
            )
            .unwrap();
        mounted
            .state
            .workspace
            .content
            .set("external change".into());
        settle().await;
        assert!(!actions.is_composing());
        assert_eq!(actions.source(), "external change");
        assert!(actions.end_composition().unwrap().is_none());
        actions.record_selection(Selection::caret(0)).unwrap();
        actions.begin_composition();
        actions
            .native_input(
                "文external change".into(),
                Selection::caret(3),
                "insertCompositionText",
                6.0,
            )
            .unwrap();
        mounted.state.workspace.reset();
        mounted.state.workspace.active_project.set(Some(1));
        mounted.state.workspace.open_file.set(Some("ime.rs".into()));
        mounted.state.workspace.content.set("new account".into());
        settle().await;
        assert!(actions.end_composition().unwrap().is_none());
        assert_eq!(
            actions.native_input(
                "文external change".into(),
                Selection::caret(3),
                "insertCompositionText",
                7.0
            ),
            Err(EditError::UnsupportedNativeInput)
        );
        assert_eq!(actions.source(), "new account");
        assert_eq!(
            actions.native_input(
                "文external change".into(),
                Selection::caret(3),
                "insertFromComposition",
                8.0
            ),
            Err(EditError::UnsupportedNativeInput)
        );
        actions
            .native_input(
                "new account".into(),
                Selection::caret(0),
                "insertFromComposition",
                9.0,
            )
            .unwrap();
        assert_eq!(actions.source(), "new account");
    }
}

#[wasm_bindgen_test]
async fn multi_native_input_and_composition_preserve_secondary_selections_in_both_modes() {
    use openwebide_core::editor::{Selection, SelectionCommand};
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [
        openwebide_core::WorkspaceMode::Local,
        openwebide_core::WorkspaceMode::Remote,
    ] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("multi.rs".into()));
            state.workspace.content.set("foo\r\nfoo".into());
            editor_view(state)
        });
        settle().await;
        frame().await;
        let textarea: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        let actions = EditorActions::new(mounted.state.workspace);
        let project = mounted
            .state
            .workspace
            .active_project
            .get_untracked()
            .unwrap();
        textarea.set_selection_range(0, 3).unwrap();
        actions
            .record_selection(Selection { anchor: 0, head: 3 })
            .unwrap();
        actions
            .selection_command(
                project,
                "multi.rs",
                "foo\r\nfoo",
                SelectionCommand::AllOccurrences,
            )
            .unwrap();
        settle().await;
        frame().await;
        assert_eq!(actions.selections(&actions.source()).len(), 2);
        textarea
            .dispatch_event(&web_sys::CompositionEvent::new("compositionstart").unwrap())
            .unwrap();
        assert!(
            actions.is_composing(),
            "composition owner is shared by facade instances"
        );
        editorNativeInput(&textarea, "文", "insertCompositionText", true);
        settle().await;
        frame().await;
        assert_eq!(textarea.value(), "文\nfoo");
        assert_eq!(actions.source(), "文\r\nfoo");
        assert_eq!(actions.selections(&actions.source()).len(), 2);
        textarea.set_selection_range(0, 1).unwrap();
        editorNativeInput(&textarea, "文字", "insertCompositionText", true);
        settle().await;
        frame().await;
        assert_eq!(
            textarea.value(),
            "文字\nfoo",
            "IME owns primary-only native value"
        );
        assert_eq!(textarea.selection_start().unwrap(), Some(2));
        textarea
            .dispatch_event(&web_sys::CompositionEvent::new("compositionend").unwrap())
            .unwrap();
        settle().await;
        frame().await;
        assert_eq!(textarea.value(), "文字\n文字");
        assert_eq!(actions.source(), "文字\r\n文字");
        assert_eq!(actions.selections(&actions.source()).len(), 2);
        assert!(!actions.is_composing());
        editor_key(&textarea, "z", true, false);
        settle().await;
        frame().await;
        assert_eq!(actions.source(), "foo\r\nfoo");
        assert_eq!(actions.selections(&actions.source()).len(), 2);
        editorNativeInput(&textarea, "bar", "insertText", false);
        settle().await;
        frame().await;
        assert_eq!(textarea.value(), "bar\nbar");
        assert_eq!(actions.source(), "bar\r\nbar");
        assert_eq!(actions.selections(&actions.source()).len(), 2);
        editor_key(&textarea, "z", true, false);
        settle().await;
        assert_eq!(actions.source(), "foo\r\nfoo");
    }
}

#[wasm_bindgen_test]
async fn native_typing_composition_and_invalid_edits_preserve_document_contract() {
    use openwebide_core::editor::{Indentation, Selection};
    use openwebide_frontend::state_actions::editor::{EditorActions, EditorCommand};
    for mode in [
        openwebide_core::WorkspaceMode::Local,
        openwebide_core::WorkspaceMode::Remote,
    ] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("fixture.rs".into()));
            state.workspace.content.set("😀\r\ntext\nlast".into());
            editor_view(state)
        });
        let actions = EditorActions::new(mounted.state.workspace);
        actions.record_selection(Selection::caret(10)).unwrap();
        actions
            .native_input(
                "😀\ntexts\nlast".into(),
                Selection::caret(10),
                "insertText",
                0.0,
            )
            .unwrap();
        actions.record_selection(Selection::caret(11)).unwrap();
        actions
            .native_input(
                "😀\ntextsx\nlast".into(),
                Selection::caret(11),
                "insertText",
                100.0,
            )
            .unwrap();
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            "😀\r\ntextsx\nlast"
        );
        actions
            .command(
                EditorCommand::Undo,
                Selection::caret(12),
                Indentation::default(),
            )
            .unwrap();
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            "😀\r\ntext\nlast"
        );
        actions.begin_composition();
        actions
            .native_input(
                "😀\ntext文\nlast".into(),
                Selection::caret(12),
                "insertCompositionText",
                1000.0,
            )
            .unwrap();
        actions
            .native_input(
                "😀\ntext文字\nlast".into(),
                Selection::caret(15),
                "insertCompositionText",
                4000.0,
            )
            .unwrap();
        actions.end_composition().unwrap();
        actions
            .command(
                EditorCommand::Undo,
                Selection::caret(16),
                Indentation::default(),
            )
            .unwrap();
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            "😀\r\ntext\nlast"
        );
        assert!(
            actions
                .native_input(
                    "😀\nchanged\nlast".into(),
                    Selection::caret(1),
                    "insertText",
                    5000.0
                )
                .is_err()
        );
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            "😀\r\ntext\nlast"
        );
    }
}

#[wasm_bindgen_test]
async fn folded_document_commands_projection_and_history_share_both_modes() {
    use openwebide_core::{
        WorkspaceMode,
        editor::{FoldCommand, ProjectionError, Selection},
    };
    use openwebide_frontend::state_actions::editor::{EditorActions, EditorCommand};
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let source = "// 😀\r\nfn main() {\r\n    let value = \"文\";\r\n}\r\nnext();\r\n";
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("folds.rs".into()));
            state.workspace.content.set(source.into());
            editor_view(state)
        });
        let actions = EditorActions::new(mounted.state.workspace);
        actions
            .record_selection(Selection::caret(source.find('文').unwrap()))
            .unwrap();
        actions.refresh_fold_ranges(|| true);
        let (projection, selection) = actions.fold_command(FoldCommand::CollapseAll).unwrap();
        assert_eq!(projection.text(), "// 😀\r\nfn main() {\r\nnext();\r\n");
        assert_eq!(
            selection,
            Selection::caret(source.find(" {\r\n").unwrap() + 2)
        );
        assert_eq!(
            projection.visible_offset(source.find('文').unwrap()),
            Err(ProjectionError::HiddenText)
        );
        assert!(!mounted.state.workspace.dirty.get_untracked());
        // Independent file/project histories own their collapse state.
        mounted
            .state
            .workspace
            .open_file
            .set(Some("other.rs".into()));
        actions.refresh_fold_ranges(|| true);
        assert!(!actions.projection().unwrap().is_folded());
        mounted
            .state
            .workspace
            .open_file
            .set(Some("folds.rs".into()));
        assert!(actions.projection().unwrap().is_folded());
        actions.fold_command(FoldCommand::Reveal(2));
        assert_eq!(actions.projection().unwrap().text(), source);
        actions.fold_command(FoldCommand::CollapseAll);
        let original = mounted.state.workspace.content.get_untracked();
        actions
            .command(
                EditorCommand::Newline,
                Selection::caret(0),
                actions.rules_untracked().indentation,
            )
            .unwrap();
        assert!(
            !actions.projection().unwrap().is_folded(),
            "stale ranges stay invalid until refreshed"
        );
        actions.refresh_fold_ranges(|| true);
        assert!(
            actions.projection().unwrap().is_folded(),
            "unchanged headers follow inserted rows"
        );
        actions
            .command(
                EditorCommand::Undo,
                Selection::caret(0),
                actions.rules_untracked().indentation,
            )
            .unwrap();
        actions.refresh_fold_ranges(|| true);
        assert_eq!(mounted.state.workspace.content.get_untracked(), original);
        assert!(actions.projection().unwrap().is_folded());
        mounted.state.workspace.active_project.set(Some(2));
        actions.refresh_fold_ranges(|| true);
        assert!(!actions.projection().unwrap().is_folded());
        mounted.state.workspace.active_project.set(Some(1));
        assert!(actions.projection().unwrap().is_folded());
        let before = mounted
            .state
            .workspace
            .editor_documents
            .with_untracked(|documents| documents.get(&(1, "folds.rs".into())).unwrap().clone());
        assert!(
            actions
                .refresh_fold_ranges(|| {
                    mounted.state.workspace.active_project.set(Some(2));
                    true
                })
                .is_none()
        );
        let after = mounted
            .state
            .workspace
            .editor_documents
            .with_untracked(|documents| documents.get(&(1, "folds.rs".into())).unwrap().clone());
        assert_eq!(
            before, after,
            "a changed scope cannot publish ranges or another buffer's content"
        );
        mounted.state.workspace.reset();
        assert!(actions.projection().is_none());
    }
}

#[wasm_bindgen_test]
async fn built_in_language_parser_folds_share_local_remote_and_wasm_contracts() {
    use openwebide_core::{
        WorkspaceMode,
        editor::{SyntaxDocument, SyntaxStatus, syntax_contracts::LANGUAGE_CASES},
        highlight::language_from_path,
    };
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            editor_view(state)
        });
        let actions = EditorActions::new(mounted.state.workspace);
        let oversized = "x".repeat(openwebide_core::editor::MAX_STRUCTURE_BYTES + 1);
        for &(path, source, expected) in LANGUAGE_CASES {
            mounted.state.workspace.open_file.set(Some(path.into()));
            mounted.state.workspace.content.set(source.into());
            let (status, folds) = actions.syntax_folds(|| true).unwrap();
            assert_eq!(status, SyntaxStatus::Ready { incremental: false }, "{path}");
            assert!(folds.contains(&expected), "{mode:?} {path}: {folds:?}");
            let revised = source.replace("文😀", "😀文 changed").replace('\n', "\r\n");
            mounted.state.workspace.content.set(revised.clone());
            let (status, folds) = actions.syntax_folds(|| true).unwrap();
            assert_eq!(status, SyntaxStatus::Ready { incremental: true }, "{path}");
            let mut fresh = SyntaxDocument::new(language_from_path(path)).unwrap();
            fresh.update(&revised, || true);
            assert_eq!(folds, fresh.folds(), "{path}");
            assert_eq!(
                actions.syntax_folds(|| false).unwrap(),
                (SyntaxStatus::Cancelled, vec![])
            );
            mounted.state.workspace.content.set(oversized.clone());
            assert_eq!(
                actions.syntax_folds(|| true).unwrap(),
                (SyntaxStatus::TooLarge, vec![])
            );
            mounted.state.workspace.content.set(source.into());
            assert_eq!(
                actions.syntax_folds(|| true).unwrap().0,
                SyntaxStatus::Ready { incremental: false }
            );
        }
        mounted.state.workspace.reset();
        assert!(
            mounted
                .state
                .workspace
                .editor_syntax
                .with_untracked(|cache| cache.is_empty())
        );
    }
}

#[wasm_bindgen_test]
async fn parsed_typing_contexts_share_modes_mixed_cursors_and_stale_guards() {
    use openwebide_core::{
        WorkspaceMode,
        editor::{Indentation, Selection},
    };
    use openwebide_frontend::state_actions::editor::{EditorActions, EditorCommand};
    let source = "<script>const s = `text ${call()} tail`;</script><style>a { color: ; }</style>";
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            editor_view(state)
        });
        mounted
            .state
            .workspace
            .open_file
            .set(Some("context.html".into()));
        mounted.state.workspace.content.set(source.into());
        let actions = EditorActions::new(mounted.state.workspace);
        let primary = Selection::caret(source.find("call()").unwrap() + "call(".len());
        mounted
            .state
            .workspace
            .editor_documents
            .update(|documents| {
                let mut document = openwebide_core::editor::Document::new(source);
                document
                    .set_selections(vec![primary, Selection::caret(source.find("; }").unwrap())])
                    .unwrap();
                documents.insert((1, "context.html".into()), document);
            });
        actions
            .command(
                EditorCommand::TypeCharacter('('),
                primary,
                Indentation::default(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            "<script>const s = `text ${call(())} tail`;</script><style>a { color: (); }</style>"
        );
        let selections = actions.selections(&mounted.state.workspace.content.get_untracked());
        actions
            .command(
                EditorCommand::DeletePair,
                selections[0],
                Indentation::default(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(mounted.state.workspace.content.get_untracked(), source);
        let before = mounted.state.workspace.content.get_untracked();
        assert!(actions.syntax_structure(|| false).is_none());
        assert_eq!(mounted.state.workspace.content.get_untracked(), before);
        let context = actions.syntax_structure(|| true).unwrap();
        assert!(context.is_code(source.find("call").unwrap()));
        assert!(!context.is_code(source.find(" tail").unwrap()));
        let blocks = "<script>\nfunction run() {}\n</script>\n<style>\na {}\n</style>";
        mounted
            .state
            .workspace
            .open_file
            .set(Some("blocks.html".into()));
        mounted.state.workspace.content.set(blocks.into());
        let primary = Selection::caret(blocks.find("{}").unwrap() + 1);
        mounted
            .state
            .workspace
            .editor_documents
            .update(|documents| {
                let mut document = openwebide_core::editor::Document::new(blocks);
                document
                    .set_selections(vec![
                        primary,
                        Selection::caret(blocks.rfind("{}").unwrap() + 1),
                    ])
                    .unwrap();
                documents.insert((1, "blocks.html".into()), document);
            });
        actions
            .command(EditorCommand::Newline, primary, Indentation::default())
            .unwrap()
            .unwrap();
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            "<script>\nfunction run() {\n    \n}\n</script>\n<style>\na {\n    \n}\n</style>"
        );
        mounted
            .state
            .workspace
            .open_file
            .set(Some("other.txt".into()));
        mounted.state.workspace.content.set("unrelated".into());
        let other = actions.syntax_structure(|| true);
        assert!(other.is_none());
        mounted.state.workspace.reset();
        assert!(actions.syntax_structure(|| true).is_none());
    }
}

#[wasm_bindgen_test]
async fn html_embedded_folds_share_workspace_modes_and_discard_stale_bodies() {
    use openwebide_core::{
        WorkspaceMode,
        editor::{FoldRange, SyntaxDocument, SyntaxStatus},
        highlight::Language,
    };
    use openwebide_frontend::state_actions::editor::EditorActions;
    let original = "<p>文😀</p>\n<script type=module>\nfunction run() {\n  return '}';\n}\n</script>\n<style>\na {\n  color: red;\n}\n</style>";
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            editor_view(state)
        });
        let actions = EditorActions::new(mounted.state.workspace);
        mounted
            .state
            .workspace
            .open_file
            .set(Some("embedded.html".into()));
        mounted.state.workspace.content.set(original.into());
        let (status, folds) = actions.syntax_folds(|| true).unwrap();
        assert_eq!(status, SyntaxStatus::Ready { incremental: false });
        assert!(folds.contains(&FoldRange {
            start_line: 2,
            end_line: 4
        }));
        assert!(folds.contains(&FoldRange {
            start_line: 7,
            end_line: 9
        }));
        for revised in [
            original
                .replace("文😀", "😀 changed 文")
                .replace('\n', "\r\n"),
            original.replace("type=module", "type=application/json"),
            format!("<script>function unfinished() {{</script>\n{original}"),
        ] {
            mounted.state.workspace.content.set(revised.clone());
            let (status, folds) = actions.syntax_folds(|| true).unwrap();
            assert_eq!(status, SyntaxStatus::Ready { incremental: true });
            let mut fresh = SyntaxDocument::new(Language::Html).unwrap();
            fresh.update(&revised, || true);
            assert_eq!(folds, fresh.folds(), "{mode:?}");
        }
        assert_eq!(
            actions.syntax_folds(|| false).unwrap(),
            (SyntaxStatus::Cancelled, vec![])
        );
        mounted
            .state
            .workspace
            .content
            .set("<script></script>\n".repeat(65));
        assert_eq!(
            actions.syntax_folds(|| true).unwrap(),
            (SyntaxStatus::TooLarge, vec![])
        );
        mounted.state.workspace.content.set(original.into());
        assert_eq!(
            actions.syntax_folds(|| true).unwrap().0,
            SyntaxStatus::Ready { incremental: false }
        );
        mounted.state.workspace.reset();
        assert!(
            mounted
                .state
                .workspace
                .editor_syntax
                .with_untracked(|cache| cache.is_empty())
        );
        assert!(actions.syntax_folds(|| true).is_none());
    }
}

#[wasm_bindgen_test]
async fn incremental_syntax_and_fold_provider_share_both_modes_and_account_reset() {
    use openwebide_core::{
        WorkspaceMode,
        editor::{FoldRange, SyntaxDocument, SyntaxStatus},
    };
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let source = "// 😀\r\nfn main() {\r\n    let text = r###\" { } \"###;\r\n    /* outer\r\n       /* nested */\r\n    */\r\n}\r\n";
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("syntax.rs".into()));
            state.workspace.content.set(source.into());
            editor_view(state)
        });
        let actions = EditorActions::new(mounted.state.workspace);
        let (status, folds) = actions.syntax_folds(|| true).unwrap();
        assert_eq!(status, SyntaxStatus::Ready { incremental: false });
        assert_eq!(
            folds,
            vec![
                FoldRange {
                    start_line: 1,
                    end_line: 6
                },
                FoldRange {
                    start_line: 3,
                    end_line: 5
                }
            ]
        );
        let revised = source
            .replace("    let text", "    if true {\r\n        let text")
            .replace("    /* outer", "    }\r\n    /* outer");
        mounted.state.workspace.content.set(revised.clone());
        let (status, folds) = actions.syntax_folds(|| true).unwrap();
        assert_eq!(status, SyntaxStatus::Ready { incremental: true });
        let mut fresh = SyntaxDocument::new(openwebide_core::highlight::Language::Rust).unwrap();
        fresh.update(&revised, || true);
        assert_eq!(folds, fresh.folds());
        mounted.state.workspace.active_project.set(Some(2));
        assert_eq!(
            actions.syntax_folds(|| true).unwrap().0,
            SyntaxStatus::Ready { incremental: false }
        );
        mounted.state.workspace.active_project.set(Some(1));
        assert_eq!(
            actions.syntax_folds(|| true).unwrap().0,
            SyntaxStatus::Ready { incremental: true }
        );
        assert_eq!(
            actions.syntax_folds(|| false).unwrap(),
            (SyntaxStatus::Cancelled, vec![])
        );
        assert_eq!(
            actions.syntax_folds(|| true).unwrap().0,
            SyntaxStatus::Ready { incremental: false }
        );
        mounted
            .state
            .workspace
            .content
            .set("x".repeat(openwebide_core::editor::MAX_STRUCTURE_BYTES + 1));
        assert_eq!(
            actions.syntax_folds(|| true).unwrap(),
            (SyntaxStatus::TooLarge, vec![])
        );
        mounted
            .state
            .workspace
            .open_file
            .set(Some("syntax.py".into()));
        assert_eq!(
            actions.syntax_folds(|| true).unwrap(),
            (SyntaxStatus::TooLarge, vec![])
        );
        mounted.state.workspace.reset();
        assert_eq!(
            mounted
                .state
                .workspace
                .editor_syntax
                .with_untracked(|documents| documents.len()),
            0
        );
    }
}

#[wasm_bindgen_test]
async fn caret_and_scroll_restore_across_files_projects_and_views_in_both_modes() {
    use openwebide_core::WorkspaceMode;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let source = format!("😀{}\r\n", "x".repeat(300)).repeat(80);
        let content = source.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("position.rs".into()));
            state.workspace.content.set(content);
            view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:500px;height:250px">{editor_view(state)}</div> }
        });
        settle().await;
        frame().await;
        let first: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        first
            .set_selection_range_with_direction(2, 320, "backward")
            .unwrap();
        first
            .dispatch_event(&web_sys::Event::new("select").unwrap())
            .unwrap();
        openwebide_frontend::viewport::set_editor_scroll_top(&first, 450.0);
        openwebide_frontend::viewport::set_editor_scroll_left(&first, 120.0);
        first
            .dispatch_event(&web_sys::Event::new("scroll").unwrap())
            .unwrap();
        let expected = (
            openwebide_frontend::viewport::editor_scroll(&first).scroll_top(),
            openwebide_frontend::viewport::editor_scroll(&first).scroll_left(),
        );
        assert!(expected.0 > 0.0 && expected.1 > 0.0);
        let actions =
            openwebide_frontend::state_actions::editor::EditorActions::new(mounted.state.workspace);
        actions.record_scroll(1, "position.rs", f64::NAN, 50.0);
        actions.record_scroll(2, "position.rs", 900.0, 50.0);
        actions.record_scroll(1, "missing.rs", 900.0, 50.0);
        assert_eq!((actions.scroll().top, actions.scroll().left), expected);
        // Identical text must still restore the other file's distinct position.
        mounted
            .state
            .workspace
            .open_file
            .set(Some("other.rs".into()));
        settle().await;
        let other: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        assert_eq!(other.selection_start().unwrap(), Some(0));
        assert!(
            openwebide_frontend::viewport::editor_scroll(&other)
                .scroll_top()
                .abs()
                < 0.5
        );
        other.set_selection_range(5, 5).unwrap();
        other
            .dispatch_event(&web_sys::Event::new("select").unwrap())
            .unwrap();
        // Late measurements from a detached file cannot overwrite the active file.
        openwebide_frontend::viewport::set_editor_scroll_top(&first, 900.0);
        first
            .dispatch_event(&web_sys::Event::new("scroll").unwrap())
            .unwrap();
        mounted
            .state
            .workspace
            .open_file
            .set(Some("position.rs".into()));
        settle().await;
        frame().await;
        let restored: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        assert_eq!(restored.selection_start().unwrap(), Some(2));
        assert_eq!(restored.selection_end().unwrap(), Some(320));
        assert_eq!(
            restored.selection_direction().unwrap().as_deref(),
            Some("backward")
        );
        assert_eq!(
            (
                openwebide_frontend::viewport::editor_scroll(&restored).scroll_top(),
                openwebide_frontend::viewport::editor_scroll(&restored).scroll_left()
            ),
            expected
        );
        mounted.state.workspace.switch_project(Some(1), 2);
        mounted
            .state
            .workspace
            .open_file
            .set(Some("position.rs".into()));
        mounted.state.workspace.content.set(source);
        settle().await;
        let second: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        assert_eq!(second.selection_start().unwrap(), Some(0));
        assert!(
            openwebide_frontend::viewport::editor_scroll(&second)
                .scroll_top()
                .abs()
                < 0.5
        );
        mounted.state.workspace.switch_project(Some(2), 1);
        settle().await;
        frame().await;
        let restored: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        assert_eq!(restored.selection_end().unwrap(), Some(320));
        assert_eq!(
            (
                openwebide_frontend::viewport::editor_scroll(&restored).scroll_top(),
                openwebide_frontend::viewport::editor_scroll(&restored).scroll_left()
            ),
            expected
        );
        // Remount the edit view, retaining selection even with no text change.
        mounted
            .state
            .git
            .head_content
            .set(Some(openwebide_frontend::state::git::HeadContent {
                project_id: Some(1),
                path: "position.rs".into(),
                content: Ok("old".into()),
            }));
        settle().await;
        mounted.click_text("Diff HEAD");
        settle().await;
        assert!(
            mounted
                .root
                .query_selector(".editor-textarea")
                .unwrap()
                .is_none()
        );
        // Detached nodes for the same file must not replace its saved selection.
        restored.set_selection_range(0, 0).unwrap();
        restored
            .dispatch_event(&web_sys::Event::new("select").unwrap())
            .unwrap();
        restored
            .dispatch_event(&web_sys::Event::new("scroll").unwrap())
            .unwrap();
        mounted.click_text("Edit");
        settle().await;
        frame().await;
        let restored: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        assert_eq!(restored.selection_end().unwrap(), Some(320));
        assert_eq!(
            (
                openwebide_frontend::viewport::editor_scroll(&restored).scroll_top(),
                openwebide_frontend::viewport::editor_scroll(&restored).scroll_left()
            ),
            expected
        );
        mounted.state.workspace.reset();
        assert!(
            mounted
                .state
                .workspace
                .editor_scroll
                .get_untracked()
                .is_empty()
        );
        assert!(
            mounted
                .state
                .workspace
                .editor_documents
                .get_untracked()
                .is_empty()
        );
    }
}

#[wasm_bindgen_test]
async fn rust_editor_commands_history_and_stale_dom_share_both_modes() {
    for mode in [
        openwebide_core::WorkspaceMode::Local,
        openwebide_core::WorkspaceMode::Remote,
    ] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("fixture.rs".into()));
            state.workspace.content.set("a\r\nb\r\nc".into());
            editor_view(state)
        });
        settle().await;
        let textarea: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        // The DOM normalizes textarea values to LF. The command must retain the
        // workspace's CRLF text rather than rewrite unrelated line endings.
        textarea.set_selection_range(0, 4).unwrap();
        assert!(editor_key(&textarea, "Tab", false, false).default_prevented());
        settle().await;
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            "    a\r\n    b\r\nc"
        );
        assert!(editor_key(&textarea, "z", true, false).default_prevented());
        settle().await;
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            "a\r\nb\r\nc"
        );
        assert!(!mounted.state.workspace.dirty.get_untracked());
        editor_key(&textarea, "z", true, true);
        settle().await;
        assert!(mounted.state.workspace.dirty.get_untracked());
        editor_key(&textarea, "m", true, false);
        assert!(!editor_key(&textarea, "Tab", false, false).default_prevented());
        mounted
            .state
            .workspace
            .open_file
            .set(Some("other.rs".into()));
        mounted.state.workspace.content.set("other".into());
        mounted.state.workspace.dirty.set(false);
        settle().await;
        editor_key(&textarea, "Tab", false, false);
        assert_eq!(mounted.state.workspace.content.get_untracked(), "other");
        mounted
            .state
            .workspace
            .open_file
            .set(Some("fixture.rs".into()));
        mounted
            .state
            .workspace
            .content
            .set("    a\r\n    b\r\nc".into());
        settle().await;
        let restored: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        editor_key(&restored, "z", true, false);
        settle().await;
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            "a\r\nb\r\nc"
        );
        mounted.state.workspace.reset();
        assert!(
            mounted
                .state
                .workspace
                .editor_documents
                .get_untracked()
                .is_empty()
        );
    }
}

#[wasm_bindgen_test]
async fn markdown_preview_gutters_share_git_and_pending_changes_in_both_modes() {
    use openwebide_core::{FileDiff, WorkspaceMode};
    use openwebide_frontend::state::git::HeadContent;
    let old = "# Keep\n\nOld text\n\nEnd\n\nRemove me\n";
    let new = "# Keep\n\nNew text\n\nAdded\n\nEnd\n";
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("README.md".into()));
            state.workspace.content.set(new.into());
            state.git.head_content.set(Some(HeadContent {
                project_id: Some(1),
                path: "README.md".into(),
                content: Ok(old.into()),
            }));
            view! { <style>{include_str!("../../styles.css")}</style>{editor_view(state)} }
        });
        settle().await;
        mounted.click_text("Preview");
        settle().await;
        assert!(
            mounted
                .root
                .query_selector(".rich-preview-block.modified")
                .unwrap()
                .is_some()
        );
        assert!(
            mounted
                .root
                .query_selector(".rich-preview-block.added")
                .unwrap()
                .is_some()
        );
        assert!(
            mounted
                .root
                .query_selector(".rich-preview-block.removed")
                .unwrap()
                .is_some()
        );
        assert!(
            mounted
                .element(".rich-preview")
                .text_content()
                .unwrap()
                .contains("Remove me")
        );
        mounted.state.workspace.merge_pending(
            1,
            FileDiff {
                path: "README.md".into(),
                old: Some(old.into()),
                new: new.into(),
                old_unavailable: false,
                backup_path: None,
            },
        );
        settle().await;
        mounted.click_text("Preview");
        settle().await;
        assert!(
            mounted
                .root
                .query_selector(".rich-preview-block.modified")
                .unwrap()
                .is_some()
        );
        mounted
            .state
            .workspace
            .pending_edits
            .update(|diff| diff.get_mut("README.md").unwrap().old_unavailable = true);
        settle().await;
        assert!(
            mounted
                .root
                .query_selector(".rich-preview-block.modified")
                .unwrap()
                .is_none()
        );
        mounted
            .state
            .workspace
            .pending_edits
            .set(Default::default());
        mounted
            .state
            .git
            .head_content
            .update(|head| head.as_mut().unwrap().project_id = Some(2));
        settle().await;
        assert!(
            mounted
                .root
                .query_selector(".rich-preview-block.modified")
                .unwrap()
                .is_none(),
            "Stale HEAD must not mark another project"
        );
        let changelog =
            "## Changed\n\n- Keep this item.\n- Make previews **clear**.\n- Keep this too.\n";
        let revised = changelog.replace("clear", "compact");
        mounted.state.workspace.content.set(revised.clone());
        mounted.state.git.head_content.set(Some(HeadContent {
            project_id: Some(1),
            path: "README.md".into(),
            content: Ok(changelog.into()),
        }));
        for pending in [false, true] {
            if pending {
                mounted.state.workspace.merge_pending(
                    1,
                    FileDiff {
                        path: "README.md".into(),
                        old: Some(changelog.into()),
                        new: revised.clone(),
                        old_unavailable: false,
                        backup_path: None,
                    },
                );
                settle().await;
                mounted.click_text("Preview");
            }
            settle().await;
            assert_eq!(
                mounted
                    .root
                    .query_selector_all(".rich-preview h2")
                    .unwrap()
                    .length(),
                1
            );
            assert_eq!(
                mounted
                    .root
                    .query_selector_all(".rich-preview li")
                    .unwrap()
                    .length(),
                3
            );
            assert_eq!(
                mounted
                    .element(".rich-preview del")
                    .text_content()
                    .as_deref(),
                Some("clear")
            );
            assert_eq!(
                mounted
                    .element(".rich-preview ins")
                    .text_content()
                    .as_deref(),
                Some("compact")
            );
            assert_eq!(
                mounted
                    .root
                    .query_selector_all(".rich-preview li.rich-preview-item.modified")
                    .unwrap()
                    .length(),
                1
            );
            assert!(
                mounted
                    .root
                    .query_selector(".rich-preview-block.modified")
                    .unwrap()
                    .is_none(),
                "The unchanged list container must not have a gutter"
            );
            assert!(
                !mounted
                    .element(".rich-preview li:first-child")
                    .has_attribute("class")
            );
            assert!(
                !mounted
                    .element(".rich-preview li:last-child")
                    .has_attribute("class")
            );
            let item = mounted.element(".rich-preview-item.modified");
            let marker = web_sys::window()
                .unwrap()
                .get_computed_style_with_pseudo_elt(&item, "::before")
                .unwrap()
                .unwrap();
            assert_eq!(marker.get_property_value("width").unwrap(), "3px");
            assert!(
                marker
                    .get_property_value("height")
                    .unwrap()
                    .trim_end_matches("px")
                    .parse::<f64>()
                    .unwrap()
                    <= item.get_bounding_client_rect().height() + 1.0
            );
            let style = web_sys::window()
                .unwrap()
                .get_computed_style(&mounted.element(".rich-preview del"))
                .unwrap()
                .unwrap();
            assert!(
                style
                    .get_property_value("text-decoration-line")
                    .unwrap()
                    .contains("line-through")
            );
        }
    }
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
    // Geometry assertions use the selected font, including its loading callbacks.
    editorFontsReady().await.unwrap();
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
        let initial = source.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state.workspace.open_file.set(Some("fixture.rs".into()));
            state.workspace.content.set(initial);
            view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:700px;height:400px">{editor_view(state)}</div> }
        });
        settle().await;
        frame().await;
        let actions =
            openwebide_frontend::state_actions::editor::EditorActions::new(mounted.state.workspace);
        wait_until("warm source dimension cache", || {
            !actions.syntax_is_pending() && actions.measured_rows().is_some()
        })
        .await;
        frame().await;
        let textarea: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        editor_key(&textarea, "End", true, false);
        frame().await;
        let mut probes = Vec::new();
        let mut latency = Vec::new();
        let mut frames = Vec::new();
        let mut counts = Vec::new();
        for run in 0..5 {
            let before = openwebide_frontend::components::viewport_highlight_count();
            let all_before = highlight_count();
            let start = now();
            for event in 0..10 {
                let text = format!("// burst {run} input {event}\n");
                let start = now();
                let native = textarea.value();
                let tail = native.find("// burst ").unwrap_or(native.len());
                textarea
                    .set_selection_range(
                        native[..tail].encode_utf16().count().try_into().unwrap(),
                        native.encode_utf16().count().try_into().unwrap(),
                    )
                    .unwrap();
                let init = web_sys::InputEventInit::new();
                init.set_bubbles(true);
                init.set_cancelable(true);
                init.set_input_type("insertReplacementText");
                init.set_data(Some(&text));
                let before =
                    web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();
                textarea.dispatch_event(&before).unwrap();
                assert!(!before.default_prevented());
                editorNativeInput(&textarea, &text, "insertReplacementText", false);
                settle().await;
                latency.push(now() - start);
                assert_eq!(actions.source(), format!("{source}{text}"));
            }
            wait_until("coalesced current syntax paint", || {
                !actions.syntax_is_pending()
                    && openwebide_frontend::components::viewport_highlight_count() > before
            })
            .await;
            frame().await;
            frames.push(now() - start);
            let visible = openwebide_frontend::components::viewport_highlight_count() - before;
            counts.push(visible);
            probes.push(highlight_count() - all_before - visible);
        }
        assert!(
            counts.iter().all(|count| *count == 1),
            "visible generations={counts:?}"
        );
        assert!(
            probes.iter().all(|count| *count <= 1),
            "coalesced changed-row probes={probes:?}"
        );
        latency.sort_by(f64::total_cmp);
        frames.sort_by(f64::total_cmp);
        console_log!(
            "editor {lines} lines: visible generations/10 inputs={counts:?}, changed-row probes={probes:?}, input+microtasks median={:.3}ms p95={:.3}ms, burst-to-paint median={:.3}ms p95={:.3}ms",
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
        mounted
            .element(".editor-source-line[data-line='1'] > .editor-source-fragment")
            .inner_html(),
        "fn plain <span class=\"tok-operator\">&lt;&gt;&amp;</span>\n"
    );
    let before = highlight_count();
    input(&mounted, "new text");
    settle().await;
    mounted
        .state
        .workspace
        .open_file
        .set(Some("README.md".into()));
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
        assert!(
            (textarea.scroll_height()
                - mounted.element(".editor-highlight-content").scroll_height())
            .abs()
                <= 2,
            "code and gutter heights must match"
        );
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

#[wasm_bindgen_test]
async fn editor_numbers_full_diff_and_find_share_both_workspace_modes() {
    use openwebide_core::{FileDiff, WorkspaceMode};
    use openwebide_frontend::state::git::HeadContent;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("fixture.rs".into()));
            state
                .workspace
                .content
                .set("start\n😀 café\nkeep\ncafé end\n".into());
            state.git.head_content.set(Some(HeadContent {
                project_id: Some(1),
                path: "fixture.rs".into(),
                content: Ok("start\nold\nkeep\nold end\n".into()),
            }));
            view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:600px;height:400px">{editor_view(state)}</div> }
        });
        settle().await;
        frame().await;
        assert_eq!(
            mounted
                .element(".editor-source-line[data-line='3']")
                .text_content()
                .unwrap(),
            "keep\n"
        );
        mounted.click("button[aria-label^='Find in file']");
        settle().await;
        let search: web_sys::HtmlInputElement =
            mounted.element(".editor-find input").unchecked_into();
        search.set_value("café");
        search
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
        settle().await;
        let textarea: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        assert_eq!(textarea.selection_start().unwrap(), Some(9));
        assert_eq!(textarea.selection_end().unwrap(), Some(13));
        assert!(
            mounted
                .element(".editor-find")
                .text_content()
                .unwrap()
                .contains("1 / 2")
        );
        mounted.click("button[aria-label='Next match']");
        settle().await;
        assert!(
            mounted
                .element(".editor-find")
                .text_content()
                .unwrap()
                .contains("2 / 2")
        );
        mounted.click_text("Diff HEAD");
        settle().await;
        assert_eq!(
            mounted
                .element(".diff-line[data-line='3'] .editor-line-text")
                .text_content()
                .unwrap(),
            "keep"
        );
        assert!(
            mounted
                .root
                .query_selector(".diff-line.add[data-line='2']")
                .unwrap()
                .is_some()
        );
        assert!(
            mounted
                .root
                .query_selector(".editor-find-match[data-line='4']")
                .unwrap()
                .is_some()
        );
        assert!(
            !mounted
                .element(".editor-header")
                .text_content()
                .unwrap()
                .contains("Content")
        );
        mounted.click_text("Split");
        settle().await;
        assert_eq!(
            mounted
                .element(".sbs-cell[data-line='3'] .editor-line-number")
                .text_content()
                .unwrap(),
            "3"
        );
        assert_eq!(
            mounted
                .element(".sbs-cell[data-line='3'] .editor-line-text")
                .text_content()
                .unwrap(),
            "keep"
        );
        mounted.click("button[aria-label='Close find']");
        settle().await;
        assert!(
            mounted
                .root
                .query_selector(".editor-find-match")
                .unwrap()
                .is_none()
        );
        let preview = mounted.element(".ui-seg-btn:last-child");
        assert!(preview.has_attribute("disabled"));
        mounted.click_text("Preview");
        settle().await;
        assert!(
            mounted
                .root
                .query_selector(".editor-line-number")
                .unwrap()
                .is_some()
        );
        mounted.click_text("Edit");
        settle().await;
        let init = web_sys::KeyboardEventInit::new();
        init.set_key("f");
        init.set_ctrl_key(true);
        init.set_bubbles(true);
        init.set_cancelable(true);
        let event =
            web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
        mounted
            .element(".editor-textarea")
            .dispatch_event(&event)
            .unwrap();
        settle().await;
        assert!(event.default_prevented());
        mounted
            .state
            .workspace
            .open_file
            .set(Some("other.rs".into()));
        mounted.state.workspace.content.set("😀 café".into());
        settle().await;
        frame().await;
        let textarea: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        assert_eq!(textarea.selection_start().unwrap(), Some(3));
        assert_eq!(textarea.selection_end().unwrap(), Some(7));
        mounted.state.workspace.merge_pending(
            1,
            FileDiff {
                path: "other.rs".into(),
                old: Some("unchanged\nold\nafter\n".into()),
                new: "unchanged\ncafé\nafter\n".into(),
                old_unavailable: false,
                backup_path: None,
            },
        );
        settle().await;
        assert_eq!(
            mounted
                .element(".diff-line[data-line='1'] .editor-line-text")
                .text_content()
                .unwrap(),
            "unchanged"
        );
        assert_eq!(
            mounted
                .element(".diff-line[data-line='3'] .editor-line-text")
                .text_content()
                .unwrap(),
            "after"
        );
        assert!(
            mounted
                .root
                .query_selector(".editor-find-match[data-line='2']")
                .unwrap()
                .is_some()
        );
    }
}

#[wasm_bindgen_test]
async fn numbered_views_scroll_horizontally_with_compact_gutters_and_linked_split_panes() {
    use openwebide_core::WorkspaceMode;
    use openwebide_frontend::state::git::HeadContent;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("wide.rs".into()));
            state.workspace.content.set(format!(
                "{}needle\n{}",
                "x".repeat(800),
                "short\n".repeat(40)
            ));
            state.git.head_content.set(Some(HeadContent {
                project_id: Some(1),
                path: "wide.rs".into(),
                content: Ok(format!("{}\n{}", "y".repeat(200), "short\n".repeat(40))),
            }));
            view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:500px;height:350px">{editor_view(state)}</div> }
        });
        settle().await;
        frame().await;
        let textarea: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        assert_eq!(textarea.get_attribute("wrap").unwrap(), "off");
        assert!(textarea.scroll_width() > textarea.client_width() * 2);
        assert!(
            (mounted
                .element(".editor-source-line[data-line='1']")
                .get_bounding_client_rect()
                .height()
                - 19.5)
                .abs()
                < 1.0
        );
        let compact_width = textarea.get_bounding_client_rect().left()
            - mounted
                .element(".editor-code")
                .get_bounding_client_rect()
                .left();
        // Reserve the 18px folding column even when this file has no folds.
        assert!(compact_width < 63.0);
        mounted.click("button[aria-label^='Find in file']");
        settle().await;
        let search: web_sys::HtmlInputElement =
            mounted.element(".editor-find input").unchecked_into();
        search.set_value("needle");
        search
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
        settle().await;
        assert_eq!(textarea.selection_start().unwrap(), Some(800));
        assert!(
            textarea.scroll_left() > 1_000.0,
            "find must reveal a match far across a line"
        );
        mounted.click("button[aria-label='Close find']");
        settle().await;
        textarea.set_scroll_left(120.0);
        textarea
            .dispatch_event(&web_sys::Event::new("scroll").unwrap())
            .unwrap();
        assert_scroll_aligned(&mounted, &textarea);
        let gutter = window()
            .get_computed_style_with_pseudo_elt(&mounted.element(".editor-source-line"), "::before")
            .unwrap()
            .unwrap();
        assert!(
            gutter
                .get_property_value("transform")
                .unwrap()
                .contains("120")
        );
        mounted.click_text("Diff HEAD");
        settle().await;
        let inline = mounted.element(".editor-diff-inline");
        assert!(inline.scroll_width() > inline.client_width() * 2);
        inline.set_scroll_left(120.0);
        let left = mounted
            .element(".editor-line-gutter")
            .get_bounding_client_rect()
            .left();
        assert!((left - inline.get_bounding_client_rect().left()).abs() < 2.0);
        mounted.click_text("Split");
        settle().await;
        let left = mounted.element(".sbs-pane:first-child");
        let right = mounted.element(".sbs-pane:last-child");
        assert_eq!(left.scroll_width(), right.scroll_width());
        right.set_scroll_left(1_900.0);
        right
            .dispatch_event(&web_sys::Event::new("scroll").unwrap())
            .unwrap();
        assert!((left.scroll_left() - right.scroll_left()).abs() < 0.5);
        assert!(
            left.scroll_left() > 1_800.0,
            "the shorter side must share the full horizontal extent"
        );
        left.set_scroll_left(100.0);
        left.set_scroll_top(160.0);
        left.dispatch_event(&web_sys::Event::new("scroll").unwrap())
            .unwrap();
        assert!((left.scroll_left() - right.scroll_left()).abs() < 0.5);
        assert!((left.scroll_top() - right.scroll_top()).abs() < 0.5);
        mounted.click("button[aria-label^='Find in file']");
        settle().await;
        super::support::wait_until("linked horizontal find", || {
            right.scroll_left() > 1_000.0 && (left.scroll_left() - right.scroll_left()).abs() < 0.5
        })
        .await;
        mounted.click("button[aria-label='Close find']");
        settle().await;
        assert!(
            (mounted
                .element(".sbs-pane:last-child .editor-line-gutter")
                .get_bounding_client_rect()
                .left()
                - right.get_bounding_client_rect().left())
            .abs()
                < 2.0
        );
        mounted.click_text("Edit");
        mounted.state.workspace.content.set("short\n".repeat(1_000));
        settle().await;
        frame().await;
        let textarea = mounted.element(".editor-textarea");
        let expanded_width = textarea.get_bounding_client_rect().left()
            - mounted
                .element(".editor-code")
                .get_bounding_client_rect()
                .left();
        assert!(expanded_width > compact_width + 10.0);
    }
}

#[wasm_bindgen_test]
async fn edit_scrollbars_stay_above_paint_and_outside_gutter_in_both_modes() {
    use openwebide_core::WorkspaceMode;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("scroll.rs".into()));
            state
                .workspace
                .content
                .set(format!("{}\n", "x".repeat(800)).repeat(100));
            view! { <style>{include_str!("../../styles.css")}</style><div class="editor-fixture" style="display:flex;width:500px;height:250px">{editor_view(state)}</div> }
        });
        settle().await;
        frame().await;
        let textarea: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        let scroll = mounted.element(".editor-scroll-surface");
        let overlay = mounted.element(".editor-highlight");
        let textarea_style = window().get_computed_style(&textarea).unwrap().unwrap();
        let overlay_style = window().get_computed_style(&overlay).unwrap().unwrap();
        let input_z: i32 = textarea_style
            .get_property_value("z-index")
            .unwrap()
            .parse()
            .unwrap();
        let paint_z: i32 = overlay_style
            .get_property_value("z-index")
            .unwrap()
            .parse()
            .unwrap();
        assert!(
            input_z > paint_z,
            "syntax paint must not cover scrollbar tracks or thumbs"
        );
        assert_ne!(
            textarea_style
                .get_property_value("scrollbar-color")
                .unwrap(),
            "auto"
        );
        let gutter_width = textarea.get_bounding_client_rect().left()
            - mounted
                .element(".editor-code")
                .get_bounding_client_rect()
                .left();
        // The folding column stays reserved alongside the three-digit gutter.
        assert!(gutter_width > 48.0 && gutter_width < 78.0);
        assert!(scroll.scroll_width() > scroll.client_width());
        assert!(scroll.scroll_height() > scroll.client_height());
        assert!(
            (scroll.get_bounding_client_rect().left() - textarea.get_bounding_client_rect().left())
                .abs()
                < 0.01
        );
        for (height, content) in [(250, None), (150, None), (150, Some("short"))] {
            mounted
                .element(".editor-fixture")
                .style()
                .set_property("height", &format!("{height}px"))
                .unwrap();
            if let Some(content) = content {
                mounted.state.workspace.content.set(content.into());
            }
            frame().await;
            super::support::wait_until("paint clips above horizontal scrollbar", || {
                (overlay.get_bounding_client_rect().height() - f64::from(scroll.client_height()))
                    .abs()
                    < 0.5
            })
            .await;
            assert!(
                (overlay.get_bounding_client_rect().bottom()
                    - (scroll.get_bounding_client_rect().top()
                        + f64::from(scroll.client_height())))
                .abs()
                    < 0.5
            );
        }
    }
}

#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
export function editorWheel(target, x, y, mode, shift, control) {
    const event = new WheelEvent('wheel', {bubbles:true,cancelable:true,deltaX:x,deltaY:y,deltaMode:mode,shiftKey:shift,ctrlKey:control});
    target.dispatchEvent(event); return event.defaultPrevented;
}
export async function editorConfigFolder() {
    const root = await navigator.storage.getDirectory();
    const name = 'editor-config-' + crypto.randomUUID();
    const handle = await root.getDirectoryHandle(name, {create:true});
    const src = await handle.getDirectoryHandle('src', {create:true});
    for (const [dir, name, text] of [
        [handle, '.editorconfig', 'root=true\n[*]\nindent_style=space\nindent_size=4\n'],
        [src, '.editorconfig', '[*.rs]\nindent_size=2\ntab_width=4\nend_of_line=lf\ninsert_final_newline=true\ntrim_trailing_whitespace=true\n'],
        [src, 'a.rs', '😀\r\n  tail  '],
    ]) { const file = await dir.getFileHandle(name, {create:true}); const writer = await file.createWritable(); await writer.write(text); await writer.close(); }
    return {root, name, handle};
}
export function editorConfigHandle(folder) { return folder.handle; }
export async function editorConfigCleanup(folder) {
    // Successful writes can precede a background reader releasing its OPFS lock.
    // Yield browser tasks during cleanup; permission and other failures stay errors.
    const deadline = performance.now() + 3000;
    for (;;) {
        try {
            await folder.root.removeEntry(folder.name, {recursive:true});
            return;
        } catch (error) {
            if (error.name !== 'NoModificationAllowedError' || performance.now() >= deadline) throw error;
            await new Promise(resolve => setTimeout(resolve, 10));
        }
    }
}
"#)]
extern "C" {
    fn editorWheel(
        target: &web_sys::HtmlTextAreaElement,
        x: f64,
        y: f64,
        mode: u32,
        shift: bool,
        control: bool,
    ) -> bool;
    #[wasm_bindgen(catch)]
    async fn editorConfigFolder() -> Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue>;
    fn editorConfigHandle(folder: &wasm_bindgen::JsValue) -> wasm_bindgen::JsValue;
    #[wasm_bindgen(catch)]
    async fn editorConfigCleanup(
        folder: &wasm_bindgen::JsValue,
    ) -> Result<(), wasm_bindgen::JsValue>;
}

#[wasm_bindgen_test]
async fn editor_scroll_surface_preserves_wheel_native_navigation_and_scope_in_both_modes() {
    use openwebide_core::editor::EditorPreferences;
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [
        openwebide_core::WorkspaceMode::Local,
        openwebide_core::WorkspaceMode::Remote,
    ] {
        for word_wrap in [false, true] {
            let source = format!("{}\n", "x".repeat(800)).repeat(100);
            let original = source.clone();
            let mounted = mount_test(move |state| {
                state.seed_project();
                state
                    .projects
                    .projects
                    .update(|projects| projects[0].mode = mode);
                state.settings.editor_preferences.set(EditorPreferences {
                    word_wrap,
                    ..Default::default()
                });
                state.workspace.open_file.set(Some("scroll.txt".into()));
                state.workspace.content.set(source);
                view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:500px;height:400px">{editor_view(state)}</div> }
            });
            wait_until("document scroll surface", || {
                mounted
                    .root
                    .query_selector(".editor-scroll-surface")
                    .unwrap()
                    .is_some_and(|scroll| {
                        scroll.client_height() > 100
                            && scroll.scroll_height() > scroll.client_height() * 3
                    })
            })
            .await;
            let input: web_sys::HtmlTextAreaElement =
                mounted.element(".editor-textarea").unchecked_into();
            let scroll = mounted.element(".editor-scroll-surface");
            let actions = EditorActions::new(mounted.state.workspace);
            assert!(editorWheel(&input, 25.0, 40.0, 0, false, false));
            frame().await;
            assert!((scroll.scroll_top() - 40.0).abs() < 1.0);
            assert!((scroll.scroll_left() - if word_wrap { 0.0 } else { 25.0 }).abs() < 1.0);
            assert!(editorWheel(&input, 0.0, 2.0, 1, false, false));
            frame().await;
            let line: f64 = window()
                .get_computed_style(&input)
                .unwrap()
                .unwrap()
                .get_property_value("line-height")
                .unwrap()
                .trim_end_matches("px")
                .parse()
                .unwrap();
            assert!((scroll.scroll_top() - 40.0 - 2.0 * line).abs() < 1.0);
            let top = scroll.scroll_top();
            let left = scroll.scroll_left();
            assert!(editorWheel(&input, 0.0, 3.0, 1, true, false));
            frame().await;
            assert!((scroll.scroll_top() - top).abs() < 1.0);
            assert!(
                (scroll.scroll_left() - if word_wrap { 0.0 } else { left + 3.0 * line }).abs()
                    < 1.0
            );
            assert!(editorWheel(&input, 0.0, 1.0, 2, false, false));
            frame().await;
            assert!((scroll.scroll_top() - top - f64::from(scroll.client_height())).abs() < 1.0);
            let before_zoom = scroll.scroll_top();
            assert!(!editorWheel(&input, 0.0, 40.0, 0, false, true));
            frame().await;
            assert!((scroll.scroll_top() - before_zoom).abs() < 0.01);
            wait_until("source-owned native window", || {
                input.has_attribute("data-editor-native-bound")
            })
            .await;
            let before_native = (scroll.scroll_top(), scroll.scroll_left());
            input.set_scroll_top(1111.0);
            input.set_scroll_left(222.0);
            input
                .dispatch_event(&web_sys::Event::new("scroll").unwrap())
                .unwrap();
            frame().await;
            assert!((scroll.scroll_top() - before_native.0).abs() < 1.0);
            assert!((scroll.scroll_left() - before_native.1).abs() < 1.0);
            assert!((actions.scroll().top - before_native.0).abs() < 1.0);
            scroll.set_scroll_top(333.0);
            scroll.set_scroll_left(44.0);
            scroll
                .dispatch_event(&web_sys::Event::new("scroll").unwrap())
                .unwrap();
            frame().await;
            assert!((scroll.scroll_top() - 333.0).abs() < 0.01);
            assert!((scroll.scroll_left() - if word_wrap { 0.0 } else { 44.0 }).abs() < 0.01);
            let saved = actions.scroll();
            assert!((saved.top - 333.0).abs() < 1.0);
            assert_eq!(mounted.state.workspace.content.get_untracked(), original);
            assert!(!mounted.state.workspace.dirty.get_untracked());
            mounted.click_text("Preview");
            settle().await;
            assert!(!scroll.is_connected());
            mounted.click_text("Edit");
            wait_until("remounted document scroll surface", || {
                mounted
                    .root
                    .query_selector(".editor-scroll-surface")
                    .unwrap()
                    .is_some_and(|scroll| (scroll.scroll_top() - saved.top).abs() < 1.0)
            })
            .await;
            scroll
                .dispatch_event(&web_sys::Event::new("scroll").unwrap())
                .unwrap();
            assert_eq!(
                actions.scroll(),
                saved,
                "detached scroll events cannot update the remounted view"
            );
            let current = mounted.element(".editor-scroll-surface");
            mounted
                .state
                .auth
                .generation
                .update(|generation| *generation += 1);
            current.set_scroll_top(555.0);
            current
                .dispatch_event(&web_sys::Event::new("scroll").unwrap())
                .unwrap();
            assert_eq!(
                actions.scroll(),
                saved,
                "old account scroll events cannot persist into the new account"
            );
        }
    }
}

#[wasm_bindgen_test]
async fn editorconfig_indentation_conversion_and_save_use_both_real_workspace_adapters() {
    use openwebide_core::WorkspaceMode;
    use openwebide_core::editor::{EditorPreferences, load_rules};
    use openwebide_frontend::workspace::Workspace;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let folder = if mode == WorkspaceMode::Local {
            Some(editorConfigFolder().await.unwrap())
        } else {
            None
        };
        let handle = folder.as_ref().map(editorConfigHandle);
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            if let Some(handle) = handle {
                state.projects.local_handles.update(|handles| {
                    handles.insert(1, handle.unchecked_into());
                });
            }
            state.fake.files.borrow_mut().extend([
                ((1,".editorconfig".into()),"root=true\n[*]\nindent_style=space\nindent_size=4\n".into()),
                ((1,"src/.editorconfig".into()),"[*.rs]\nindent_size=2\ntab_width=4\nend_of_line=lf\ninsert_final_newline=true\ntrim_trailing_whitespace=true\n".into()),
                ((1,"src/a.rs".into()),"😀\r\n  tail  ".into()),
            ]);
            state.workspace.open_file.set(Some("src/a.rs".into()));
            state.workspace.content.set("😀\r\n  tail  ".into());
            view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:640px;height:400px">{editor_view(state)}</div> }
        });
        wait_until("nested editor configuration", || {
            mounted
                .state
                .workspace
                .editor_rules
                .with_untracked(|rules| {
                    rules
                        .get(&(1, "src/a.rs".into()))
                        .is_some_and(|rules| rules.indentation.width == 2)
                })
        })
        .await;
        let ws = Workspace::for_project(mounted.state.api, mounted.state.projects, 1).unwrap();
        let textarea: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        textarea.set_selection_range(5, 5).unwrap();
        editor_key(&textarea, "Tab", false, false);
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            "😀\r\n    tail  "
        );
        textarea.set_value("    😀\n tail  ");
        textarea
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
        mounted.click(".editor-footer .ui-seg-btn:nth-child(2)");
        mounted.click_text("Convert indentation");
        settle().await;
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            "\t😀\r\n tail  "
        );
        let style = window().get_computed_style(&textarea).unwrap().unwrap();
        assert_eq!(style.get_property_value("tab-size").unwrap(), "4");
        editor_key(&textarea, "z", true, false);
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            "    😀\r\n tail  "
        );
        editor_key(&textarea, "z", true, true);
        mounted.click_text("Save");
        wait_until("configured file save", || {
            !mounted.state.workspace.dirty.get_untracked()
        })
        .await;
        assert_eq!(ws.read("src/a.rs").await.unwrap(), "\t😀\n tail\n");
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            "\t😀\n tail\n"
        );
        editor_key(&textarea, "z", true, false);
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            "\t😀\r\n tail  "
        );
        assert!(mounted.state.workspace.dirty.get_untracked());
        ws.write_bytes("src/.editorconfig", &[255]).await.unwrap();
        let (rules, warnings) = load_rules(
            &ws,
            "src/a.rs",
            "😀\n  tail",
            EditorPreferences::default(),
            || true,
        )
        .await
        .unwrap();
        assert_eq!(rules.indentation.width, 4);
        assert_eq!(warnings.len(), 1);
        assert!(
            load_rules(&ws, "src/a.rs", "", EditorPreferences::default(), || false)
                .await
                .is_err()
        );
        drop(mounted);
        if let Some(folder) = folder {
            editorConfigCleanup(&folder).await.unwrap();
        }
    }
}

#[wasm_bindgen_test]
async fn block_indentation_pairs_and_mobile_input_share_both_workspace_modes() {
    for mode in [
        openwebide_core::WorkspaceMode::Remote,
        openwebide_core::WorkspaceMode::Local,
    ] {
        let folder = if mode == openwebide_core::WorkspaceMode::Local {
            Some(editorConfigFolder().await.unwrap())
        } else {
            None
        };
        let handle = folder.as_ref().map(editorConfigHandle);
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            if let Some(handle) = handle {
                state.projects.local_handles.update(|handles| {
                    handles.insert(1, handle.unchecked_into());
                });
            }
            state.fake.files.borrow_mut().extend([
                (
                    (1, ".editorconfig".into()),
                    "root=true\n[*]\nindent_size=2\n".into(),
                ),
                ((1, "src/a.rs".into()), "fn f() {}".into()),
            ]);
            state.workspace.open_file.set(Some("src/a.rs".into()));
            state.workspace.content.set("fn f() {}".into());
            editor_view(state)
        });
        wait_until("configured block indentation", || {
            mounted
                .state
                .workspace
                .editor_rules
                .with_untracked(|rules| {
                    rules
                        .get(&(1, "src/a.rs".into()))
                        .is_some_and(|rules| rules.indentation.width == 2)
                })
        })
        .await;
        let textarea = mounted
            .element(".editor-textarea")
            .unchecked_into::<web_sys::HtmlTextAreaElement>();
        textarea.set_selection_start(Some(8)).unwrap();
        textarea.set_selection_end(Some(8)).unwrap();
        assert!(editor_key(&textarea, "Enter", false, false).default_prevented());
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            "fn f() {\n  \n}"
        );
        assert_eq!(textarea.selection_start().unwrap(), Some(11));
        assert!(editor_key(&textarea, "}", false, false).default_prevented());
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            "fn f() {\n}\n}"
        );
        editor_key(&textarea, "z", true, false);
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            "fn f() {\n  \n}"
        );
        editor_key(&textarea, "z", true, false);
        assert_eq!(mounted.state.workspace.content.get_untracked(), "fn f() {}");
        textarea.set_selection_start(Some(8)).unwrap();
        textarea.set_selection_end(Some(8)).unwrap();
        assert!(editor_key(&textarea, "[", false, false).default_prevented());
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            "fn f() {[]}"
        );
        assert!(editor_key(&textarea, "Backspace", false, false).default_prevented());
        assert_eq!(mounted.state.workspace.content.get_untracked(), "fn f() {}");
        textarea.set_selection_start(Some(0)).unwrap();
        textarea.set_selection_end(Some(0)).unwrap();
        assert!(!editor_key(&textarea, "Backspace", false, false).default_prevented());
        textarea.set_selection_start(Some(8)).unwrap();
        textarea.set_selection_end(Some(8)).unwrap();
        let init = web_sys::InputEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_input_type("insertText");
        init.set_data(Some("["));
        let input = web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();
        textarea.dispatch_event(&input).unwrap();
        assert!(input.default_prevented());
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            "fn f() {[]}"
        );
        let init = web_sys::InputEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_input_type("insertText");
        init.set_data(Some("{"));
        init.set_is_composing(true);
        let composition =
            web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();
        textarea.dispatch_event(&composition).unwrap();
        assert!(composition.default_prevented());
        textarea
            .dispatch_event(&web_sys::CompositionEvent::new("compositionstart").unwrap())
            .unwrap();
        let composition =
            web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();
        textarea.dispatch_event(&composition).unwrap();
        assert!(!composition.default_prevented());
        mounted.state.workspace.content.set("foobar".into());
        settle().await;
        let textarea = mounted
            .element(".editor-textarea")
            .unchecked_into::<web_sys::HtmlTextAreaElement>();
        textarea
            .set_selection_range_with_direction(3, 6, "backward")
            .unwrap();
        assert!(editor_key(&textarea, "'", false, false).default_prevented());
        assert_eq!(mounted.state.workspace.content.get_untracked(), "foo'bar'");
        assert_eq!(
            textarea.selection_direction().unwrap().as_deref(),
            Some("backward")
        );
        editor_key(&textarea, "z", true, false);
        assert_eq!(mounted.state.workspace.content.get_untracked(), "foobar");
        drop(mounted);
        if let Some(folder) = folder {
            editorConfigCleanup(&folder).await.unwrap();
        }
    }
}

#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
export function editorNativeInput(target, text, type, composing) {
    target.setRangeText(text, target.selectionStart, target.selectionEnd, 'end');
    target.dispatchEvent(new InputEvent('input', {bubbles:true, inputType:type, data:text, isComposing:composing}));
}
export function editorClipboardCut(target) {
    const event = new ClipboardEvent('cut', {bubbles:true, cancelable:true, clipboardData:new DataTransfer()});
    target.dispatchEvent(event); return event;
}
export function editorClipboardCopy(target) {
    const data = new DataTransfer();
    const event = new ClipboardEvent('copy', {bubbles:true, cancelable:true, clipboardData:data});
    target.dispatchEvent(event); return data.getData('text/plain');
}
export function editorClipboardPaste(target, text) {
    const data = new DataTransfer(); data.setData('text/plain', text);
    const event = new ClipboardEvent('paste', {bubbles:true, cancelable:true, clipboardData:data});
    target.dispatchEvent(event); return event;
}
export function editorClipboardPasteData(target, copiedEvent) {
    const event = new ClipboardEvent('paste', {bubbles:true, cancelable:true, clipboardData:copiedEvent.clipboardData});
    target.dispatchEvent(event); return event;
}
function editorGestureRect(target, line, column) {
    const row = target.parentElement.querySelector(`.editor-source-line[data-line='${line}']`);
    const walker = document.createTreeWalker(row, NodeFilter.SHOW_TEXT);
    let node, offset = column;
    while ((node = walker.nextNode())) { if (offset <= node.length) break; offset -= node.length; }
    if (!node) throw new Error('Missing gesture text position');
    const range = document.createRange(); range.setStart(node, offset); range.collapse(true);
    return range.getBoundingClientRect();
}
export function editorWatchPresentation(target) {
    const state = {lost:0};
    const observer = new MutationObserver(records => {
        for (const record of records) {
            if (!(record.oldValue || '').split(/\s+/).includes('highlight-ready')) state.lost++;
        }
        if (!target.classList.contains('highlight-ready')) state.lost++;
    });
    observer.observe(target, {attributes:true, attributeFilter:['class'], attributeOldValue:true});
    state.stop = () => observer.disconnect();
    return state;
}
export function editorPresentationLosses(state) { return state.lost; }
export function editorStopPresentationWatch(state) { state.stop(); }
export function editorEdgeGesture(target, horizontal, end, type) {
    const bounds = target.getBoundingClientRect();
    const x = horizontal ? (end ? bounds.right + 30 : bounds.left - 30) : bounds.left + 70;
    const y = horizontal ? bounds.top + 10 : (end ? bounds.bottom + 30 : bounds.top - 30);
    const event = new MouseEvent(type, {bubbles:true, cancelable:true, button:0,
        buttons:type === 'mouseup' ? 0 : 1, clientX:x, clientY:y});
    target.dispatchEvent(event); return event;
}
export function editorClickContainerFallback(target, line, column, beyond) {
    const original = Object.getOwnPropertyDescriptor(document, 'caretPositionFromPoint');
    Object.defineProperty(document, 'caretPositionFromPoint', {configurable:true, value:() => ({offsetNode:target.parentElement.querySelector(`.editor-source-line[data-line='${line}']`), offset:0})});
    try { return editorClickPosition(target, line, column, beyond); }
    finally {
        if (original) Object.defineProperty(document, 'caretPositionFromPoint', original);
        else delete document.caretPositionFromPoint;
    }
}
export function editorClickPosition(target, line, column, beyond) {
    const rect = editorGestureRect(target, line, column);
    const x = rect.left + (beyond ? 60 : .25), y = rect.top + rect.height / 2;
    let prevented;
    for (const type of ['mousedown', 'mouseup', 'click']) {
        const event = new MouseEvent(type, {bubbles:true,cancelable:true,detail:1,button:0,buttons:type === 'mousedown' ? 1 : 0,clientX:x,clientY:y});
        target.dispatchEvent(event);
        if (type === 'mousedown') prevented = event.defaultPrevented;
    }
    return prevented;
}
export function editorPrimaryGesture(target, line, column, clicks, shift, type) {
    const rect = editorGestureRect(target, line, column), viewport = target.getBoundingClientRect();
    // A partially clipped final row still has a visible glyph to click. Its full
    // rectangle midpoint can sit below the pane, depending on platform fonts.
    const top = Math.max(rect.top, viewport.top), bottom = Math.min(rect.bottom, viewport.bottom);
    if (bottom <= top) throw new Error('Gesture glyph is outside the editor viewport');
    const event = new MouseEvent(type, {bubbles:true, cancelable:true, detail:clicks, shiftKey:shift, button:0, buttons:type === 'mouseup' ? 0 : 1, clientX:rect.left + .25, clientY:(top + bottom) / 2});
    target.dispatchEvent(event); return event;
}
export function editorGesture(target, line, column, shift, moving) {
    const rect = editorGestureRect(target, line, column);
    const event = new MouseEvent(moving ? 'mousemove' : 'mousedown', {bubbles:true, cancelable:true, altKey:true, shiftKey:shift, button:0, buttons:1, clientX:rect.left + .25, clientY:rect.top + rect.height / 2});
    target.dispatchEvent(event); return event;
}
"#)]
extern "C" {
    fn editorNativeInput(
        target: &web_sys::HtmlTextAreaElement,
        text: &str,
        kind: &str,
        composing: bool,
    );
    fn editorClipboardCut(target: &web_sys::HtmlTextAreaElement) -> web_sys::Event;
    fn editorClipboardCopy(target: &web_sys::HtmlTextAreaElement) -> String;
    fn editorClipboardPaste(target: &web_sys::HtmlTextAreaElement, text: &str) -> web_sys::Event;
    fn editorClipboardPasteData(
        target: &web_sys::HtmlTextAreaElement,
        copied_event: &web_sys::Event,
    ) -> web_sys::Event;
    fn editorWatchPresentation(target: &web_sys::HtmlElement) -> wasm_bindgen::JsValue;
    fn editorPresentationLosses(watch: &wasm_bindgen::JsValue) -> u32;
    fn editorStopPresentationWatch(watch: &wasm_bindgen::JsValue);
    fn editorEdgeGesture(
        target: &web_sys::HtmlTextAreaElement,
        horizontal: bool,
        end: bool,
        kind: &str,
    ) -> web_sys::Event;
    fn editorClickContainerFallback(
        target: &web_sys::HtmlTextAreaElement,
        line: u32,
        column: u32,
        beyond: bool,
    ) -> bool;
    fn editorClickPosition(
        target: &web_sys::HtmlTextAreaElement,
        line: u32,
        column: u32,
        beyond: bool,
    ) -> bool;
    fn editorPrimaryGesture(
        target: &web_sys::HtmlTextAreaElement,
        line: u32,
        column: u32,
        clicks: u32,
        shift: bool,
        kind: &str,
    ) -> web_sys::Event;
    fn editorGesture(
        target: &web_sys::HtmlTextAreaElement,
        line: u32,
        column: u32,
        shift: bool,
        moving: bool,
    ) -> web_sys::Event;
}
fn editor_alt_key(
    textarea: &web_sys::HtmlTextAreaElement,
    key: &str,
    shift: bool,
) -> web_sys::KeyboardEvent {
    let init = web_sys::KeyboardEventInit::new();
    init.set_key(key);
    init.set_alt_key(true);
    init.set_shift_key(shift);
    init.set_bubbles(true);
    init.set_cancelable(true);
    let event =
        web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    textarea.dispatch_event(&event).unwrap();
    event
}

#[wasm_bindgen_test]
async fn line_comment_reindent_and_explicit_paste_commands_share_both_modes() {
    for mode in [
        openwebide_core::WorkspaceMode::Remote,
        openwebide_core::WorkspaceMode::Local,
    ] {
        let folder = if mode == openwebide_core::WorkspaceMode::Local {
            Some(editorConfigFolder().await.unwrap())
        } else {
            None
        };
        let handle = folder.as_ref().map(editorConfigHandle);
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            if let Some(handle) = handle {
                state.projects.local_handles.update(|handles| {
                    handles.insert(1, handle.unchecked_into());
                });
            }
            state.fake.files.borrow_mut().extend([
                (
                    (1, ".editorconfig".into()),
                    "root=true\n[*]\nindent_size=2\n".into(),
                ),
                ((1, "src/a.rs".into()), "a\n😀\nz".into()),
            ]);
            state.workspace.open_file.set(Some("src/a.rs".into()));
            state.workspace.content.set("a\n😀\nz".into());
            editor_view(state)
        });
        wait_until("line command rules", || {
            mounted
                .state
                .workspace
                .editor_rules
                .with_untracked(|rules| {
                    rules
                        .get(&(1, "src/a.rs".into()))
                        .is_some_and(|rules| rules.indentation.width == 2)
                })
        })
        .await;
        let textarea = mounted
            .element(".editor-textarea")
            .unchecked_into::<web_sys::HtmlTextAreaElement>();
        textarea.set_selection_range(2, 2).unwrap();
        assert!(editor_alt_key(&textarea, "ArrowUp", false).default_prevented());
        assert_eq!(mounted.state.workspace.content.get_untracked(), "😀\na\nz");
        assert_eq!(textarea.selection_start().unwrap(), Some(0));
        editor_key(&textarea, "z", true, false);
        assert_eq!(mounted.state.workspace.content.get_untracked(), "a\n😀\nz");
        textarea.set_selection_range(2, 2).unwrap();
        editor_alt_key(&textarea, "ArrowDown", true);
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            "a\n😀\n😀\nz"
        );
        editor_key(&textarea, "z", true, false);
        textarea.set_selection_range(2, 4).unwrap();
        editor_key(&textarea, "D", true, true);
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            "a\n😀😀\nz"
        );
        editor_key(&textarea, "z", true, false);
        textarea.set_selection_range(0, 6).unwrap();
        editor_key(&textarea, "/", true, false);
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            "// a\n// 😀\n// z"
        );
        editor_key(&textarea, "/", true, false);
        assert_eq!(mounted.state.workspace.content.get_untracked(), "a\n😀\nz");
        mounted
            .state
            .workspace
            .content
            .set("fn f() {\nx();\n}".into());
        settle().await;
        let textarea = mounted
            .element(".editor-textarea")
            .unchecked_into::<web_sys::HtmlTextAreaElement>();
        textarea.set_selection_range(0, 15).unwrap();
        mounted.click("button[aria-label='Editing commands']");
        settle().await;
        let items = mounted
            .root
            .query_selector_all("[role='menuitem']")
            .unwrap();
        let reindent = (0..items.length())
            .filter_map(|i| items.item(i))
            .filter_map(|item| item.dyn_into::<web_sys::HtmlElement>().ok())
            .find(|item| item.text_content().as_deref() == Some("Reindent selected lines"))
            .unwrap();
        reindent.click();
        settle().await;
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            "fn f() {\n  x();\n}"
        );
        assert!(
            mounted
                .root
                .query_selector(".ui-dropdown-menu")
                .unwrap()
                .is_none()
        );
        editor_key(&textarea, "z", true, false);
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            "fn f() {\nx();\n}"
        );
        mounted.state.workspace.content.set("  here\nnext".into());
        settle().await;
        let textarea = mounted
            .element(".editor-textarea")
            .unchecked_into::<web_sys::HtmlTextAreaElement>();
        textarea.set_selection_range(2, 6).unwrap();
        assert!(!editorClipboardPaste(&textarea, "  raw").default_prevented());
        assert!(!editor_key(&textarea, "v", true, true).default_prevented());
        assert!(editorClipboardPaste(&textarea, "  😀\n    body").default_prevented());
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            "  😀\n    body\nnext"
        );
        editor_key(&textarea, "z", true, false);
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            "  here\nnext"
        );
        assert!(!editorClipboardPaste(&textarea, "  raw").default_prevented());
        editor_key(&textarea, "v", true, true);
        mounted
            .state
            .workspace
            .open_file
            .set(Some("src/a.json".into()));
        settle().await;
        let textarea = mounted
            .element(".editor-textarea")
            .unchecked_into::<web_sys::HtmlTextAreaElement>();
        assert!(!editorClipboardPaste(&textarea, "untouched").default_prevented());
        mounted.click("button[aria-label='Editing commands']");
        settle().await;
        let items = mounted
            .root
            .query_selector_all("[role='menuitem']")
            .unwrap();
        for label in ["Toggle line comment", "Toggle block comment"] {
            let button = (0..items.length())
                .filter_map(|i| items.item(i))
                .filter_map(|item| item.dyn_into::<web_sys::HtmlButtonElement>().ok())
                .find(|item| item.text_content().as_deref() == Some(label))
                .unwrap();
            assert!(button.disabled());
        }
        mounted
            .state
            .workspace
            .open_file
            .set(Some("preview.png".into()));
        settle().await;
        assert!(
            mounted
                .root
                .query_selector("button[aria-label='Editing commands']")
                .unwrap()
                .is_none()
        );
        drop(mounted);
        if let Some(folder) = folder {
            editorConfigCleanup(&folder).await.unwrap();
        }
    }
}

#[wasm_bindgen_test]
async fn fold_controls_preserve_scrolled_viewport_with_a_distant_caret_in_both_modes() {
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [
        openwebide_core::WorkspaceMode::Local,
        openwebide_core::WorkspaceMode::Remote,
    ] {
        let source = format!(
            "{}fn folded() {{\n    one();\n    two();\n}}\n{}",
            format!("const PAD: &str = \"{}\";\n", "long ".repeat(80)).repeat(100),
            "const END: u8 = 0;\n".repeat(200)
        );
        let expected = source.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("fold-scroll.rs".into()));
            state.workspace.content.set(source.clone());
            view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:420px;height:280px">{editor_view(state)}</div> }
        });
        wait_until("initial fold paint", || {
            mounted
                .root
                .query_selector(".editor-source-line")
                .unwrap()
                .is_some()
        })
        .await;
        let textarea: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        textarea.set_selection_range(0, 0).unwrap();
        openwebide_frontend::viewport::set_editor_scroll_top(&textarea, 1900.0);
        openwebide_frontend::viewport::set_editor_scroll_left(&textarea, 120.0);
        textarea
            .dispatch_event(&web_sys::Event::new("scroll").unwrap())
            .unwrap();
        wait_until("scrolled fold control", || {
            mounted
                .root
                .query_selector("button[aria-label='Collapse block at line 101']")
                .unwrap()
                .is_some()
        })
        .await;
        frame().await;
        let scroll = (
            openwebide_frontend::viewport::editor_scroll(&textarea).scroll_top(),
            openwebide_frontend::viewport::editor_scroll(&textarea).scroll_left(),
        );
        assert!(scroll.0 > 1000.0 && scroll.1 > 50.0);
        let actions = EditorActions::new(mounted.state.workspace);
        for label in ["Collapse block at line 101", "Expand block at line 101"] {
            mounted.click(&format!("button[aria-label='{label}']"));
            settle().await;
            frame().await;
            frame().await;
            assert!(
                (openwebide_frontend::viewport::editor_scroll(&textarea).scroll_top() - scroll.0)
                    .abs()
                    < 1.0,
                "{label}: vertical position {} instead of {}",
                openwebide_frontend::viewport::editor_scroll(&textarea).scroll_top(),
                scroll.0
            );
            assert!(
                (openwebide_frontend::viewport::editor_scroll(&textarea).scroll_left() - scroll.1)
                    .abs()
                    < 1.0,
                "{label}: horizontal position"
            );
            assert!(
                (actions.scroll().top
                    - openwebide_frontend::viewport::editor_scroll(&textarea).scroll_top())
                .abs()
                    < 1.0
            );
            assert_eq!(mounted.state.workspace.content.get_untracked(), expected);
            assert!(!mounted.state.workspace.dirty.get_untracked());
        }
        for label in ["Fold all", "Unfold all"] {
            mounted.click("button[aria-label='Editing commands']");
            settle().await;
            let items = mounted
                .root
                .query_selector_all("[role='menuitem']")
                .unwrap();
            let item = (0..items.length())
                .filter_map(|index| items.item(index))
                .filter_map(|item| item.dyn_into::<web_sys::HtmlElement>().ok())
                .find(|item| item.text_content().as_deref() == Some(label))
                .unwrap();
            item.click();
            settle().await;
            frame().await;
            assert!(
                (openwebide_frontend::viewport::editor_scroll(&textarea).scroll_top() - scroll.0)
                    .abs()
                    < 1.0,
                "menu {label}"
            );
            assert!(
                (openwebide_frontend::viewport::editor_scroll(&textarea).scroll_left() - scroll.1)
                    .abs()
                    < 1.0
            );
        }
        let header = u32::try_from(expected.find("fn folded()").unwrap()).unwrap();
        textarea.set_selection_range(header, header).unwrap();
        openwebide_frontend::viewport::set_editor_scroll_top(&textarea, scroll.0);
        openwebide_frontend::viewport::set_editor_scroll_left(&textarea, scroll.1);
        for key in ["[", "]"] {
            let init = web_sys::KeyboardEventInit::new();
            init.set_key(key);
            init.set_ctrl_key(true);
            init.set_alt_key(true);
            init.set_bubbles(true);
            init.set_cancelable(true);
            textarea
                .dispatch_event(
                    &web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init)
                        .unwrap(),
                )
                .unwrap();
            settle().await;
            frame().await;
            assert!(
                (openwebide_frontend::viewport::editor_scroll(&textarea).scroll_top() - scroll.0)
                    .abs()
                    < 1.0,
                "keyboard {key}"
            );
            assert!(
                (openwebide_frontend::viewport::editor_scroll(&textarea).scroll_left() - scroll.1)
                    .abs()
                    < 1.0
            );
        }
    }
}

#[wasm_bindgen_test]
async fn visible_folding_preserves_source_and_replays_input_in_both_modes() {
    let source = "fn main() {\r\n    /* hidden {\r\n       comment\r\n    */\r\n    let text = \"文😀\";\r\n}\r\n// after\r\n";
    for mode in [
        openwebide_core::WorkspaceMode::Local,
        openwebide_core::WorkspaceMode::Remote,
    ] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("fold.rs".into()));
            state.workspace.content.set(source.into());
            let commands = super::support::command_actions(state.clone());
            view! { <button class="capture-folded" on:click=move |_| commands.run.run(openwebide_frontend::commands::Command::CaptureEditor)>"Capture"</button> {editor_view(state)} }
        });
        wait_until("fold gutter", || {
            mounted
                .root
                .query_selector("button[aria-label='Collapse block at line 1']")
                .unwrap()
                .is_some()
        })
        .await;
        let textarea: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        mounted.click("button[aria-label='Collapse block at line 1']");
        settle().await;
        frame().await;
        assert_eq!(textarea.value(), "fn main() {\n// after\n");
        textarea.set_selection_range(0, 12).unwrap();
        assert_eq!(
            editorClipboardCopy(&textarea),
            source[..source.find("// after").unwrap()]
        );
        mounted.click(".capture-folded");
        settle().await;
        let captured = mounted
            .state
            .chat
            .active_editor_context
            .get_untracked()
            .unwrap();
        assert_eq!(
            captured.selection.unwrap().text,
            source[..source.find("// after").unwrap()]
        );

        assert_eq!(mounted.state.workspace.content.get_untracked(), source);
        assert!(!mounted.state.workspace.dirty.get_untracked());
        assert!(
            mounted
                .root
                .query_selector(".editor-source-line[data-line='5']")
                .unwrap()
                .is_none()
        );
        assert!(
            mounted
                .root
                .query_selector("[data-line='7'] .tok-comment")
                .unwrap()
                .is_some()
        );
        textarea.set_selection_range(12, 12).unwrap();
        textarea
            .dispatch_event(&web_sys::Event::new("select").unwrap())
            .unwrap();
        textarea.set_value("fn main() {\nZ// after\n");
        textarea.set_selection_range(13, 13).unwrap();
        let init = web_sys::InputEventInit::new();
        init.set_bubbles(true);
        init.set_input_type("insertText");
        init.set_data(Some("Z"));
        textarea
            .dispatch_event(&web_sys::InputEvent::new_with_event_init_dict("input", &init).unwrap())
            .unwrap();
        settle().await;
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            source.replace("// after", "Z// after")
        );
        assert_eq!(textarea.value(), "fn main() {\nZ// after\n");
        editor_key(&textarea, "z", true, false);
        settle().await;
        assert_eq!(mounted.state.workspace.content.get_untracked(), source);
        wait_until("fold retained after undo", || {
            mounted
                .root
                .query_selector("button[aria-label='Expand block at line 1']")
                .unwrap()
                .is_some()
        })
        .await;
        textarea.set_selection_range(12, 12).unwrap();
        // Input-only replay maps local Unicode/LF offsets directly to source
        // edits without normalizing unrelated source or dropping the hidden body.
        textarea.set_value("fn main() {\n🦀\n// after\n");
        textarea.set_selection_range(15, 15).unwrap();
        textarea
            .dispatch_event(&web_sys::InputEvent::new_with_event_init_dict("input", &init).unwrap())
            .unwrap();
        settle().await;
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            source.replace("// after", "🦀\r\n// after")
        );
        assert_eq!(textarea.selection_start().unwrap(), Some(15));
        editor_key(&textarea, "z", true, false);
        settle().await;
        assert_eq!(mounted.state.workspace.content.get_untracked(), source);
        textarea.set_selection_range(12, 12).unwrap();
        let before = web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();
        textarea.dispatch_event(&before).unwrap();
        assert!(!before.default_prevented());
        assert_eq!(textarea.value(), "fn main() {\n// after\n");
        assert_eq!(mounted.state.workspace.content.get_untracked(), source);
        settle().await;
        mounted.click("button[aria-label^='Find in file']");
        settle().await;
        let search: web_sys::HtmlInputElement =
            mounted.element(".editor-find input").unchecked_into();
        search.set_value("文");
        search
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
        frame().await;
        assert!(textarea.value().contains("文😀"));
        let normalized = source.replace("\r\n", "\n");
        let offset = u32::try_from(
            normalized[..normalized.find("文").unwrap()]
                .encode_utf16()
                .count(),
        )
        .unwrap();
        assert_eq!(textarea.selection_start().unwrap(), Some(offset));
        assert_eq!(textarea.selection_end().unwrap(), Some(offset + 1));
        assert_eq!(mounted.state.workspace.content.get_untracked(), source);
    }
}

#[wasm_bindgen_test]
async fn fallback_and_region_folds_render_and_reveal_in_both_modes() {
    use openwebide_core::editor::{FoldCommand, SyntaxStatus};
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [
        openwebide_core::WorkspaceMode::Local,
        openwebide_core::WorkspaceMode::Remote,
    ] {
        for (path, source, collapsed, end_line) in [
            (
                "fallback.py",
                "def main():\r\n\tif ready:\r\n\t\twork(\"文😀\")\r\n\tfinish()\r\nafter()\r\n",
                "def main():\nafter()\n",
                3,
            ),
            (
                "fallback.js",
                "function main() {\n  const pattern = /[{}]/;\n  const text = `fake }`;\n}\nafter();\n",
                "function main() {\nafter();\n",
                3,
            ),
            (
                "fallback.yaml",
                "root:\n  first: 1\n  second: 2\nnext: 3\n",
                "root:\nnext: 3\n",
                2,
            ),
            (
                "fallback.txt",
                "#region group\n文😀\n#endregion\nafter\n",
                "#region group\nafter\n",
                2,
            ),
            (
                "region.rs",
                "// #region group\nfn first() {}\n// #endregion\nfn after() {}\n",
                "// #region group\nfn after() {}\n",
                2,
            ),
        ] {
            let mounted = mount_test(move |state| {
                state.seed_project();
                state
                    .projects
                    .projects
                    .update(|projects| projects[0].mode = mode);
                state.workspace.open_file.set(Some(path.into()));
                state.workspace.content.set(source.into());
                editor_view(state)
            });
            wait_until("fallback fold control", || {
                mounted
                    .root
                    .query_selector("button[aria-label='Collapse block at line 1']")
                    .unwrap()
                    .is_some()
            })
            .await;
            let actions = EditorActions::new(mounted.state.workspace);
            let textarea: web_sys::HtmlTextAreaElement =
                mounted.element(".editor-textarea").unchecked_into();
            mounted.click("button[aria-label='Collapse block at line 1']");
            settle().await;
            frame().await;
            assert_eq!(textarea.value(), collapsed, "{path}");
            assert_eq!(mounted.state.workspace.content.get_untracked(), source);
            assert!(!mounted.state.workspace.dirty.get_untracked());
            assert!(
                mounted
                    .root
                    .query_selector(&format!(
                        ".editor-source-line[data-line='{}']",
                        end_line + 2
                    ))
                    .unwrap()
                    .is_some()
            );
            let retained_folds = actions.fold_state().unwrap();
            assert_eq!(
                actions.refresh_fold_ranges(|| false),
                Some(SyntaxStatus::Cancelled)
            );
            settle().await;
            assert_eq!(textarea.value(), collapsed);
            assert_eq!(actions.fold_state().unwrap(), retained_folds);
            assert_eq!(mounted.state.workspace.content.get_untracked(), source);
            assert!(!mounted.state.workspace.dirty.get_untracked());
            actions.refresh_fold_ranges(|| true);
            actions.fold_command(FoldCommand::CollapseAll);
            settle().await;
            assert_eq!(textarea.value(), collapsed);
            actions.fold_command(FoldCommand::Reveal(end_line));
            settle().await;
            assert!(
                textarea
                    .value()
                    .contains(source.lines().nth(end_line).unwrap())
            );
            assert_eq!(mounted.state.workspace.content.get_untracked(), source);
        }
    }
}

#[wasm_bindgen_test]
async fn native_edits_clipboard_commands_and_composition_keep_disjoint_folds_in_both_modes() {
    use openwebide_core::editor::{FoldCommand, byte_to_textarea};
    use openwebide_frontend::state_actions::editor::EditorActions;
    let source = "fn first() {\r\n    one();\r\n}\r\nlet middle = 1;\r\nfn second() {\r\n    two();\r\n}\r\n";
    for mode in [
        openwebide_core::WorkspaceMode::Local,
        openwebide_core::WorkspaceMode::Remote,
    ] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("retained.rs".into()));
            state.workspace.content.set(source.into());
            editor_view(state)
        });
        wait_until("sibling fold ranges", || {
            EditorActions::new(mounted.state.workspace)
                .fold_state()
                .is_some_and(|folds| folds.ranges().iter().any(|range| range.start_line == 4))
        })
        .await;
        let actions = EditorActions::new(mounted.state.workspace);
        let textarea: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        actions.fold_command(FoldCommand::CollapseAll);
        settle().await;
        let select = |needle: &str| {
            let view = textarea.value();
            let offset =
                u32::try_from(byte_to_textarea(&view, view.find(needle).unwrap()).unwrap())
                    .unwrap();
            textarea.set_selection_range(offset, offset).unwrap();
            textarea
                .dispatch_event(&web_sys::Event::new("select").unwrap())
                .unwrap();
        };
        select(";\nfn second");
        let init = web_sys::InputEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_input_type("insertText");
        init.set_data(Some("0"));
        let before = web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();
        textarea.dispatch_event(&before).unwrap();
        assert!(!before.default_prevented());
        editorNativeInput(&textarea, "0", "insertText", false);
        settle().await;
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            source.replace("middle = 1", "middle = 10")
        );
        assert!(!textarea.value().contains("one();"));
        assert!(!textarea.value().contains("two();"));
        assert!(actions.fold_state().unwrap().collapsed_at(0).is_some());
        assert!(actions.fold_state().unwrap().collapsed_at(4).is_some());
        select(";\nfn second");
        assert!(editor_key(&textarea, "Enter", false, false).default_prevented());
        settle().await;
        assert!(actions.fold_state().unwrap().collapsed_at(5).is_some());
        assert!(!textarea.value().contains("two();"));
        editor_key(&textarea, "z", true, false);
        settle().await;
        assert!(actions.fold_state().unwrap().collapsed_at(4).is_some());
        select(";\nfn second");
        assert!(!editorClipboardPaste(&textarea, "pasted").default_prevented());
        editorNativeInput(&textarea, "pasted", "insertFromPaste", false);
        settle().await;
        assert!(
            mounted
                .state
                .workspace
                .content
                .get_untracked()
                .contains("middle = 10pasted;\r\n")
        );
        assert!(!textarea.value().contains("one();"));
        assert!(!textarea.value().contains("two();"));
        select(";\nfn second");
        let before_composition = mounted.state.workspace.content.get_untracked();
        textarea
            .dispatch_event(&web_sys::CompositionEvent::new("compositionstart").unwrap())
            .unwrap();
        editorNativeInput(&textarea, "文", "insertCompositionText", true);
        let caret = textarea.selection_start().unwrap().unwrap();
        textarea.set_selection_range(caret - 1, caret).unwrap();
        editorNativeInput(&textarea, "文😀", "insertCompositionText", true);
        textarea
            .dispatch_event(&web_sys::CompositionEvent::new("compositionend").unwrap())
            .unwrap();
        settle().await;
        assert!(
            mounted
                .state
                .workspace
                .content
                .get_untracked()
                .contains("pasted文😀;\r\n")
        );
        assert!(!textarea.value().contains("one();"));
        assert!(!textarea.value().contains("two();"));
        editor_key(&textarea, "z", true, false);
        settle().await;
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            before_composition
        );
        assert!(actions.fold_state().unwrap().collapsed_at(0).is_some());
        assert!(actions.fold_state().unwrap().collapsed_at(4).is_some());
        select("first");
        init.set_data(Some("x"));
        let before = web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();
        textarea.dispatch_event(&before).unwrap();
        assert!(!before.default_prevented());
        assert!(textarea.value().contains("one();"));
        assert!(!textarea.value().contains("two();"));
        editorNativeInput(&textarea, "x", "insertText", false);
        settle().await;
        assert!(
            mounted
                .state
                .workspace
                .content
                .get_untracked()
                .starts_with("fn xfirst()")
        );
        assert!(actions.fold_state().unwrap().collapsed_at(0).is_none());
        assert!(actions.fold_state().unwrap().collapsed_at(4).is_some());
        select("fn second");
        editor_key(&textarea, "z", true, false);
        settle().await;
        assert!(
            mounted
                .state
                .workspace
                .content
                .get_untracked()
                .starts_with("fn first()")
        );
        assert!(actions.fold_state().unwrap().collapsed_at(4).is_some());
        editor_key(&textarea, "z", true, true);
        settle().await;
        assert!(
            mounted
                .state
                .workspace
                .content
                .get_untracked()
                .starts_with("fn xfirst()")
        );
        assert!(actions.fold_state().unwrap().collapsed_at(4).is_some());
        actions.fold_command(FoldCommand::CollapseAll);
        settle().await;
        let before_cut = mounted.state.workspace.content.get_untracked();
        let view = textarea.value();
        let end = u32::try_from(byte_to_textarea(&view, view.find("let middle").unwrap()).unwrap())
            .unwrap();
        textarea.set_selection_range(0, end).unwrap();
        let cut = editorClipboardCut(&textarea);
        assert!(cut.default_prevented());
        assert_eq!(
            cut.unchecked_ref::<web_sys::ClipboardEvent>()
                .clipboard_data()
                .unwrap()
                .get_data("text/plain")
                .unwrap(),
            before_cut[..before_cut.find("let middle").unwrap()]
        );
        assert!(!textarea.value().contains("one();"));
        assert!(!textarea.value().contains("two();"));
        settle().await;
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            before_cut[before_cut.find("let middle").unwrap()..]
        );
        assert!(actions.fold_state().unwrap().collapsed_at(1).is_some());
        editor_key(&textarea, "z", true, false);
        settle().await;
        assert_eq!(mounted.state.workspace.content.get_untracked(), before_cut);
        assert!(actions.fold_state().unwrap().collapsed_at(4).is_some());
    }
}

#[wasm_bindgen_test]
async fn editor_navigation_status_and_decorations_share_source_coordinates_in_both_modes() {
    use openwebide_core::editor::{FoldCommand, byte_to_textarea};
    use openwebide_frontend::state_actions::editor::EditorActions;
    let source = "fn main() {\r\n    let text = \"文😀\";\r\n}\r\nafter();\r\n";
    for mode in [
        openwebide_core::WorkspaceMode::Local,
        openwebide_core::WorkspaceMode::Remote,
    ] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("navigation.rs".into()));
            state.workspace.content.set(source.into());
            view! { <style>{include_str!("../../styles.css")}</style> <div style="width:600px;height:320px;display:flex">{editor_view(state)}</div> }
        });
        settle().await;
        frame().await;
        let actions = EditorActions::new(mounted.state.workspace);
        let textarea: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        wait_until("navigation source paint ready", || {
            !actions.syntax_is_pending()
                && mounted
                    .element(".editor-code")
                    .class_list()
                    .contains("highlight-ready")
        })
        .await;
        frame().await;
        let before = highlight_count();
        let offset =
            u32::try_from(byte_to_textarea(source, source.find('文').unwrap()).unwrap()).unwrap();
        textarea.set_selection_range(offset, offset + 3).unwrap();
        textarea
            .dispatch_event(&web_sys::Event::new("select").unwrap())
            .unwrap();
        settle().await;
        frame().await;
        assert!(
            mounted
                .element(".editor-cursor-status")
                .text_content()
                .unwrap()
                .contains("2 selected")
        );
        assert!(
            mounted
                .element(".editor-active-line")
                .get_attribute("data-line")
                .as_deref()
                == Some("2")
        );
        assert_eq!(
            highlight_count(),
            before,
            "caret decoration must not regenerate syntax paint"
        );
        assert_eq!(
            mounted
                .element(".editor-source-line[data-line='2']")
                .style()
                .get_property_value("--editor-indent-columns")
                .unwrap(),
            "4"
        );
        let guide = web_sys::window()
            .unwrap()
            .get_computed_style_with_pseudo_elt(
                &mounted.element(".editor-source-line[data-line='2']"),
                "::after",
            )
            .unwrap()
            .unwrap();
        assert!(
            guide
                .get_property_value("width")
                .unwrap()
                .trim_end_matches("px")
                .parse::<f64>()
                .unwrap()
                > 1.0
        );

        actions.fold_command(FoldCommand::CollapseAll);
        settle().await;
        editor_key(&textarea, "g", true, false);
        settle().await;
        let input: web_sys::HtmlInputElement =
            mounted.element(".editor-navigation input").unchecked_into();
        input.set_value("2:17");
        input
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
        mounted.click_text("Go");
        settle().await;
        frame().await;
        frame().await;
        assert!(textarea.value().contains("文😀"));
        assert_eq!(textarea.selection_start().unwrap(), Some(offset));
        assert!(
            mounted
                .element(".editor-cursor-status")
                .text_content()
                .unwrap()
                .contains("Ln 2, Col 17")
        );
        assert_eq!(mounted.state.workspace.content.get_untracked(), source);
        assert!(!mounted.state.workspace.dirty.get_untracked());
        let open = source.find('{').unwrap();
        textarea
            .set_selection_range(u32::try_from(open).unwrap(), u32::try_from(open).unwrap())
            .unwrap();
        textarea
            .dispatch_event(&web_sys::Event::new("select").unwrap())
            .unwrap();
        settle().await;
        frame().await;
        assert_eq!(
            mounted
                .root
                .query_selector_all(".editor-bracket-match")
                .unwrap()
                .length(),
            2
        );
        {
            let mark = mounted.element(".editor-bracket-match");
            assert!(mark.get_bounding_client_rect().width() > 1.0);
            assert!(mark.get_bounding_client_rect().height() > 1.0);
        }
        actions.fold_command(FoldCommand::CollapseAll);
        settle().await;
        editor_key(&textarea, "\\", true, true);
        settle().await;
        frame().await;
        frame().await;
        let close = u32::try_from(byte_to_textarea(source, source.find("}\r\n").unwrap()).unwrap())
            .unwrap();
        assert_eq!(textarea.selection_start().unwrap(), Some(close));
        assert!(textarea.value().contains("文😀"));
        editor_key(&textarea, "\\", true, true);
        settle().await;
        assert_eq!(
            textarea.selection_start().unwrap(),
            Some(u32::try_from(open).unwrap())
        );
        editor_key(&textarea, "g", true, false);
        settle().await;
        let input: web_sys::HtmlInputElement =
            mounted.element(".editor-navigation input").unchecked_into();
        input.set_value("0:not-a-column");
        input
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
        settle().await;
        assert!(
            mounted
                .element(".editor-navigation button.btn")
                .has_attribute("disabled")
        );
        input.set_value("2:1");
        input
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
        settle().await;
        mounted.click_text("Go");
        mounted
            .state
            .workspace
            .open_file
            .set(Some("other.rs".into()));
        mounted.state.workspace.content.set("other".into());
        settle().await;
        frame().await;
        frame().await;
        assert!(
            mounted
                .root
                .query_selector(".editor-navigation")
                .unwrap()
                .is_none()
        );
        assert_eq!(
            mounted
                .element(".editor-textarea")
                .unchecked_into::<web_sys::HtmlTextAreaElement>()
                .selection_start()
                .unwrap(),
            Some(0)
        );
        mounted
            .state
            .workspace
            .content
            .set(format!("header\n{}\n", "x".repeat(1100)));
        settle().await;
        frame().await;
        let textarea: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        editor_key(&textarea, "g", true, false);
        settle().await;
        let input: web_sys::HtmlInputElement =
            mounted.element(".editor-navigation input").unchecked_into();
        input.set_value("2:1001");
        input
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
        settle().await;
        mounted.click_text("Go");
        settle().await;
        wait_until("navigation reveals long-line column", || {
            textarea.scroll_left() > 1000.0
        })
        .await;
        assert_eq!(textarea.selection_start().unwrap(), Some(1007));
    }
}

#[wasm_bindgen_test]
async fn search_replace_options_scope_captures_and_failures_share_both_modes() {
    use openwebide_core::editor::{SearchOptions, Selection};
    use openwebide_frontend::state_actions::editor::EditorActions;
    let source = "😀 café caféine CAFÉ\r\ncafé\r\n";
    for mode in [
        openwebide_core::WorkspaceMode::Local,
        openwebide_core::WorkspaceMode::Remote,
    ] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("search.rs".into()));
            state.workspace.content.set(source.into());
            editor_view(state)
        });
        settle().await;
        let actions = EditorActions::new(mounted.state.workspace);
        let textarea: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        textarea.set_selection_range(3, 7).unwrap();
        textarea
            .dispatch_event(&web_sys::Event::new("select").unwrap())
            .unwrap();
        mounted.click("button[aria-label^='Find in file']");
        settle().await;
        let search: web_sys::HtmlInputElement =
            mounted.element(".editor-find input").unchecked_into();
        let set_query = |value: &str| {
            search.set_value(value);
            search
                .dispatch_event(&web_sys::Event::new("input").unwrap())
                .unwrap();
        };
        let toggle = |label: &str| {
            let labels = mounted
                .root
                .query_selector_all(".editor-find-options label")
                .unwrap();
            for index in 0..labels.length() {
                let node = labels.item(index).unwrap();
                if node.text_content().as_deref() == Some(label) {
                    let label: web_sys::HtmlElement = node.unchecked_into();
                    label.click();
                    return;
                }
            }
            panic!("Missing search option {label}");
        };
        set_query("café");
        settle().await;
        assert!(
            mounted
                .element(".editor-find")
                .text_content()
                .unwrap()
                .contains("1 / 3")
        );
        toggle("Match case");
        toggle("Whole word");
        settle().await;
        assert!(
            mounted
                .element(".editor-find")
                .text_content()
                .unwrap()
                .contains("1 / 3")
        );
        toggle("In selection");
        settle().await;
        assert!(
            mounted
                .element(".editor-find")
                .text_content()
                .unwrap()
                .contains("1 / 1")
        );
        mounted.click_text("Replace");
        settle().await;
        let replacement: web_sys::HtmlInputElement = mounted
            .element("input[aria-label='Replace with']")
            .unchecked_into();
        let set_replacement = |value: &str| {
            replacement.set_value(value);
            replacement
                .dispatch_event(&web_sys::Event::new("input").unwrap())
                .unwrap();
        };
        set_replacement("tea😀");
        mounted.click_text("Replace all");
        settle().await;
        assert_eq!(actions.source(), "😀 tea😀 caféine CAFÉ\r\ncafé\r\n");
        assert!(mounted.state.workspace.dirty.get_untracked());
        editor_key(&textarea, "z", true, false);
        settle().await;
        assert_eq!(actions.source(), source);
        // Undo invalidates the old captured scope; full-document replacement remains atomic.
        toggle("Regex");
        set_query("(?P<word>café)");
        set_replacement("${word}!");
        settle().await;
        mounted.click_text("Replace all");
        settle().await;
        assert_eq!(actions.source(), "😀 café! caféine CAFÉ!\r\ncafé!\r\n");
        editor_key(&textarea, "z", true, false);
        settle().await;
        assert_eq!(actions.source(), source);
        set_query("[");
        settle().await;
        assert!(
            mounted
                .element("[role='alert']")
                .text_content()
                .unwrap()
                .contains("Invalid search pattern")
        );
        let replace_all: web_sys::HtmlButtonElement = mounted
            .element(".editor-replace button:last-child")
            .unchecked_into();
        assert!(replace_all.disabled());
        assert_eq!(actions.source(), source);
        set_query("^");
        set_replacement(">");
        settle().await;
        toggle("Whole word");
        settle().await;
        mounted.click_text("Replace all");
        settle().await;
        assert_eq!(actions.source(), ">😀 café caféine CAFÉ\r\n>café\r\n>");
        editor_key(&textarea, "z", true, false);
        settle().await;
        assert_eq!(actions.source(), source);
        assert!(
            actions
                .replace_search("stale", "café", SearchOptions::default(), None, "x", None)
                .is_err()
        );
        mounted
            .state
            .workspace
            .open_file
            .set(Some("other.rs".into()));
        mounted.state.workspace.content.set("other".into());
        settle().await;
        assert_eq!(actions.source(), "other");
        assert_eq!(actions.selection("other"), Some(Selection::caret(0)));
        mounted.state.workspace.merge_pending(
            1,
            openwebide_core::FileDiff {
                path: "other.rs".into(),
                old: Some("other".into()),
                new: "pending".into(),
                old_unavailable: false,
                backup_path: None,
            },
        );
        settle().await;
        assert!(replace_all.disabled());
        assert_eq!(
            actions.replace_search("other", "other", SearchOptions::default(), None, "x", None),
            Err(openwebide_core::editor::SearchError::ReadOnly)
        );
        assert_eq!(actions.source(), "other");
    }
}

#[wasm_bindgen_test]
async fn wrapping_whitespace_fold_geometry_and_navigation_share_both_modes() {
    use openwebide_core::editor::{FoldCommand, line_column};
    use openwebide_frontend::state_actions::editor::EditorActions;
    let source = format!(
        "fn first() {{\r\n\tlet text = \"{}end\";\r\n}}\r\nfn second() {{\r\n    after();\r\n}}\r\n",
        "文😀 words ".repeat(45)
    );
    for mode in [
        openwebide_core::WorkspaceMode::Local,
        openwebide_core::WorkspaceMode::Remote,
    ] {
        let source = source.clone();
        let expected = source.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("wrapped.rs".into()));
            state.workspace.content.set(source.clone());
            state.settings.editor_preferences.update(|preferences| {
                preferences.word_wrap = true;
                preferences.show_whitespace = true;
            });
            view! { <style>{include_str!("../../styles.css")}</style><div class="wrap-fixture" style="display:flex;width:340px;height:340px">{editor_view(state)}</div> }
        });
        let textarea: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        let actions = EditorActions::new(mounted.state.workspace);
        wait_until("wrapped line and fold row measurement", || {
            let row = mounted
                .root
                .query_selector(".editor-source-line[data-line='2']")
                .unwrap();
            let fold = mounted
                .root
                .query_selector(".editor-fold-row:nth-child(2)")
                .unwrap();
            row.zip(fold).is_some_and(|(row, fold)| {
                row.get_bounding_client_rect().height() > 100.0
                    && (row.get_bounding_client_rect().height()
                        - fold.get_bounding_client_rect().height())
                    .abs()
                        < 1.0
            })
        })
        .await;
        assert_eq!(textarea.wrap(), "soft");
        assert!(textarea.scroll_width() <= textarea.client_width() + 1);
        let paint = mounted.element(".editor-highlight-content");
        assert_eq!(paint.text_content().unwrap(), textarea.value());
        assert!(
            mounted
                .root
                .query_selector(".editor-space")
                .unwrap()
                .is_some()
        );
        assert!(
            mounted
                .root
                .query_selector(".editor-tab")
                .unwrap()
                .is_some()
        );
        assert!(
            mounted
                .root
                .query_selector(".editor-line-ending")
                .unwrap()
                .is_some()
        );
        let full_height = paint.get_bounding_client_rect().height();
        assert!(
            (full_height - f64::from(textarea.scroll_height())).abs() < 2.0,
            "native and paint wrapping differ: {} vs {}",
            textarea.scroll_height(),
            full_height
        );
        mounted.click_text("Ln 1, Col 1");
        settle().await;
        let go: web_sys::HtmlInputElement = mounted
            .element("input[aria-label='Go to line and column']")
            .unchecked_into();
        go.set_value("2:220");
        go.dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
        mounted.click_text("Go");
        wait_until("wrapped-column navigation scroll", || {
            textarea.scroll_top() > 100.0
        })
        .await;
        assert_eq!(
            line_column(&expected, actions.selection(&expected).unwrap().head),
            (2, 220)
        );
        assert_eq!(actions.source(), expected);
        assert!(!mounted.state.workspace.dirty.get_untracked());
        let narrow = mounted
            .element(".editor-source-line[data-line='2']")
            .get_bounding_client_rect()
            .height();
        mounted
            .element(".wrap-fixture")
            .style()
            .set_property("width", "540px")
            .unwrap();
        wait_until("resized wrapped row and fold geometry", || {
            let row = mounted
                .element(".editor-source-line[data-line='2']")
                .get_bounding_client_rect()
                .height();
            let fold = mounted
                .element(".editor-fold-row:nth-child(2)")
                .get_bounding_client_rect()
                .height();
            row < narrow && (row - fold).abs() < 1.0
        })
        .await;
        wait_until("resized source dimensions prepared", || {
            mounted
                .element(".editor-scroll-extent")
                .has_attribute("data-source-width")
        })
        .await;
        frame().await;
        // Layout changes may generate temporary measurement HTML. The reusable
        // visible paint must remain unchanged when only wrapping is toggled.
        let before = openwebide_frontend::components::viewport_highlight_count();
        mounted
            .state
            .settings
            .editor_preferences
            .update(|preferences| preferences.word_wrap = false);
        wait_until("horizontal default restored", || {
            textarea.wrap() == "off" && textarea.scroll_width() > textarea.client_width()
        })
        .await;
        frame().await;
        frame().await;
        assert_eq!(
            openwebide_frontend::components::viewport_highlight_count(),
            before
        );
        assert_eq!(actions.source(), expected);
        mounted
            .state
            .settings
            .editor_preferences
            .update(|preferences| {
                preferences.word_wrap = true;
                preferences.show_whitespace = false;
            });
        wait_until("whitespace disabled", || {
            mounted
                .root
                .query_selector(".editor-space")
                .unwrap()
                .is_none()
        })
        .await;
        assert_eq!(paint.text_content().unwrap(), textarea.value());
        actions.fold_command(FoldCommand::CollapseAll);
        wait_until("folded wrapped rows align", || {
            let rows = mounted
                .root
                .query_selector_all(".editor-source-line")
                .unwrap();
            let folds = mounted.root.query_selector_all(".editor-fold-row").unwrap();
            rows.length() == 3 && rows.length() == folds.length()
        })
        .await;
        assert!(
            mounted
                .root
                .query_selector(".editor-source-line[data-line='4']")
                .unwrap()
                .is_some()
        );
        assert_eq!(actions.source(), expected);
        assert!(!mounted.state.workspace.dirty.get_untracked());
    }
}

#[wasm_bindgen_test]
async fn recovery_document_format_restores_committed_browser_edits_in_both_modes() {
    use openwebide_core::editor::{DocumentRecovery, Selection};
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [
        openwebide_core::WorkspaceMode::Local,
        openwebide_core::WorkspaceMode::Remote,
    ] {
        let actions_slot = std::rc::Rc::new(std::cell::Cell::new(None));
        let slot = actions_slot.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state.projects.projects.update(|items| items[0].mode = mode);
            state.workspace.open_file.set(Some("main.rs".into()));
            state.workspace.content.set("α\r\n".into());
            slot.set(Some(EditorActions::new(state.workspace)));
            editor_view(state)
        });
        settle().await;
        let actions = actions_slot.get().unwrap();
        actions
            .native_input("α😀\n".into(), Selection::caret(6), "insertText", 1.0)
            .unwrap();
        let snapshot = mounted
            .state
            .workspace
            .editor_documents
            .with_untracked(|documents| documents[&(1, "main.rs".into())].recovery());
        let encoded = serde_json::to_string(&snapshot).unwrap();
        let decoded: DocumentRecovery = serde_json::from_str(&encoded).unwrap();
        let mut restored = decoded.restore().unwrap();
        assert_eq!(restored.text(), "α😀\r\n");
        assert_eq!(restored.selections(), &[Selection::caret(6)]);
        assert!(restored.is_dirty());
        assert!(restored.undo());
        assert_eq!(restored.text(), "α\r\n");
        assert!(!restored.is_dirty());
        assert!(restored.redo());
        assert_eq!(restored.text(), snapshot.text);
    }
}

#[wasm_bindgen_test]
async fn workspace_recovery_collects_active_and_hidden_drafts_without_composition_previews() {
    use openwebide_core::editor::{EditorRecoveryRecord, Selection};
    use openwebide_frontend::{backend::RecoveryError, state_actions::editor::EditorActions};
    for mode in [
        openwebide_core::WorkspaceMode::Local,
        openwebide_core::WorkspaceMode::Remote,
    ] {
        let slot = std::rc::Rc::new(std::cell::Cell::new(None));
        let capture = slot.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("one.rs".into()));
            state.workspace.content.set("one".into());
            capture.set(Some(EditorActions::new(state.workspace)));
            editor_view(state)
        });
        settle().await;
        let editor = slot.get().unwrap();
        editor
            .native_input("one!".into(), Selection::caret(4), "insertText", 1.0)
            .unwrap();
        mounted.state.workspace.retain_editor_buffer(false);
        mounted
            .state
            .workspace
            .register_editor_tab(1, "one.rs".into());
        batch(|| {
            mounted.state.workspace.open_file.set(Some("two.rs".into()));
            mounted.state.workspace.content.set("two".into());
            mounted.state.workspace.dirty.set(false);
        });
        settle().await;
        editor
            .native_input("two!".into(), Selection::caret(4), "insertText", 2.0)
            .unwrap();
        editor.begin_composition();
        editor
            .native_input(
                "two!文".into(),
                Selection::caret(7),
                "insertCompositionText",
                3.0,
            )
            .unwrap();
        let project = mounted.state.projects.project(1).unwrap();
        let candidate = mounted
            .state
            .workspace
            .editor_recovery(&project, false)
            .unwrap();
        assert_eq!(candidate.selected.as_deref(), Some("two.rs"));
        assert_eq!(
            candidate
                .files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            vec!["one.rs", "two.rs"]
        );
        assert_eq!(candidate.files[0].document.as_ref().unwrap().text, "one!");
        assert_eq!(candidate.files[0].document.as_ref().unwrap().saved, "one");
        assert_eq!(candidate.files[1].document.as_ref().unwrap().text, "two!");
        assert_eq!(candidate.files[1].document.as_ref().unwrap().saved, "two");
        let backend = mounted.state.api.with_value(Clone::clone);
        let record = EditorRecoveryRecord {
            revision: 0,
            state: candidate,
        };
        assert_eq!(backend.save_editor_recovery(1, &record).await.unwrap(), 1);
        assert_eq!(
            backend.editor_recovery(1).await.unwrap().state,
            record.state
        );
        assert!(matches!(
            backend.save_editor_recovery(1, &record).await,
            Err(RecoveryError::Conflict(_))
        ));
        let loaded = backend.editor_recovery(1).await.unwrap();
        assert_eq!(loaded.revision, 1);
        editor.cancel_composition();
        // Unsupported direct dirty assignments must never fabricate a clean baseline.
        mounted
            .state
            .workspace
            .content
            .set("foreign direct assignment".into());
        mounted.state.workspace.dirty.set(true);
        assert!(
            mounted
                .state
                .workspace
                .editor_recovery(&project, false)
                .is_err()
        );
    }
}

#[wasm_bindgen_test]
async fn recovery_hydration_and_disk_reconciliation_use_both_real_workspace_adapters() {
    use openwebide_core::{
        WorkspaceMode,
        editor::{
            Document, EditorRecovery, EditorRecoveryFile, EditorRecoveryRoot, RecoveryDiskState,
            RecoveryScroll, Selection,
        },
    };
    use openwebide_frontend::workspace::Workspace;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let folder = if mode == WorkspaceMode::Local {
            Some(editorConfigFolder().await.unwrap())
        } else {
            None
        };
        let handle = folder.as_ref().map(editorConfigHandle);
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            if let Some(handle) = handle {
                state.projects.local_handles.update(|handles| {
                    handles.insert(1, handle.unchecked_into());
                });
            }
            state
                .fake
                .files
                .borrow_mut()
                .insert((1, "src/a.rs".into()), "😀\r\n  tail  ".into());
            editor_view(state)
        });
        settle().await;
        let project = mounted.state.projects.project(1).unwrap();
        let mut document = Document::new("😀\r\n  tail  ");
        document.set_selections(vec![Selection::caret(4)]).unwrap();
        document.replace_selections("文", None).unwrap();
        let draft = document.recovery();
        let recovery = EditorRecovery {
            format: 1,
            root: Some(EditorRecoveryRoot::for_project(&project)),
            selected: Some("src/a.rs".into()),
            files: vec![EditorRecoveryFile {
                path: "src/a.rs".into(),
                document: Some(draft.clone()),
                scroll: RecoveryScroll::default(),
                read_only: false,
            }],
        };
        let guard = mounted
            .state
            .workspace
            .editor_recovery_guard(&project, false)
            .unwrap();
        mounted
            .state
            .workspace
            .restore_editor_recovery(&project, &guard, &recovery, false)
            .unwrap()
            .unwrap();
        settle().await;
        let textarea: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        assert_eq!(textarea.value(), draft.text.replace("\r\n", "\n"));
        assert!(mounted.state.workspace.dirty.get_untracked());
        assert_eq!(
            mounted
                .state
                .workspace
                .editor_recovery(&project, false)
                .unwrap(),
            recovery
        );
        let ws = Workspace::for_project(mounted.state.api, mounted.state.projects, 1).unwrap();
        let disk = ws.read_optional_bytes("src/a.rs").await.unwrap().unwrap();
        let disk = String::from_utf8(disk).unwrap();
        assert_eq!(
            draft.reconcile_disk(Some(&disk)).unwrap().1,
            RecoveryDiskState::Current
        );
        ws.write("src/a.rs", "external").await.unwrap();
        let disk =
            String::from_utf8(ws.read_optional_bytes("src/a.rs").await.unwrap().unwrap()).unwrap();
        let (restored, status) = draft.reconcile_disk(Some(&disk)).unwrap();
        assert_eq!(status, RecoveryDiskState::Conflict);
        assert_eq!(restored.recovery(), draft);
        assert!(
            ws.read_optional_bytes("missing/a.rs")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            ws.read_optional_bytes("src/missing.rs")
                .await
                .unwrap()
                .is_none()
        );
        assert!(ws.read_optional_bytes("../a.rs").await.is_err());
        // An edit arriving after a load captured its guard must win atomically.
        let guard = mounted
            .state
            .workspace
            .editor_recovery_guard(&project, false)
            .unwrap();
        let actions =
            openwebide_frontend::state_actions::editor::EditorActions::new(mounted.state.workspace);
        let changed = format!("{}!", draft.text);
        let projected = changed.replace("\r\n", "\n");
        actions
            .native_input(
                projected.clone(),
                Selection::caret(projected.len()),
                "insertText",
                1.0,
            )
            .unwrap();
        assert!(
            mounted
                .state
                .workspace
                .restore_editor_recovery(&project, &guard, &EditorRecovery::default(), false)
                .unwrap()
                .is_none()
        );
        assert_eq!(mounted.state.workspace.content.get_untracked(), changed);
        if let Some(folder) = folder {
            editorConfigCleanup(&folder).await.unwrap();
        }
    }
}

fn sign_in_recovery(state: &super::support::TestState) {
    state.auth.set_user(openwebide_core::User {
        id: openwebide_core::UserId::new(1),
        username: "alice".into(),
        role: openwebide_core::UserRole::User,
        created_at: 0,
    });
    state.projects.projects_loaded.set(true);
}

#[wasm_bindgen_test]
async fn automatic_editor_recovery_restores_and_saves_drafts_in_both_modes() {
    use openwebide_core::{
        WorkspaceMode,
        editor::{
            Document, EditorRecovery, EditorRecoveryFile, EditorRecoveryRecord, EditorRecoveryRoot,
            RecoveryScroll, Selection,
        },
    };
    use openwebide_frontend::{
        state::editor_recovery::{EditorRecoveryState, RecoveryPhase},
        state_actions::{editor::EditorActions, workspace::WorkspaceActions},
    };
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let folder = if mode == WorkspaceMode::Local {
            Some(editorConfigFolder().await.unwrap())
        } else {
            None
        };
        let handle = folder.as_ref().map(editorConfigHandle);
        let slots = std::rc::Rc::new(std::cell::Cell::new(None));
        let capture = slots.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            sign_in_recovery(&state);
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            if let Some(handle) = handle {
                state.projects.local_handles.update(|handles| {
                    handles.insert(1, handle.unchecked_into());
                });
            }
            state
                .fake
                .files
                .borrow_mut()
                .insert((1, "src/a.rs".into()), "😀\r\n  tail  ".into());
            let mut doc = Document::new("😀\r\n  tail  ");
            doc.replace_selections("draft ", None).unwrap();
            state.fake.editor_recovery_records.borrow_mut().insert(
                1,
                EditorRecoveryRecord {
                    revision: 7,
                    state: EditorRecovery {
                        format: 1,
                        root: Some(EditorRecoveryRoot::for_project(
                            &state.projects.project(1).unwrap(),
                        )),
                        selected: Some("src/a.rs".into()),
                        files: vec![EditorRecoveryFile {
                            path: "src/a.rs".into(),
                            document: Some(doc.recovery()),
                            scroll: RecoveryScroll::default(),
                            read_only: false,
                        }],
                    },
                },
            );
            let view = super::support::recovery_editor_view(state);
            capture.set(Some((
                expect_context::<EditorRecoveryState>(),
                expect_context::<WorkspaceActions>(),
            )));
            view
        });
        let (recovery, actions) = slots.get().unwrap();
        wait_until("automatic draft hydration and disk check", || {
            recovery.projects.with_untracked(|projects| {
                projects
                    .get(&1)
                    .is_some_and(|entry| entry.phase == RecoveryPhase::Ready)
            }) && mounted.state.workspace.open_file.get_untracked().as_deref() == Some("src/a.rs")
                && mounted
                    .state
                    .workspace
                    .editor_recovery_checks
                    .with_untracked(std::collections::HashMap::is_empty)
        })
        .await;
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            "draft 😀\r\n  tail  "
        );
        assert!(mounted.state.workspace.dirty.get_untracked());
        let editor = EditorActions::new(mounted.state.workspace);
        let projected = "latest draft 😀\n  tail  ";
        editor
            .native_input(
                projected.into(),
                Selection::caret(projected.len()),
                "insertText",
                1.0,
            )
            .unwrap();
        wait_until("automatic recovery save", || {
            mounted
                .state
                .fake
                .editor_recovery_records
                .borrow()
                .get(&1)
                .is_some_and(|record| {
                    record.revision > 7
                        && record.state.files[0].document.as_ref().unwrap().text
                            == "latest draft 😀\r\n  tail  "
                })
        })
        .await;
        assert_eq!(
            mounted.state.fake.files.borrow()[&(1, "src/a.rs".into())],
            "😀\r\n  tail  "
        );
        let saved = mounted.state.fake.editor_recovery_records.borrow()[&1].clone();
        let handle = folder.as_ref().map(editorConfigHandle);
        let second =
            super::support::mount_test_with_backend(mounted.state.fake.clone(), move |state| {
                state.seed_project();
                sign_in_recovery(&state);
                state
                    .projects
                    .projects
                    .update(|projects| projects[0].mode = mode);
                if let Some(handle) = handle {
                    state.projects.local_handles.update(|handles| {
                        handles.insert(1, handle.unchecked_into());
                    });
                }
                super::support::recovery_editor_view(state)
            });
        wait_until("fresh window restores selected file and draft", || {
            second.state.workspace.open_file.get_untracked().as_deref() == Some("src/a.rs")
                && second.state.workspace.content.get_untracked() == "latest draft 😀\r\n  tail  "
                && second
                    .state
                    .workspace
                    .editor_recovery_checks
                    .with_untracked(std::collections::HashMap::is_empty)
        })
        .await;
        assert!(second.state.workspace.dirty.get_untracked());
        assert_eq!(
            second
                .state
                .workspace
                .editor_documents
                .with_untracked(|documents| documents[&(1, "src/a.rs".into())].recovery().saved),
            "😀\r\n  tail  "
        );
        drop(second);
        actions.close_file.run("src/a.rs".into());
        mounted
            .state
            .ui
            .confirm
            .get_untracked()
            .unwrap()
            .action
            .run(());
        wait_until("close-all recovery tombstone", || {
            mounted.state.fake.editor_recovery_records.borrow()[&1]
                .state
                .files
                .is_empty()
        })
        .await;
        assert!(mounted.state.fake.editor_recovery_records.borrow()[&1].revision > saved.revision);
        if let Some(folder) = folder {
            editorConfigCleanup(&folder).await.unwrap();
        }
    }
}

#[wasm_bindgen_test]
async fn editor_recovery_coalesces_slow_writes_and_preserves_errors_and_revision_conflicts() {
    use openwebide_core::editor::Selection;
    use openwebide_frontend::{
        backend::RecoveryError,
        state::editor_recovery::{EditorRecoveryState, RecoveryPhase},
        state_actions::editor::EditorActions,
    };
    let slot = std::rc::Rc::new(std::cell::Cell::new(None));
    let capture = slot.clone();
    let mounted = mount_test(move |state| {
        state.seed_project();
        sign_in_recovery(&state);
        state.workspace.open_file.set(Some("a.rs".into()));
        state.workspace.content.set("base".into());
        let view = super::support::recovery_editor_view(state);
        capture.set(Some(expect_context::<EditorRecoveryState>()));
        view
    });
    let recovery = slot.get().unwrap();
    wait_until("recovery ready", || {
        recovery.projects.with_untracked(|projects| {
            projects
                .get(&1)
                .is_some_and(|entry| entry.phase == RecoveryPhase::Ready)
        })
    })
    .await;
    let editor = EditorActions::new(mounted.state.workspace);
    let (send, receive) = futures::channel::oneshot::channel();
    mounted
        .state
        .fake
        .recovery_save_results
        .borrow_mut()
        .push_back(receive);
    editor
        .native_input("first".into(), Selection::caret(5), "insertText", 1.0)
        .unwrap();
    wait_until("slow recovery write", || {
        recovery
            .projects
            .with_untracked(|projects| projects[&1].writing)
    })
    .await;
    editor
        .native_input("newest".into(), Selection::caret(6), "insertText", 2.0)
        .unwrap();
    send.send(Ok(())).unwrap();
    wait_until("coalesced newest draft", || {
        mounted
            .state
            .fake
            .editor_recovery_records
            .borrow()
            .get(&1)
            .is_some_and(|record| record.state.files[0].document.as_ref().unwrap().text == "newest")
    })
    .await;
    let (send, receive) = futures::channel::oneshot::channel();
    mounted
        .state
        .fake
        .recovery_save_results
        .borrow_mut()
        .push_back(receive);
    send.send(Err(RecoveryError::Unavailable("offline".into())))
        .unwrap();
    editor
        .native_input(
            "offline draft".into(),
            Selection::caret(13),
            "insertText",
            3.0,
        )
        .unwrap();
    wait_until("visible recovery failure", || {
        recovery
            .projects
            .with_untracked(|projects| matches!(projects[&1].phase, RecoveryPhase::SaveFailed(_)))
    })
    .await;
    assert_eq!(
        mounted.state.workspace.content.get_untracked(),
        "offline draft"
    );
    mounted.click_text("Retry recovery");
    wait_until("retry saves retained draft", || {
        mounted.state.fake.editor_recovery_records.borrow()[&1]
            .state
            .files[0]
            .document
            .as_ref()
            .unwrap()
            .text
            == "offline draft"
    })
    .await;
    mounted
        .state
        .fake
        .editor_recovery_records
        .borrow_mut()
        .get_mut(&1)
        .unwrap()
        .revision += 1;
    editor
        .native_input(
            "window draft".into(),
            Selection::caret(12),
            "insertText",
            4.0,
        )
        .unwrap();
    wait_until("revision conflict stops recovery writes", || {
        recovery
            .projects
            .with_untracked(|projects| matches!(projects[&1].phase, RecoveryPhase::Conflict(_)))
    })
    .await;
    assert_eq!(
        mounted.state.workspace.content.get_untracked(),
        "window draft"
    );
    assert_eq!(
        mounted.state.fake.editor_recovery_records.borrow()[&1]
            .state
            .files[0]
            .document
            .as_ref()
            .unwrap()
            .text,
        "offline draft"
    );
    mounted.click_text("Keep this window");
    mounted
        .state
        .ui
        .confirm
        .get_untracked()
        .unwrap()
        .action
        .run(());
    wait_until("explicit conflict choice saves this window", || {
        mounted.state.fake.editor_recovery_records.borrow()[&1]
            .state
            .files[0]
            .document
            .as_ref()
            .unwrap()
            .text
            == "window draft"
    })
    .await;
}

fn draft_record(
    project: &openwebide_core::Project,
) -> openwebide_core::editor::EditorRecoveryRecord {
    use openwebide_core::editor::*;
    let mut document = Document::new("base");
    document.replace_selections("draft ", None).unwrap();
    EditorRecoveryRecord {
        revision: 1,
        state: EditorRecovery {
            format: 1,
            root: Some(EditorRecoveryRoot::for_project(project)),
            selected: Some("a.rs".into()),
            files: vec![EditorRecoveryFile {
                path: "a.rs".into(),
                document: Some(document.recovery()),
                scroll: RecoveryScroll::default(),
                read_only: false,
            }],
        },
    }
}

#[wasm_bindgen_test]
async fn editor_recovery_late_load_preserves_new_files_and_does_not_cross_accounts() {
    use openwebide_frontend::{
        state::editor_recovery::{EditorRecoveryState, RecoveryPhase},
        state_actions::workspace::WorkspaceActions,
    };
    let slot = std::rc::Rc::new(std::cell::Cell::new(None));
    let capture = slot.clone();
    let (send, receive) = futures::channel::oneshot::channel();
    let mounted = mount_test(move |state| {
        state.seed_project();
        sign_in_recovery(&state);
        state
            .fake
            .files
            .borrow_mut()
            .insert((1, "new.rs".into()), "new file".into());
        state
            .fake
            .recovery_load_results
            .borrow_mut()
            .push_back(receive);
        let view = super::support::recovery_editor_view(state);
        capture.set(Some((
            expect_context::<EditorRecoveryState>(),
            expect_context::<WorkspaceActions>(),
        )));
        view
    });
    let (recovery, actions) = slot.get().unwrap();
    wait_until("deferred recovery load", || {
        mounted.state.fake.calls.borrow().iter().any(|call| {
            matches!(
                call,
                openwebide_frontend::testing::fake_backend::Call::Request {
                    method: "editor_recovery"
                }
            )
        })
    })
    .await;
    actions.request_open.run("new.rs".into());
    wait_until("new file opened before recovery response", || {
        mounted.state.workspace.content.get_untracked() == "new file"
    })
    .await;
    send.send(Ok(draft_record(
        &mounted.state.projects.project(1).unwrap(),
    )))
    .unwrap();
    wait_until("late load offers explicit choice", || {
        recovery
            .projects
            .with_untracked(|projects| matches!(projects[&1].phase, RecoveryPhase::Conflict(_)))
    })
    .await;
    assert_eq!(
        mounted.state.workspace.open_file.get_untracked().as_deref(),
        Some("new.rs")
    );
    assert_eq!(mounted.state.workspace.content.get_untracked(), "new file");
    let (send, receive) = futures::channel::oneshot::channel();
    mounted
        .state
        .fake
        .recovery_load_results
        .borrow_mut()
        .push_back(receive);
    mounted.click_text("Restore saved files");
    mounted
        .state
        .ui
        .confirm
        .get_untracked()
        .unwrap()
        .action
        .run(());
    wait_until("replacement load", || {
        recovery
            .projects
            .with_untracked(|projects| projects[&1].phase == RecoveryPhase::Loading)
    })
    .await;
    mounted.state.auth.logout();
    mounted.state.workspace.reset();
    mounted.state.projects.projects_loaded.set(false);
    settle().await;
    send.send(Ok(draft_record(
        &mounted.state.projects.project(1).unwrap(),
    )))
    .unwrap();
    settle().await;
    assert!(mounted.state.workspace.open_file.get_untracked().is_none());
    assert!(
        recovery
            .projects
            .with_untracked(std::collections::HashMap::is_empty)
    );
}

#[wasm_bindgen_test]
async fn editor_recovery_disk_conflict_blocks_host_save_and_retry_keeps_the_draft() {
    use openwebide_frontend::{
        state::workspace::RecoveredFileIssue, state_actions::workspace::WorkspaceActions,
    };
    let slot = std::rc::Rc::new(std::cell::Cell::new(None));
    let capture = slot.clone();
    let mounted = mount_test(move |state| {
        state.seed_project();
        sign_in_recovery(&state);
        state
            .fake
            .files
            .borrow_mut()
            .insert((1, "a.rs".into()), "external".into());
        state
            .fake
            .editor_recovery_records
            .borrow_mut()
            .insert(1, draft_record(&state.projects.project(1).unwrap()));
        let view = super::support::recovery_editor_view(state);
        capture.set(Some(expect_context::<WorkspaceActions>()));
        view
    });
    wait_until("disk conflict", || {
        mounted
            .state
            .workspace
            .editor_recovery_checks
            .with_untracked(|checks| {
                checks.get(&(1, "a.rs".into())) == Some(&RecoveredFileIssue::Conflict)
            })
    })
    .await;
    assert_eq!(
        mounted.state.workspace.content.get_untracked(),
        "draft base"
    );
    slot.get().unwrap().on_save.run(());
    settle().await;
    assert_eq!(
        mounted.state.fake.files.borrow()[&(1, "a.rs".into())],
        "external"
    );
    mounted
        .state
        .fake
        .files
        .borrow_mut()
        .insert((1, "a.rs".into()), "base".into());
    mounted.click_text("Check disk again");
    wait_until("verified original baseline", || {
        mounted
            .state
            .workspace
            .editor_recovery_checks
            .with_untracked(std::collections::HashMap::is_empty)
    })
    .await;
    assert_eq!(
        mounted.state.workspace.content.get_untracked(),
        "draft base"
    );
    slot.get().unwrap().on_save.run(());
    wait_until("save after baseline verification", || {
        mounted.state.fake.files.borrow()[&(1, "a.rs".into())] == "draft base"
    })
    .await;
}

#[wasm_bindgen_test]
async fn editor_recovery_changed_root_blocks_saves_without_discarding_the_draft() {
    use openwebide_frontend::{
        state::editor_recovery::{EditorRecoveryState, RecoveryPhase},
        state_actions::workspace::WorkspaceActions,
    };
    let slot = std::rc::Rc::new(std::cell::Cell::new(None));
    let capture = slot.clone();
    let mounted = mount_test(move |state| {
        state.seed_project();
        sign_in_recovery(&state);
        state
            .fake
            .files
            .borrow_mut()
            .insert((1, "a.rs".into()), "base".into());
        state
            .fake
            .editor_recovery_records
            .borrow_mut()
            .insert(1, draft_record(&state.projects.project(1).unwrap()));
        let view = super::support::recovery_editor_view(state);
        capture.set(Some((
            expect_context::<WorkspaceActions>(),
            expect_context::<EditorRecoveryState>(),
        )));
        view
    });
    let (actions, recovery) = slot.get().unwrap();
    wait_until("verified recovered draft", || {
        mounted.state.workspace.content.get_untracked() == "draft base"
            && mounted
                .state
                .workspace
                .editor_recovery_checks
                .with_untracked(std::collections::HashMap::is_empty)
    })
    .await;
    mounted
        .state
        .projects
        .projects
        .update(|projects| projects[0].path = Some("different-root".into()));
    actions.on_save.run(());
    wait_until("changed root pauses database saves", || {
        recovery
            .projects
            .with_untracked(|projects| matches!(projects[&1].phase, RecoveryPhase::Conflict(_)))
    })
    .await;
    assert_eq!(
        mounted.state.workspace.content.get_untracked(),
        "draft base"
    );
    assert_eq!(
        mounted.state.fake.files.borrow()[&(1, "a.rs".into())],
        "base"
    );
    assert_eq!(
        mounted.state.fake.editor_recovery_records.borrow()[&1]
            .state
            .root
            .as_ref()
            .unwrap()
            .path
            .as_deref(),
        Some("test")
    );
}

#[wasm_bindgen_test]
async fn editor_recovery_retains_local_draft_until_folder_access_returns() {
    use openwebide_core::WorkspaceMode;
    use openwebide_frontend::state_actions::workspace::WorkspaceActions;
    let folder = editorConfigFolder().await.unwrap();
    let slot = std::rc::Rc::new(std::cell::Cell::new(None));
    let capture = slot.clone();
    let mounted = mount_test(move |state| {
        state.seed_project();
        sign_in_recovery(&state);
        state
            .projects
            .projects
            .update(|projects| projects[0].mode = WorkspaceMode::Local);
        let project = state.projects.project(1).unwrap();
        let mut record = draft_record(&project);
        let mut document = openwebide_core::editor::Document::new("😀\r\n  tail  ");
        document.replace_selections("draft ", None).unwrap();
        record.state.selected = Some("src/a.rs".into());
        record.state.files[0].path = "src/a.rs".into();
        record.state.files[0].document = Some(document.recovery());
        state
            .fake
            .editor_recovery_records
            .borrow_mut()
            .insert(1, record);
        let view = super::support::recovery_editor_view(state);
        capture.set(Some(expect_context::<WorkspaceActions>()));
        view
    });
    wait_until("draft retained without a directory handle", || {
        mounted.state.workspace.content.get_untracked() == "draft 😀\r\n  tail  "
            && mounted
                .state
                .projects
                .needs_grant
                .with_untracked(|ids| ids.contains(&1))
    })
    .await;
    assert!(
        mounted
            .root
            .text_content()
            .unwrap()
            .contains("Grant folder access")
    );
    let actions = slot.get().unwrap();
    actions.on_save.run(());
    mounted.state.projects.local_handles.update(|handles| {
        handles.insert(1, editorConfigHandle(&folder).unchecked_into());
    });
    mounted.state.projects.needs_grant.update(|ids| {
        ids.remove(&1);
    });
    wait_until("restored handle verifies the original baseline", || {
        mounted
            .state
            .workspace
            .editor_recovery_checks
            .with_untracked(std::collections::HashMap::is_empty)
    })
    .await;
    assert_eq!(
        mounted.state.workspace.content.get_untracked(),
        "draft 😀\r\n  tail  "
    );
    actions.on_save.run(());
    let ws = openwebide_frontend::workspace::Workspace::for_project(
        mounted.state.api,
        mounted.state.projects,
        1,
    )
    .unwrap();
    // Save applies the file's actual nested EditorConfig policies.
    wait_until("saving recovered file after access returns", || {
        !mounted.state.workspace.dirty.get_untracked()
    })
    .await;
    assert_eq!(ws.read("src/a.rs").await.unwrap(), "draft 😀\n  tail\n");
    editorConfigCleanup(&folder).await.unwrap();
}

#[wasm_bindgen_test]
async fn recovered_file_review_reload_and_overwrite_share_both_real_adapters() {
    use openwebide_core::{
        WorkspaceMode,
        editor::{Document, Selection},
    };
    use openwebide_frontend::state_actions::{
        editor::EditorActions, editor_recovery::RecoveryActions,
    };
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let folder = if mode == WorkspaceMode::Local {
            Some(editorConfigFolder().await.unwrap())
        } else {
            None
        };
        let handle = folder.as_ref().map(editorConfigHandle);
        let slot = std::rc::Rc::new(std::cell::Cell::new(None));
        let capture = slot.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            sign_in_recovery(&state);
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            if let Some(handle) = handle {
                state.projects.local_handles.update(|handles| {
                    handles.insert(1, handle.unchecked_into());
                });
            }
            state.fake.files.borrow_mut().extend([
                ((1, "src/a.rs".into()), "external\n".into()),
                ((1, "src/.editorconfig".into()), "[*.rs]\nend_of_line=lf\ninsert_final_newline=true\ntrim_trailing_whitespace=true\n".into()),
            ]);
            let mut record = draft_record(&state.projects.project(1).unwrap());
            let mut document = Document::new("original");
            document.replace_selections("draft ", None).unwrap();
            record.state.files[0].document = Some(document.recovery());
            record.state.files[0].path = "src/a.rs".into();
            record.state.selected = Some("src/a.rs".into());
            state
                .fake
                .editor_recovery_records
                .borrow_mut()
                .insert(1, record);
            let view = super::support::recovery_editor_view(state);
            capture.set(Some(expect_context::<RecoveryActions>()));
            view
        });
        let ws = openwebide_frontend::workspace::Workspace::for_project(
            mounted.state.api,
            mounted.state.projects,
            1,
        )
        .unwrap();
        if mode == WorkspaceMode::Local {
            ws.write("src/a.rs", "external\n").await.unwrap();
        }
        wait_until("reviewable recovered conflict", || {
            mounted
                .root
                .text_content()
                .unwrap()
                .contains("Review recovered file")
        })
        .await;
        mounted.click_text("Review recovered file");
        wait_until("recovered diff dialog", || {
            mounted.root.query_selector(".modal").unwrap().is_some()
        })
        .await;
        assert!(mounted.root.text_content().unwrap().contains("external"));
        assert!(
            mounted
                .root
                .text_content()
                .unwrap()
                .contains("draft original")
        );
        mounted.click_text("Cancel");
        settle().await;
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            "draft original"
        );
        assert_eq!(ws.read("src/a.rs").await.unwrap(), "external\n");
        mounted.click_text("Review recovered file");
        wait_until("reload choice", || {
            mounted.root.query_selector(".modal").unwrap().is_some()
        })
        .await;
        mounted.click_text("Reload disk");
        wait_until("disk reloaded into editor", || {
            mounted.state.workspace.content.get_untracked() == "external\n"
        })
        .await;
        assert!(!mounted.state.workspace.dirty.get_untracked());
        let editor = EditorActions::new(mounted.state.workspace);
        editor
            .native_input(
                "replacement  ".into(),
                Selection::caret(13),
                "insertText",
                1.0,
            )
            .unwrap();
        ws.write("src/a.rs", "new disk\n").await.unwrap();
        let actions = slot.get().unwrap();
        actions.check_files.run(1);
        // Check files checks only pending recovered files; request another normal
        // Save to detect an external edit against the newly loaded baseline.
        mounted
            .state
            .workspace
            .editor_recovery_checks
            .update(|checks| {
                checks.insert(
                    (1, "src/a.rs".into()),
                    openwebide_frontend::state::workspace::RecoveredFileIssue::Pending,
                );
            });
        wait_until("second conflict", || {
            mounted
                .root
                .text_content()
                .unwrap()
                .contains("Review recovered file")
        })
        .await;
        mounted.click_text("Review recovered file");
        wait_until("overwrite review", || {
            mounted.root.query_selector(".modal").unwrap().is_some()
        })
        .await;
        mounted.click_text("Save draft");
        wait_until("draft saved through normal save policy", || {
            !mounted.state.workspace.dirty.get_untracked()
        })
        .await;
        assert_eq!(ws.read("src/a.rs").await.unwrap(), "replacement\n");
        assert!(
            mounted
                .state
                .workspace
                .editor_recovery_overwrites
                .with_untracked(std::collections::HashMap::is_empty)
        );
        // Missing and empty recovered files also require the explicit save action.
        ws.delete("src/a.rs").await.unwrap();
        editor
            .native_input("".into(), Selection::caret(0), "deleteContentBackward", 2.0)
            .unwrap();
        mounted
            .state
            .workspace
            .editor_recovery_checks
            .update(|checks| {
                checks.insert(
                    (1, "src/a.rs".into()),
                    openwebide_frontend::state::workspace::RecoveredFileIssue::Pending,
                );
            });
        wait_until("missing file review", || {
            mounted
                .root
                .text_content()
                .unwrap()
                .contains("Review recovered file")
        })
        .await;
        mounted.click_text("Review recovered file");
        wait_until("recreate review", || {
            mounted.root.query_selector(".modal").unwrap().is_some()
        })
        .await;
        mounted.click_text("Save draft");
        wait_until("empty draft recreated", || {
            mounted.root.query_selector(".modal").unwrap().is_none()
                && mounted
                    .state
                    .workspace
                    .editor_recovery_overwrites
                    .with_untracked(std::collections::HashMap::is_empty)
        })
        .await;
        // The shared save policy preserves genuinely empty files.
        for _ in 0..20 {
            if ws.read_optional_bytes("src/a.rs").await.unwrap().is_some() {
                break;
            }
            settle().await;
        }
        assert_eq!(ws.read("src/a.rs").await.unwrap(), "");
        if let Some(folder) = folder {
            editorConfigCleanup(&folder).await.unwrap();
        }
    }
}

#[wasm_bindgen_test]
async fn recovered_file_review_rejects_changed_disk_and_editor_and_cancelled_actions() {
    use openwebide_core::{WorkspaceMode, editor::Selection};
    use openwebide_frontend::state_actions::{
        editor::EditorActions, editor_recovery::RecoveryActions,
    };
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let folder = if mode == WorkspaceMode::Local {
            Some(editorConfigFolder().await.unwrap())
        } else {
            None
        };
        let handle = folder.as_ref().map(editorConfigHandle);
        let slot = std::rc::Rc::new(std::cell::Cell::new(None));
        let capture = slot.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            sign_in_recovery(&state);
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            if let Some(handle) = handle {
                state.projects.local_handles.update(|handles| {
                    handles.insert(1, handle.unchecked_into());
                });
            }
            let mut record = draft_record(&state.projects.project(1).unwrap());
            record.state.files[0].path = "src/a.rs".into();
            record.state.selected = Some("src/a.rs".into());
            state
                .fake
                .files
                .borrow_mut()
                .insert((1, "src/a.rs".into()), "first disk".into());
            state
                .fake
                .editor_recovery_records
                .borrow_mut()
                .insert(1, record);
            let view = super::support::recovery_editor_view(state);
            capture.set(Some(expect_context::<RecoveryActions>()));
            view
        });
        wait_until("conflicting draft ready", || {
            mounted
                .root
                .text_content()
                .unwrap()
                .contains("Review recovered file")
        })
        .await;
        let ws = openwebide_frontend::workspace::Workspace::for_project(
            mounted.state.api,
            mounted.state.projects,
            1,
        )
        .unwrap();
        ws.write("src/a.rs", "first disk").await.unwrap();
        mounted.click_text("Review recovered file");
        wait_until("first review", || {
            mounted.root.query_selector(".modal").unwrap().is_some()
        })
        .await;
        ws.write("src/a.rs", "later disk").await.unwrap();
        mounted.click_text("Save draft");
        wait_until("changed disk rejects approval", || {
            mounted.root.query_selector(".modal").unwrap().is_none()
        })
        .await;
        assert_eq!(ws.read("src/a.rs").await.unwrap(), "later disk");
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            "draft base"
        );
        mounted.click_text("Review recovered file");
        wait_until("second review", || {
            mounted.root.query_selector(".modal").unwrap().is_some()
        })
        .await;
        EditorActions::new(mounted.state.workspace)
            .native_input(
                "newer draft".into(),
                Selection::caret(11),
                "insertText",
                1.0,
            )
            .unwrap();
        mounted.click_text("Reload disk");
        settle().await;
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            "newer draft"
        );
        assert_eq!(ws.read("src/a.rs").await.unwrap(), "later disk");
        mounted.click_text("Review recovered file");
        wait_until("cancel review", || {
            mounted.root.query_selector(".modal").unwrap().is_some()
        })
        .await;
        mounted.click_text("Cancel");
        slot.get().unwrap().overwrite_file.run(());
        settle().await;
        assert_eq!(ws.read("src/a.rs").await.unwrap(), "later disk");
        assert!(
            mounted
                .state
                .workspace
                .editor_recovery_overwrites
                .with_untracked(std::collections::HashMap::is_empty)
        );
        if let Some(folder) = folder {
            editorConfigCleanup(&folder).await.unwrap();
        }
    }
}

#[wasm_bindgen_test]
async fn recovered_file_review_recreates_a_missing_clean_empty_file_in_both_modes() {
    use openwebide_core::{WorkspaceMode, editor::Document};
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let folder = if mode == WorkspaceMode::Local {
            Some(editorConfigFolder().await.unwrap())
        } else {
            None
        };
        let handle = folder.as_ref().map(editorConfigHandle);
        if let Some(handle) = &handle {
            openwebide_frontend::workspace::Workspace::Local {
                handle: handle.clone().unchecked_into(),
            }
            .delete("src/a.rs")
            .await
            .unwrap();
        }
        let mounted = mount_test(move |state| {
            state.seed_project();
            sign_in_recovery(&state);
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            if let Some(handle) = handle {
                state.projects.local_handles.update(|handles| {
                    handles.insert(1, handle.unchecked_into());
                });
            }
            let mut record = draft_record(&state.projects.project(1).unwrap());
            record.state.files[0].path = "src/a.rs".into();
            record.state.selected = Some("src/a.rs".into());
            record.state.files[0].document = Some(Document::new("").recovery());
            state
                .fake
                .editor_recovery_records
                .borrow_mut()
                .insert(1, record);
            super::support::recovery_editor_view(state)
        });
        wait_until("missing clean file preserved", || {
            mounted
                .root
                .text_content()
                .unwrap()
                .contains("Review recovered file")
        })
        .await;
        assert!(!mounted.state.workspace.dirty.get_untracked());
        mounted.click_text("Review recovered file");
        wait_until("empty-file review", || {
            mounted.root.query_selector(".modal").unwrap().is_some()
        })
        .await;
        mounted.click_text("Save draft");
        wait_until("recreation approval consumed", || {
            mounted.root.query_selector(".modal").unwrap().is_none()
                && mounted
                    .state
                    .workspace
                    .editor_recovery_overwrites
                    .with_untracked(std::collections::HashMap::is_empty)
        })
        .await;
        let ws = openwebide_frontend::workspace::Workspace::for_project(
            mounted.state.api,
            mounted.state.projects,
            1,
        )
        .unwrap();
        for _ in 0..40 {
            if ws.read_optional_bytes("src/a.rs").await.unwrap().is_some() {
                break;
            }
            settle().await;
        }
        assert_eq!(
            ws.read_optional_bytes("src/a.rs").await.unwrap(),
            Some(Vec::new())
        );
        if let Some(folder) = folder {
            editorConfigCleanup(&folder).await.unwrap();
        }
    }
}

#[wasm_bindgen_test]
async fn diff_syntax_paint_keeps_word_changes_and_embedded_context_in_both_modes() {
    use openwebide_core::WorkspaceMode;
    use openwebide_frontend::state::git::HeadContent;
    let old = "<script>\r\n/*\r\nold café\r\n*/\r\nconst value = call(41);\r\n</script>\r\n<style>p { color: red; }</style>\r\n";
    let new = "<script>\r\n/*\r\nnew 😀\r\n*/\r\nconst value = call(42);\r\n</script>\r\n<style>p { color: blue; }</style>\r\n";
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("fixture.html".into()));
            state.workspace.content.set(new.into());
            state.git.head_content.set(Some(HeadContent {
                project_id: Some(1),
                path: "fixture.html".into(),
                content: Ok(old.into()),
            }));
            view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:600px;height:400px">{editor_view(state)}</div> }
        });
        settle().await;
        mounted.click_text("Diff HEAD");
        settle().await;
        assert_eq!(
            mounted
                .element(".diff-line.add[data-line='3'] .editor-line-text")
                .text_content()
                .unwrap(),
            "new 😀"
        );
        assert!(
            mounted
                .root
                .query_selector(".diff-line.add[data-line='3'] .tok-comment.diff-word-add")
                .unwrap()
                .is_some()
        );
        assert_eq!(
            mounted
                .element(".diff-line.add[data-line='5'] .tok-function")
                .text_content()
                .unwrap(),
            "call"
        );
        assert!(
            mounted
                .root
                .query_selector(".diff-line.add[data-line='5'] .tok-number.diff-word-add")
                .unwrap()
                .is_some()
        );
        assert_eq!(
            mounted
                .element(".diff-line.add[data-line='7'] .tok-attribute")
                .text_content()
                .unwrap(),
            "color"
        );
        // Source tags remain text, rather than becoming executable preview DOM.
        assert!(
            mounted
                .root
                .query_selector(".editor-diff script")
                .unwrap()
                .is_none()
        );
        mounted.click_text("Split");
        settle().await;
        let panes = mounted.root.query_selector_all(".sbs-pane").unwrap();
        for (index, word) in [(0, "old café"), (1, "new 😀")] {
            let pane: web_sys::Element = panes.item(index).unwrap().unchecked_into();
            assert_eq!(
                pane.query_selector(".sbs-cell[data-line='3'] .editor-line-text")
                    .unwrap()
                    .unwrap()
                    .text_content()
                    .unwrap(),
                word
            );
            assert!(
                pane.query_selector(".sbs-cell[data-line='3'] .tok-comment")
                    .unwrap()
                    .is_some()
            );
            assert_eq!(
                pane.query_selector(".sbs-cell[data-line='5'] .tok-function")
                    .unwrap()
                    .unwrap()
                    .text_content()
                    .unwrap(),
                "call"
            );
        }
        assert_eq!(mounted.state.workspace.content.get_untracked(), new);
    }
}

#[wasm_bindgen_test]
async fn wrapped_multi_cursor_arrows_follow_measured_rows_in_both_modes() {
    use openwebide_core::{
        WorkspaceMode,
        editor::{Selection, byte_to_textarea, line_column},
    };
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let line = format!("\tlet café = \"{}\";", "文😀 words ".repeat(12));
        let source = format!("{line}\r\nx\r\n{line}\r\n");
        let third = line.len() + 5;
        let expected = source.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("visual.rs".into()));
            state.workspace.content.set(source.clone());
            state
                .settings
                .editor_preferences
                .update(|preferences| preferences.word_wrap = true);
            view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:340px;height:380px">{editor_view(state)}</div> }
        });
        wait_until("wrapped paint ready", || {
            mounted
                .root
                .query_selector(".editor-code.highlight-ready")
                .unwrap()
                .is_some()
        })
        .await;
        editorFontsReady().await.unwrap();
        frame().await;
        let actions = EditorActions::new(mounted.state.workspace);
        let textarea: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        actions.record_selection(Selection::caret(5)).unwrap();
        actions
            .toggle_cursor(1, "visual.rs", &expected, third + 5)
            .unwrap();
        let primary = actions.selections(&expected)[0].head;
        let utf16 = u32::try_from(byte_to_textarea(&expected, primary).unwrap()).unwrap();
        textarea.set_selection_range(utf16, utf16).unwrap();
        let before = actions.selections(&expected);
        assert!(editor_key(&textarea, "ArrowDown", false, false).default_prevented());
        let after = actions.selections(&expected);
        assert_eq!(after.len(), 2);
        for (before, after) in before.iter().zip(&after) {
            assert_eq!(
                line_column(&expected, before.head).0,
                line_column(&expected, after.head).0,
                "first Down must remain inside the same wrapped source line"
            );
            assert!(after.head > before.head);
            assert!(expected.is_char_boundary(after.head));
        }
        assert!(
            mounted
                .root
                .query_selector("[role='alert']")
                .unwrap()
                .is_none()
        );
        wait_until("measured primary and secondary caret paint", || {
            textarea.class_list().contains("editor-visual-carets")
                && mounted
                    .root
                    .query_selector_all(".editor-primary-caret, .editor-secondary-caret")
                    .unwrap()
                    .length()
                    == 2
        })
        .await;
        assert!(editor_key(&textarea, "ArrowUp", false, false).default_prevented());
        assert_eq!(actions.selections(&expected), before);
        assert_eq!(actions.source(), expected);
        assert!(!mounted.state.workspace.dirty.get_untracked());
    }
}

#[wasm_bindgen_test]
async fn measured_multi_cursor_motion_skips_folded_rows_in_both_modes() {
    use openwebide_core::{
        WorkspaceMode,
        editor::{FoldCommand, Selection, byte_to_textarea, line_column},
    };
    use openwebide_frontend::state_actions::editor::EditorActions;
    let source = "fn first() {\r\n    first();\r\n}\r\nfn second() {\r\n    second();\r\n}\r\n";
    let second = source.find("fn second").unwrap();
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("folded.rs".into()));
            state.workspace.content.set(source.into());
            state
                .settings
                .editor_preferences
                .update(|preferences| preferences.word_wrap = true);
            view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:340px;height:380px">{editor_view(state)}</div> }
        });
        wait_until("folded row provider", || {
            mounted
                .root
                .query_selector(".editor-fold-control")
                .unwrap()
                .is_some()
        })
        .await;
        let actions = EditorActions::new(mounted.state.workspace);
        let textarea: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        actions.record_selection(Selection::caret(5)).unwrap();
        actions
            .toggle_cursor(1, "folded.rs", source, second + 5)
            .unwrap();
        let before = actions.selections(source);
        actions.fold_command(FoldCommand::Toggle(0));
        wait_until("collapsed source projection paint", || {
            mounted
                .root
                .query_selector(".editor-source-line[data-line='2']")
                .unwrap()
                .is_none()
                && mounted
                    .element(".editor-highlight-content")
                    .text_content()
                    .unwrap()
                    == textarea.value()
        })
        .await;
        // Native selection coordinates come from the current folded projection.
        let projection = actions.projection().unwrap();
        let visible = projection.visible_selection(before[0]).unwrap();
        let primary =
            u32::try_from(byte_to_textarea(projection.text(), visible.head).unwrap()).unwrap();
        textarea.set_selection_range(primary, primary).unwrap();
        assert!(editor_key(&textarea, "ArrowDown", false, false).default_prevented());
        wait_until("folded downward motion applied", || {
            actions.queued_motion_ticket().is_none()
        })
        .await;
        let after = actions.selections(source);
        assert_eq!(after.len(), 2);
        assert_eq!(line_column(source, after[0].head).0, 5);
        assert_eq!(line_column(source, after[1].head).0, 4);
        assert!(editor_key(&textarea, "ArrowUp", false, false).default_prevented());
        wait_until("folded upward motion applied", || {
            actions.queued_motion_ticket().is_none()
        })
        .await;
        assert_eq!(actions.selections(source), before);
        assert_eq!(actions.source(), source);
        assert!(!mounted.state.workspace.dirty.get_untracked());
    }
}

fn seed_wrapped_carets(mounted: &Mounted, source: &str, second: usize) {
    use openwebide_core::editor::{Selection, byte_to_textarea};
    use openwebide_frontend::state_actions::editor::EditorActions;
    let actions = EditorActions::new(mounted.state.workspace);
    let input: web_sys::HtmlTextAreaElement = mounted.element(".editor-textarea").unchecked_into();
    actions.record_selection(Selection::caret(5)).unwrap();
    actions
        .toggle_cursor(
            1,
            &mounted.state.workspace.open_file.get_untracked().unwrap(),
            source,
            second + 5,
        )
        .unwrap();
    let at = u32::try_from(byte_to_textarea(source, actions.selections(source)[0].head).unwrap())
        .unwrap();
    input.set_selection_range(at, at).unwrap();
}

#[wasm_bindgen_test]
async fn pending_paint_motion_preserves_key_order_and_flushes_before_edits_in_both_modes() {
    use openwebide_core::WorkspaceMode;
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let line = format!("\tlet café = \"{}\";", "文😀 words ".repeat(12));
        let source = format!("{line}\r\nx\r\n{line}\r\n");
        let second = line.len() + 5;
        let expected = source.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("queued.rs".into()));
            state.workspace.content.set(source.clone());
            state
                .settings
                .editor_preferences
                .update(|preferences| preferences.word_wrap = true);
            view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:340px;height:380px">{editor_view(state)}</div> }
        });
        wait_until("initial queued-motion paint", || {
            mounted
                .root
                .query_selector(".editor-code.highlight-ready")
                .unwrap()
                .is_some()
        })
        .await;
        let actions = EditorActions::new(mounted.state.workspace);
        let input: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        seed_wrapped_carets(&mounted, &expected, second);
        for (key, shift) in [
            ("ArrowDown", false),
            ("ArrowDown", false),
            ("ArrowUp", true),
        ] {
            editor_key(&input, key, false, shift);
        }
        let ordered = actions.selections(&expected);
        seed_wrapped_carets(&mounted, &expected, second);
        // A native scrollbar/resize may settle after the observer's last width.
        // Immediate and queued motion must use the same current input geometry.
        mounted
            .element(".editor-highlight")
            .style()
            .set_property(
                "--editor-text-width",
                &format!("{}px", input.client_width() + 24),
            )
            .unwrap();
        for (key, shift) in [
            ("ArrowDown", false),
            ("ArrowDown", false),
            ("ArrowUp", true),
        ] {
            editor_key(&input, key, false, shift);
        }
        assert_eq!(actions.selections(&expected), ordered);
        seed_wrapped_carets(&mounted, &expected, second);
        let before = actions.selections(&expected);
        let projection_revision = actions.projection_revision();
        let code = mounted.element(".editor-code");
        code.class_list().remove_1("highlight-ready").unwrap();
        for (key, shift) in [
            ("ArrowDown", false),
            ("ArrowDown", false),
            ("ArrowUp", true),
        ] {
            assert!(editor_key(&input, key, false, shift).default_prevented());
        }
        assert_eq!(actions.selections(&expected), before);
        assert!(actions.queued_motion_ticket().is_some());
        code.class_list().add_1("highlight-ready").unwrap();
        wait_until("ordered deferred arrows", || {
            actions.queued_motion_ticket().is_none()
        })
        .await;
        assert_eq!(actions.selections(&expected), ordered);
        assert_eq!(actions.projection_revision(), projection_revision);
        assert_eq!(actions.source(), expected);
        assert!(
            mounted
                .root
                .query_selector("[role='alert']")
                .unwrap()
                .is_none()
        );

        seed_wrapped_carets(&mounted, &expected, second);
        editor_key(&input, "ArrowDown", false, false);
        editor_key(&input, "ArrowDown", false, false);
        editor_key(&input, "Enter", false, false);
        let immediate_edit = actions.source();
        assert_ne!(immediate_edit, expected);
        editor_key(&input, "z", true, false);
        wait_until("restored source paint", || {
            mounted
                .element(".editor-highlight-content")
                .text_content()
                .unwrap()
                == input.value()
                && code.class_list().contains("highlight-ready")
        })
        .await;
        assert_eq!(actions.source(), expected);
        seed_wrapped_carets(&mounted, &expected, second);
        code.class_list().remove_1("highlight-ready").unwrap();
        editor_key(&input, "ArrowDown", false, false);
        editor_key(&input, "ArrowDown", false, false);
        assert!(actions.queued_motion_ticket().is_some());
        assert!(editor_key(&input, "Enter", false, false).default_prevented());
        assert_eq!(
            actions.source(),
            immediate_edit,
            "the edit must use the completed queued caret positions"
        );
        assert!(actions.queued_motion_ticket().is_none());
        settle().await;
        frame().await;
        assert_eq!(
            actions.source(),
            immediate_edit,
            "the old frame cannot replay the completed queue"
        );

        // Mobile/native input can arrive without a keydown. Its beforeinput
        // handler must also finish pending navigation before browser mutation.
        editor_key(&input, "z", true, false);
        wait_until("paint before native queued edit", || {
            mounted
                .element(".editor-highlight-content")
                .text_content()
                .unwrap()
                == input.value()
                && code.class_list().contains("highlight-ready")
        })
        .await;
        seed_wrapped_carets(&mounted, &expected, second);
        editor_key(&input, "ArrowDown", false, false);
        let edit_at = actions.selections(&expected);
        let mut reference = openwebide_core::editor::Document::new(expected.clone());
        reference.set_selections(edit_at).unwrap();
        reference.replace_selections("X", None).unwrap();
        seed_wrapped_carets(&mounted, &expected, second);
        code.class_list().remove_1("highlight-ready").unwrap();
        editor_key(&input, "ArrowDown", false, false);
        let init = web_sys::InputEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_input_type("insertText");
        init.set_data(Some("X"));
        let event = web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();
        input.dispatch_event(&event).unwrap();
        assert!(!event.default_prevented());
        editorNativeInput(&input, "X", "insertText", false);
        assert_eq!(actions.source(), reference.text());
        assert!(actions.queued_motion_ticket().is_none());
    }
}

#[wasm_bindgen_test]
async fn queued_motion_cannot_cross_accounts_files_or_newer_tickets_in_both_modes() {
    use openwebide_core::{WorkspaceMode, editor::SelectionMotion};
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let facade = std::rc::Rc::new(std::cell::RefCell::new(None::<EditorActions>));
        let captured = facade.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("queued.rs".into()));
            state.workspace.content.set("one\r\ntwo\r\nthree".into());
            captured.replace(Some(EditorActions::new(state.workspace)));
            editor_view(state)
        });
        settle().await;
        let actions = facade.borrow().expect("facade created within app context");
        let source = actions.source();
        actions
            .record_selection(openwebide_core::editor::Selection::caret(0))
            .unwrap();
        let old = actions
            .queue_motion(1, "queued.rs", &source, SelectionMotion::Down, false)
            .unwrap()
            .unwrap()
            .0;
        mounted
            .state
            .auth
            .generation
            .update(|generation| *generation += 1);
        assert!(actions.next_queued_motion(old).is_none());
        assert_eq!(actions.selection(&source).unwrap().head, 0);
        let old = actions
            .queue_motion(1, "queued.rs", &source, SelectionMotion::Down, false)
            .unwrap()
            .unwrap()
            .0;
        mounted
            .state
            .workspace
            .open_file
            .set(Some("other.rs".into()));
        assert!(actions.next_queued_motion(old).is_none());
        mounted
            .state
            .workspace
            .open_file
            .set(Some("queued.rs".into()));
        let new = actions
            .queue_motion(1, "queued.rs", &source, SelectionMotion::Right, false)
            .unwrap()
            .unwrap()
            .0;
        assert_ne!(new, old);
        assert!(actions.apply_queued_motion(old, None).unwrap().is_none());
        assert_eq!(actions.queued_motion_ticket(), Some(new));
        actions.apply_queued_motion(new, None).unwrap();
        assert_eq!(actions.selection(&source).unwrap().head, 1);
        assert_eq!(actions.source(), source);
        assert!(!mounted.state.workspace.dirty.get_untracked());
    }
}

#[wasm_bindgen_test]
async fn literal_contexts_and_paint_preserve_heredocs_and_nested_interpolation_in_both_modes() {
    use openwebide_core::{WorkspaceMode, editor::syntax_contracts::LITERAL_CASES};
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            editor_view(state)
        });
        let actions = EditorActions::new(mounted.state.workspace);
        for &(path, source, literal, code) in LITERAL_CASES {
            let source = source.replace('\n', "\r\n");
            mounted.state.workspace.open_file.set(Some(path.into()));
            mounted.state.workspace.content.set(source.clone());
            let structure = actions.syntax_structure(|| true).unwrap();
            assert!(
                !structure.is_code(source.find(literal).unwrap()),
                "{mode:?} {path}"
            );
            if let Some(code) = code {
                assert!(
                    structure.is_code(source.find(code).unwrap()),
                    "{mode:?} {path}"
                );
            }
            if let Some(inner) = source.find("inner") {
                assert!(!structure.is_code(inner), "{mode:?} {path}");
            }
            wait_until("literal paint", || {
                let Ok(Some(paint)) = mounted.root.query_selector(".editor-highlight-content")
                else {
                    return false;
                };
                paint
                    .text_content()
                    .is_some_and(|text| text == source.replace("\r\n", "\n"))
            })
            .await;
            let tokens = mounted.root.query_selector_all(".tok-string").unwrap();
            assert!(
                (0..tokens.length()).any(|index| tokens
                    .item(index)
                    .unwrap()
                    .text_content()
                    .is_some_and(|text| text.contains(literal))),
                "literal paint: {mode:?} {path} {source}"
            );
        }
    }
}

#[wasm_bindgen_test]
async fn long_wrapped_lines_move_cursors_without_measuring_the_entire_line_in_both_modes() {
    use openwebide_core::{
        WorkspaceMode,
        editor::{Selection, byte_to_textarea},
    };
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let source = format!("{}\r\nshort\r\n", "文😀e\u{301} words ".repeat(20_000));
        let expected = source.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("long.txt".into()));
            state.workspace.content.set(source.clone());
            state
                .settings
                .editor_preferences
                .update(|preferences| preferences.word_wrap = true);
            view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:340px;height:380px">{editor_view(state)}</div> }
        });
        wait_until("long wrapped paint", || {
            mounted
                .root
                .query_selector(".editor-code.highlight-ready")
                .unwrap()
                .is_some()
        })
        .await;
        frame().await;
        wait_until("bounded long-row paint after font loading", || {
            let runs = mounted.root.query_selector_all(".editor-text-run").unwrap();
            runs.length() > 0
                && runs.length() < 100
                && mounted
                    .element(".editor-highlight-content")
                    .text_content()
                    .unwrap()
                    .len()
                    < 16_384
                && mounted
                    .element(".editor-source-line")
                    .has_attribute("data-paint-top")
        })
        .await;
        let runs = mounted.root.query_selector_all(".editor-text-run").unwrap();
        assert!(runs.length() > 0 && runs.length() < 100);
        let painted = mounted.element(".editor-highlight-content");
        assert!(painted.text_content().unwrap().len() < 16_384);
        assert!(
            mounted
                .element(".editor-source-line")
                .has_attribute("data-paint-top")
        );
        for index in 0..runs.length() {
            assert!(runs.item(index).unwrap().text_content().unwrap().len() <= 516);
        }
        let actions = EditorActions::new(mounted.state.workspace);
        let input: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        let first = "文😀e\u{301} words ".len() * 15_000 + 3;
        let second = "文😀e\u{301} words ".len() * 18_000 + 3;
        actions.record_selection(Selection::caret(first)).unwrap();
        actions
            .toggle_cursor(1, "long.txt", &expected, second)
            .unwrap();
        let at = u32::try_from(
            byte_to_textarea(&expected, actions.selections(&expected)[0].head).unwrap(),
        )
        .unwrap();
        input.set_selection_range(at, at).unwrap();
        let before = actions.selections(&expected);
        assert_eq!(before.len(), 2);
        let measured =
            wasm_bindgen_futures::JsFuture::from(measure_wrapped_key(&input, "ArrowDown"))
                .await
                .unwrap();
        let measured = js_sys::Array::from(&measured);
        assert_eq!(measured.get(0).as_bool(), Some(true));
        let count = measured.get(1).as_f64().unwrap();
        assert!(count < 2_000.0, "{mode:?}: {count} range measurements");
        wasm_bindgen_test::console_log!(
            "{mode:?}: long-line Down measured {count} ranges in {}ms",
            measured.get(2).as_f64().unwrap()
        );
        let after = actions.selections(&expected);
        assert_eq!(after.len(), 2);
        for (before, after) in before.iter().zip(after.iter()) {
            assert!(after.head > before.head && after.head - before.head < 200);
            assert!(expected.is_char_boundary(after.head));
        }
        assert!(editor_key(&input, "ArrowUp", false, false).default_prevented());
        wait_until("queued long-line Up", || {
            actions.selections(&expected) == before
        })
        .await;
        assert_eq!(actions.selections(&expected), before);
        assert_eq!(actions.source(), expected);
        assert!(!mounted.state.workspace.dirty.get_untracked());
        assert!(
            mounted
                .root
                .query_selector("[role='alert']")
                .unwrap()
                .is_none()
        );
    }
}

#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
export async function measureWrappedKey(input, key) {
    const original = Range.prototype.getClientRects;
    let count = 0;
    Range.prototype.getClientRects = function() { ++count; return original.call(this); };
    const event = new KeyboardEvent('keydown', {key, bubbles:true, cancelable:true});
    const start = performance.now();
    try {
        const before = input.selectionStart;
        input.dispatchEvent(event);
        for (let frame = 0; input.selectionStart === before && frame < 60; ++frame) {
            await new Promise(resolve => requestAnimationFrame(resolve));
        }
        return [event.defaultPrevented, count, performance.now() - start];
    } finally { Range.prototype.getClientRects = original; }
}
"#)]
extern "C" {
    #[wasm_bindgen(js_name = measureWrappedKey)]
    fn measure_wrapped_key(input: &web_sys::HtmlTextAreaElement, key: &str) -> js_sys::Promise;
}

#[wasm_bindgen_test]
async fn cooperative_terminal_lexical_paint_preserves_context_and_rejects_stale_scopes_in_both_modes()
 {
    use openwebide_core::{
        WorkspaceMode,
        highlight::{Language, TokenKind, highlight_lines},
    };
    use openwebide_frontend::state_actions::editor::EditorActions;
    // Stay inside interactive line limits while exceeding the parser's source cap.
    let row = format!("inside {}文😀\r\n", "words ".repeat(70));
    let source = format!("/*\r\n{}*/\r\nlet done = 1;", row.repeat(6000));
    assert!(source.len() > openwebide_core::editor::MAX_STRUCTURE_BYTES);
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        for terminal in [false, true] {
            let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
            let mounted = mount_test({
                let source = source.clone();
                let slot = slot.clone();
                move |state| {
                    state.seed_project();
                    state
                        .projects
                        .projects
                        .update(|projects| projects[0].mode = mode);
                    state.workspace.open_file.set(Some("fallback.rs".into()));
                    state.workspace.content.set(source.clone());
                    if terminal {
                        state.workspace.editor_worker_active.set(true);
                        state.workspace.editor_preparation.set(Some(
                            openwebide_frontend::state::workspace::PreparedEditorSyntax {
                                scope: openwebide_frontend::state::workspace::EditorSyntaxScope {
                                    key: (1, "fallback.rs".into()),
                                    source: source.into(),
                                    epoch: 0,
                                    read_revision: state
                                        .workspace
                                        .editor_read_revision
                                        .get_untracked(),
                                    account_generation: state.auth.generation.get_untracked(),
                                    tab_width: 4,
                                },
                                status: openwebide_core::editor::SyntaxStatus::TooLarge,
                                analysis: None,
                            },
                        ));
                    }
                    let actions = EditorActions::new(state.workspace);
                    actions.install_syntax_worker();
                    *slot.borrow_mut() = Some(actions);
                    view! { <div/> }
                }
            });
            let actions = slot.borrow_mut().take().unwrap();
            wait_until("fallback job becomes pending", || {
                actions.syntax_is_pending()
            })
            .await;
            assert!(
                !actions.full_row_paint_ready(),
                "fallback must resolve before full-row probes allocate neutral paint"
            );
            assert!(
                actions.syntax_paint().1.is_empty(),
                "pending fallback borrows source instead of lexing synchronously"
            );
            wait_until(
                "cooperative lexical job publishes complete contextual rows",
                || {
                    !actions.syntax_is_pending()
                        && mounted
                            .state
                            .workspace
                            .editor_fallback_paint
                            .get_untracked()
                            .is_some()
                },
            )
            .await;
            let painted = actions.syntax_paint();
            assert!(!painted.0);
            assert_eq!(
                *painted.1,
                openwebide_core::highlight::share_token_rows(highlight_lines(
                    &source.replace("\r\n", "\n"),
                    Language::Rust
                ))
            );
            assert_eq!(painted.1[1][0].kind, TokenKind::Comment);
            assert!(
                std::sync::Arc::ptr_eq(&painted.1, &actions.syntax_paint().1),
                "unchanged fallback consumers share completed paint"
            );
            if terminal {
                assert!(
                    std::sync::Arc::ptr_eq(
                        &mounted
                            .state
                            .workspace
                            .editor_preparation
                            .get_untracked()
                            .unwrap()
                            .scope
                            .source,
                        &mounted
                            .state
                            .workspace
                            .editor_fallback_paint
                            .get_untracked()
                            .unwrap()
                            .scope
                            .source,
                    ),
                    "terminal fallback reuses immutable worker source"
                );
                mounted.state.workspace.editor_worker_active.set(false);
            }
            let revised = source.replacen("inside words", "inside revised words", 1);
            mounted.state.workspace.content.set(revised.clone());
            wait_until("one-row edit reuses converged lexical context", || {
                mounted
                    .state
                    .workspace
                    .editor_fallback_paint
                    .with_untracked(|paint| {
                        paint
                            .as_ref()
                            .is_some_and(|paint| paint.scope.source.as_ref() == revised)
                    })
            })
            .await;
            assert_eq!(
                mounted
                    .state
                    .workspace
                    .editor_fallback_paint
                    .get_untracked()
                    .unwrap()
                    .lexical
                    .unwrap()
                    .retokenized_rows(),
                1
            );
            let revised_paint = actions.syntax_paint().1;
            assert!(
                std::sync::Arc::ptr_eq(&painted.1[2], &revised_paint[2]),
                "unchanged rows share token strings across published source revisions"
            );
            assert!(!std::sync::Arc::ptr_eq(&painted.1[1], &revised_paint[1]));
            mounted
                .state
                .workspace
                .editor_read_revision
                .update(|read| *read += 1);
            wait_until("new read cannot reuse prior lexical snapshot", || {
                mounted
                    .state
                    .workspace
                    .editor_fallback_paint
                    .with_untracked(|paint| {
                        paint.as_ref().is_some_and(|paint| {
                            paint.scope.read_revision
                                == mounted.state.workspace.editor_read_revision.get_untracked()
                        })
                    })
            })
            .await;
            assert_eq!(
                mounted
                    .state
                    .workspace
                    .editor_fallback_paint
                    .get_untracked()
                    .unwrap()
                    .lexical
                    .unwrap()
                    .retokenized_rows(),
                revised.split('\n').count()
            );
            mounted
                .state
                .workspace
                .content
                .set(format!("{source}\nold pending"));
            wait_until("replacement starts pending fallback", || {
                actions.syntax_is_pending()
            })
            .await;
            mounted
                .state
                .auth
                .generation
                .update(|generation| *generation += 1);
            mounted
                .state
                .workspace
                .content
                .set("/* new scope */ let revised = true;\r\n".into());
            wait_until(
                &format!(
                    "new source/account owns paint after cancellation: {mode:?} terminal={terminal}"
                ),
                || {
                    mounted
                        .state
                        .workspace
                        .editor_fallback_paint
                        .get_untracked()
                        .is_some_and(|paint| {
                            paint.scope.account_generation
                                == mounted.state.auth.generation.get_untracked()
                                && paint.scope.source.starts_with("/* new scope */")
                        })
                },
            )
            .await;
            let current = actions.syntax_paint();
            assert!(
                current.1.len() < 5,
                "old lexical rows must never replace current source"
            );
            for _ in 0..3 {
                frame().await;
            }
            assert!(
                mounted
                    .state
                    .workspace
                    .editor_fallback_paint
                    .get_untracked()
                    .unwrap()
                    .scope
                    .source
                    .starts_with("/* new scope */")
            );
        }
    }
}

#[wasm_bindgen_test]
async fn syntax_consumers_share_immutable_preparation_and_invalidate_it_in_both_modes() {
    use openwebide_core::{WorkspaceMode, editor::SyntaxStatus};
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let source = "fn main() {\r\n    call(\"文😀\");\r\n}\r\n";
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("prepared.rs".into()));
            state.workspace.content.set(source.into());
            editor_view(state)
        });
        let actions = EditorActions::new(mounted.state.workspace);
        let first = actions.syntax_structure(|| true).unwrap();
        let first_paint = actions.syntax_highlights().unwrap();
        actions.syntax_folds(|| true).unwrap();
        assert!(std::sync::Arc::ptr_eq(
            &first,
            &actions.syntax_structure(|| true).unwrap()
        ));
        assert!(std::sync::Arc::ptr_eq(
            &first_paint,
            &actions.syntax_highlights().unwrap()
        ));
        let revised = source.replace("文😀", "😀 changed");
        mounted.state.workspace.content.set(revised.clone());
        let next = actions.syntax_structure(|| true).unwrap();
        assert!(!std::sync::Arc::ptr_eq(&first, &next));
        assert!(first.matches_source(source));
        assert!(!first.matches_source(&revised));
        assert!(next.matches_source(&revised));
        assert_eq!(
            actions.syntax_folds(|| false).unwrap(),
            (SyntaxStatus::Cancelled, vec![])
        );
        assert!(!std::sync::Arc::ptr_eq(
            &next,
            &actions.syntax_structure(|| true).unwrap()
        ));
        mounted
            .state
            .workspace
            .open_file
            .set(Some("other.txt".into()));
        mounted.state.workspace.content.set("unrelated".into());
        assert!(actions.syntax_structure(|| true).is_none());
        assert!(first.matches_source(source));
        mounted.state.workspace.reset();
        assert!(actions.syntax_highlights().is_none());
    }
}

#[wasm_bindgen_test]
async fn syntax_preparation_cannot_publish_after_scope_changes_in_both_modes() {
    use openwebide_core::{WorkspaceMode, editor::Indentation};
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let facade = std::rc::Rc::new(std::cell::RefCell::new(None::<EditorActions>));
        let captured = facade.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("scope.rs".into()));
            state
                .workspace
                .content
                .set("fn main() {\n    call();\n}".into());
            *captured.borrow_mut() = Some(EditorActions::new(state.workspace));
            editor_view(state)
        });
        let actions = facade.borrow().unwrap();
        for mutation in 0..4 {
            mounted.state.workspace.editor_indentation.update(|values| {
                values.remove(&(1, "scope.rs".into()));
            });
            actions.refresh_fold_ranges(|| true).unwrap();
            let before = actions.fold_state();
            let mut mutated = false;
            let result = actions.refresh_fold_ranges(|| {
                if !mutated {
                    mutated = true;
                    match mutation {
                        0 => mounted
                            .state
                            .workspace
                            .editor_read_revision
                            .update(|revision| *revision += 1),
                        1 => mounted
                            .state
                            .auth
                            .generation
                            .update(|generation| *generation += 1),
                        2 => mounted
                            .state
                            .workspace
                            .pending_epoch
                            .update(|epoch| *epoch += 1),
                        _ => mounted.state.workspace.editor_indentation.update(|values| {
                            values.insert(
                                (1, "scope.rs".into()),
                                Indentation {
                                    tab_width: 8,
                                    ..Indentation::default()
                                },
                            );
                        }),
                    }
                }
                true
            });
            assert!(result.is_none(), "{mode:?} mutation {mutation}");
            assert_eq!(actions.fold_state(), before);
        }
    }
}

struct DeferredSyntaxReply {
    message: String,
    sender: futures::channel::oneshot::Sender<
        Result<String, openwebide_frontend::editor_worker::WorkerError>,
    >,
}
#[derive(Default)]
struct DeferredSyntax {
    pending: std::cell::RefCell<std::collections::VecDeque<DeferredSyntaxReply>>,
    service: std::cell::RefCell<openwebide_core::editor::SyntaxPreparations<String>>,
    calls: std::cell::Cell<usize>,
    stopped: std::cell::Cell<bool>,
    source_delta: std::cell::Cell<bool>,
    sources: std::cell::RefCell<std::collections::HashMap<String, String>>,
}
impl openwebide_frontend::editor_worker::SyntaxTransport for DeferredSyntax {
    fn request(
        &self,
        _ticket: u32,
        message: String,
    ) -> futures::future::LocalBoxFuture<
        '_,
        Result<String, openwebide_frontend::editor_worker::WorkerError>,
    > {
        let (sender, receiver) = futures::channel::oneshot::channel();
        self.calls.set(self.calls.get() + 1);
        self.pending
            .borrow_mut()
            .push_back(DeferredSyntaxReply { message, sender });
        Box::pin(async move {
            receiver.await.unwrap_or(Err(
                openwebide_frontend::editor_worker::WorkerError::Unavailable,
            ))
        })
    }
    fn stop(&self) {
        self.stopped.set(true);
        for reply in self.pending.borrow_mut().drain(..) {
            let _ = reply.sender.send(Err(
                openwebide_frontend::editor_worker::WorkerError::Unavailable,
            ));
        }
    }
}
impl DeferredSyntax {
    fn respond(&self, valid: bool) {
        let DeferredSyntaxReply { message, sender } =
            self.pending.borrow_mut().pop_front().unwrap();
        let output = if valid {
            Ok(self
                .service
                .borrow_mut()
                .handle_message(&message, || true)
                .unwrap())
        } else {
            Err(openwebide_frontend::editor_worker::WorkerError::Transport)
        };
        if output.as_ref().is_ok_and(|message| {
            let value: serde_json::Value = serde_json::from_str(message).unwrap();
            !value["analysis"].is_null()
        }) {
            let request: openwebide_core::editor::SyntaxRequest =
                serde_json::from_str(&message).unwrap();
            let source = request
                .source
                .resolve(
                    self.sources
                        .borrow()
                        .get(&request.document)
                        .map(String::as_str),
                )
                .unwrap();
            self.sources.borrow_mut().insert(request.document, source);
        }
        self.source_delta.set(output.as_ref().is_ok_and(|message| {
            let value: serde_json::Value = serde_json::from_str(message).unwrap();
            value["analysis"]["source"].is_object()
        }));
        sender.send(output).unwrap();
    }
    fn source(&self) -> String {
        let request = serde_json::from_str::<openwebide_core::editor::SyntaxRequest>(
            &self.pending.borrow()[0].message,
        )
        .unwrap();
        request
            .source
            .resolve(
                self.sources
                    .borrow()
                    .get(&request.document)
                    .map(String::as_str),
            )
            .unwrap()
    }
}

#[wasm_bindgen_test]
async fn pending_worker_paints_requested_source_rows_without_full_file_lexical_tokens_in_both_modes()
 {
    use openwebide_core::WorkspaceMode;
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let transport = std::rc::Rc::new(DeferredSyntax::default());
        let installed = transport.clone();
        let source = "fn hello() { call(\"文🦀\"); }\r\n".repeat(3000);
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("cold-paint.rs".into()));
            state.workspace.content.set(source);
            EditorActions::new(state.workspace).install_syntax_transport(installed);
            view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:500px;height:300px">{editor_view(state)}</div> }
        });
        let actions = EditorActions::new(mounted.state.workspace);
        wait_until("pending source rows visible", || {
            !transport.pending.borrow().is_empty()
                && mounted
                    .root
                    .query_selector(".editor-source-line[data-line='1']")
                    .unwrap()
                    .is_some()
        })
        .await;
        assert!(actions.syntax_is_pending());
        assert!(
            actions.full_row_paint_ready(),
            "pending background workers retain borrowed source paint"
        );
        let (prepared, tokens) = actions.syntax_paint();
        assert!(!prepared);
        assert!(
            tokens.is_empty(),
            "pending paint must not tokenize the whole file on the UI thread"
        );
        assert_eq!(
            mounted
                .element(".editor-source-line[data-line='1']")
                .text_content()
                .unwrap(),
            "fn hello() { call(\"文🦀\"); }\n"
        );
        assert!(
            mounted
                .root
                .query_selector_all(".editor-source-line")
                .unwrap()
                .length()
                < 100
        );
        transport.respond(true);
        wait_until("prepared source styles", || {
            // Startup configuration can supersede the first scoped request.
            if !transport.pending.borrow().is_empty() {
                transport.respond(true);
            }
            !actions.syntax_is_pending()
                && mounted
                    .root
                    .query_selector(".editor-source-line .tok-keyword")
                    .unwrap()
                    .is_some()
        })
        .await;
        // This large response can exceed the analysis-record transfer limit.
        // Both prepared syntax and the terminal lexical fallback must restore styles.
        assert!(!actions.syntax_paint().1.is_empty());
        wait_until("styled source measurements ready", || {
            if !transport.pending.borrow().is_empty() {
                transport.respond(true);
            }
            !actions.syntax_is_pending()
                && mounted
                    .state
                    .workspace
                    .editor_rows
                    .get_untracked()
                    .is_some()
                && mounted
                    .state
                    .workspace
                    .editor_row_preparation
                    .get_untracked()
                    .is_none()
        })
        .await;
        let styled = mounted
            .root
            .query_selector_all(".editor-source-line .tok-keyword")
            .unwrap()
            .length();
        actions
            .command(
                openwebide_frontend::state_actions::editor::EditorCommand::Newline,
                openwebide_core::editor::Selection::caret(0),
                actions.rules().indentation,
            )
            .unwrap();
        wait_until("changed source pending", || {
            !transport.pending.borrow().is_empty()
        })
        .await;
        for _ in 0..3 {
            frame().await;
        }
        assert!(actions.syntax_is_pending());
        assert!(
            mounted
                .state
                .workspace
                .editor_row_preparation
                .get_untracked()
                .is_none(),
            "retained styled source must not start a neutral replacement batch"
        );
        assert_eq!(
            mounted
                .root
                .query_selector_all(".editor-source-line .tok-keyword")
                .unwrap()
                .length(),
            styled
        );
        wait_until("changed styled source restored", || {
            if !transport.pending.borrow().is_empty() {
                transport.respond(true);
            }
            !actions.syntax_is_pending()
                && mounted
                    .root
                    .query_selector(".editor-source-line[data-line='2'] .tok-keyword")
                    .unwrap()
                    .is_some()
        })
        .await;
    }
}

#[wasm_bindgen_test]
async fn entering_newlines_keeps_fold_gutter_fixed_while_syntax_is_pending_in_both_modes() {
    use openwebide_core::WorkspaceMode;
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let transport = std::rc::Rc::new(DeferredSyntax::default());
        let installed = transport.clone();
        let captured = std::rc::Rc::new(std::cell::Cell::new(None::<EditorActions>));
        let capture = captured.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state
                .workspace
                .open_file
                .set(Some("fixed-gutter.rs".into()));
            state
                .workspace
                .content
                .set("fn main() {\n    call();\n}".into());
            let actions = EditorActions::new(state.workspace);
            actions.install_syntax_transport(installed);
            capture.set(Some(actions));
            view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:700px;height:400px">{editor_view(state)}</div> }
        });
        let actions = captured.get().unwrap();
        wait_until("initial folding request", || {
            !transport.pending.borrow().is_empty()
        })
        .await;
        let code: web_sys::HtmlElement = mounted.element(".editor-code").unchecked_into();
        let input: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        let gutter = code
            .style()
            .get_property_value("--editor-gutter-width")
            .unwrap();
        assert!(gutter.contains("42px"));
        wait_until("folding controls", || {
            if !transport.pending.borrow().is_empty() {
                transport.respond(true);
            }
            mounted
                .root
                .query_selector(".editor-fold-control")
                .unwrap()
                .is_some()
        })
        .await;
        assert_eq!(
            code.style()
                .get_property_value("--editor-gutter-width")
                .unwrap(),
            gutter
        );
        frame().await;
        let left = input.get_bounding_client_rect().left();
        assert!(code.class_list().contains("highlight-ready"));
        let presentation = editorWatchPresentation(&code);
        let styled_count = mounted
            .root
            .query_selector_all(".editor-source-line .tok-keyword")
            .unwrap()
            .length();
        assert!(styled_count > 0);
        input.focus().unwrap();
        input.set_selection_range(12, 12).unwrap();
        assert!(editor_key(&input, "Enter", false, false).default_prevented());
        wait_until("new folding request", || {
            !transport.pending.borrow().is_empty()
        })
        .await;
        assert!(actions.syntax_structure(|| true).is_none());
        assert_eq!(actions.source().split('\n').count(), 4);
        assert_eq!(
            code.style()
                .get_property_value("--editor-gutter-width")
                .unwrap(),
            gutter
        );
        assert!((input.get_bounding_client_rect().left() - left).abs() < 0.1);
        for _ in 0..4 {
            frame().await;
        }
        let indicator: web_sys::HtmlButtonElement =
            mounted.element(".editor-fold-control").unchecked_into();
        assert_eq!(
            indicator.get_attribute("aria-label").as_deref(),
            Some("Collapse block at line 1")
        );
        assert!(
            indicator.disabled(),
            "retained indicators must not apply obsolete ranges"
        );
        assert!(code.class_list().contains("highlight-ready"));
        assert_eq!(editorPresentationLosses(&presentation), 0);
        assert_eq!(
            mounted
                .root
                .query_selector_all(".editor-source-line .tok-keyword")
                .unwrap()
                .length(),
            styled_count,
            "pending source must retain its styled frame"
        );
        transport.respond(true);
        wait_until("updated folding controls", || {
            actions.syntax_structure(|| true).is_some()
        })
        .await;
        settle().await;
        assert_eq!(
            code.style()
                .get_property_value("--editor-gutter-width")
                .unwrap(),
            gutter
        );
        assert!((input.get_bounding_client_rect().left() - left).abs() < 0.1);
        frame().await;
        let indicator: web_sys::HtmlButtonElement =
            mounted.element(".editor-fold-control").unchecked_into();
        assert!(!indicator.disabled());
        assert_eq!(editorPresentationLosses(&presentation), 0);
        editorStopPresentationWatch(&presentation);
        drop(mounted);
        settle().await;
    }
}

#[wasm_bindgen_test]
async fn retained_paint_releases_document_and_account_scopes_in_both_modes() {
    use openwebide_core::WorkspaceMode;
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        for boundary in 0..5 {
            let transport = std::rc::Rc::new(DeferredSyntax::default());
            let installed = transport.clone();
            let captured = std::rc::Rc::new(std::cell::Cell::new(None::<EditorActions>));
            let capture = captured.clone();
            let mounted = mount_test(move |state| {
                state.seed_project();
                state
                    .projects
                    .projects
                    .update(|projects| projects[0].mode = mode);
                state.workspace.open_file.set(Some("retained.rs".into()));
                state
                    .workspace
                    .content
                    .set("fn old() {\n    call();\n}".into());
                let actions = EditorActions::new(state.workspace);
                actions.install_syntax_transport(installed);
                capture.set(Some(actions));
                view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:700px;height:400px">{editor_view(state)}</div> }
            });
            wait_until("published retained frame", || {
                if !transport.pending.borrow().is_empty() {
                    transport.respond(true);
                }
                mounted
                    .root
                    .query_selector(".editor-code.highlight-ready")
                    .unwrap()
                    .is_some()
            })
            .await;
            frame().await;
            let actions = captured.get().unwrap();
            let original = untrack(|| actions.presentation_scope());
            match boundary {
                0 => mounted
                    .state
                    .workspace
                    .open_file
                    .set(Some("replacement.rs".into())),
                1 => mounted.state.workspace.active_project.set(Some(2)),
                2 => mounted
                    .state
                    .workspace
                    .editor_read_revision
                    .update(|value| *value += 1),
                3 => mounted
                    .state
                    .workspace
                    .pending_epoch
                    .update(|value| *value += 1),
                _ => mounted.state.auth.generation.update(|value| *value += 1),
            }
            settle().await;
            assert_ne!(untrack(|| actions.presentation_scope()), original);
            assert!(
                mounted
                    .root
                    .query_selector(".editor-code.highlight-ready")
                    .unwrap()
                    .is_none(),
                "scope boundary {boundary} must release the retained frame in {mode:?}"
            );
            drop(mounted);
            settle().await;
        }
    }
}

#[wasm_bindgen_test]
async fn worker_preparation_coalesces_edits_rejects_stale_scopes_and_falls_back_in_both_modes() {
    use openwebide_core::WorkspaceMode;
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let transport = std::rc::Rc::new(DeferredSyntax::default());
        let installed = transport.clone();
        let captured = std::rc::Rc::new(std::cell::Cell::new(None::<EditorActions>));
        let capture = captured.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state.projects.projects.update(|projects| {
                projects[0].mode = mode;
                let mut other = projects[0].clone();
                other.id = 2;
                projects.push(other);
            });
            state.workspace.open_file.set(Some("worker.rs".into()));
            state
                .workspace
                .content
                .set("fn main() {\r\n call(\"文😀\");\r\n}".into());
            let actions = EditorActions::new(state.workspace);
            actions.install_syntax_transport(installed);
            capture.set(Some(actions));
            editor_view(state)
        });
        let actions = captured.get().unwrap();
        wait_until("first deferred worker request", || {
            !transport.pending.borrow().is_empty()
        })
        .await;
        assert!(actions.syntax_structure(|| true).is_none());
        assert!(
            mounted
                .state
                .workspace
                .editor_syntax
                .with_untracked(|cache| cache.is_empty())
        );
        for update in 0..50 {
            mounted
                .state
                .workspace
                .content
                .set(format!("fn latest_{update}() {{\r\n call(\"文😀\");\r\n}}"));
            settle().await;
        }
        assert_eq!(transport.calls.get(), 1);
        assert_eq!(transport.pending.borrow().len(), 1);
        transport.respond(true);
        wait_until("latest coalesced request", || {
            !transport.pending.borrow().is_empty()
        })
        .await;
        assert!(transport.source().contains("latest_49"));
        assert_eq!(transport.calls.get(), 2);
        assert!(actions.syntax_structure(|| true).is_none());
        transport.respond(true);
        wait_until("worker contexts published", || {
            actions.syntax_structure(|| true).is_some()
        })
        .await;
        wait_until("worker syntax paint", || {
            mounted
                .root
                .query_selector(".tok-function")
                .unwrap()
                .is_some()
        })
        .await;
        assert!(
            actions
                .syntax_structure(|| true)
                .unwrap()
                .matches_source(&actions.source())
        );
        assert!(!actions.fold_state().unwrap().ranges().is_empty());
        assert!(
            mounted
                .state
                .workspace
                .editor_syntax
                .with_untracked(|cache| cache.is_empty())
        );
        for change in 0..6 {
            mounted
                .state
                .workspace
                .content
                .set(format!("fn changed_{change}() {{\n call();\n}}"));
            wait_until("request before scope change", || {
                !transport.pending.borrow().is_empty()
            })
            .await;
            match change {
                0 => mounted.state.auth.generation.update(|value| *value += 1),
                1 => mounted
                    .state
                    .workspace
                    .editor_read_revision
                    .update(|value| *value += 1),
                2 => mounted
                    .state
                    .workspace
                    .pending_epoch
                    .update(|value| *value += 1),
                3 => mounted.state.workspace.editor_indentation.update(|values| {
                    values.insert(
                        (1, "worker.rs".into()),
                        openwebide_core::editor::Indentation {
                            tab_width: 8,
                            ..Default::default()
                        },
                    );
                }),
                4 => mounted
                    .state
                    .workspace
                    .open_file
                    .set(Some("other.rs".into())),
                _ => mounted.state.workspace.active_project.set(Some(2)),
            }
            settle().await;
            transport.respond(true);
            wait_until("new scope request replaces stale reply", || {
                !transport.pending.borrow().is_empty()
            })
            .await;
            assert!(
                actions.syntax_structure(|| true).is_none(),
                "{mode:?} scope {change}"
            );
            transport.respond(true);
            wait_until("current scope published", || {
                actions.syntax_structure(|| true).is_some()
            })
            .await;
        }
        mounted
            .state
            .workspace
            .content
            .set("fn fallback() {}".into());
        wait_until("request for failure", || {
            !transport.pending.borrow().is_empty()
        })
        .await;
        transport.respond(false);
        wait_until("shared synchronous fallback", || {
            !mounted.state.workspace.editor_worker_active.get_untracked()
        })
        .await;
        assert!(
            actions
                .syntax_structure(|| true)
                .unwrap()
                .matches_source("fn fallback() {}")
        );
        assert!(
            !mounted
                .state
                .workspace
                .editor_syntax
                .with_untracked(|cache| cache.is_empty())
        );
        assert!(transport.stopped.get());
    }
}

#[wasm_bindgen_test]
async fn lexical_worker_paint_is_cached_lossless_and_source_guarded_in_both_modes() {
    use openwebide_core::WorkspaceMode;
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let source = "{\r\n  \"name\": \"文😀 unchanged source text keeps the publication large enough to benefit from a source replacement span while testing raw CRLF preservation\", \"value\": 42\r\n}\r\n";
        let transport = std::rc::Rc::new(DeferredSyntax::default());
        let installed = transport.clone();
        let captured = std::rc::Rc::new(std::cell::Cell::new(None::<EditorActions>));
        let capture = captured.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("worker.json".into()));
            state.workspace.content.set(source.into());
            let actions = EditorActions::new(state.workspace);
            actions.install_syntax_transport(installed);
            capture.set(Some(actions));
            editor_view(state)
        });
        let actions = captured.get().unwrap();
        wait_until("lexical worker request", || {
            !transport.pending.borrow().is_empty()
        })
        .await;
        assert!(actions.syntax_highlights().is_none());
        transport.respond(true);
        // Configuration can resolve before or after the first worker reply. Drive
        // the bounded replacement request, still checking exact source coverage.
        for _ in 0..2 {
            wait_until("lexical paint or replacement scope", || {
                actions.syntax_highlights().is_some() || !transport.pending.borrow().is_empty()
            })
            .await;
            if actions.syntax_highlights().is_some() {
                break;
            }
            assert_eq!(transport.source(), source);
            transport.respond(true);
        }
        wait_until("lexical worker paint published", || {
            actions.syntax_highlights().is_some()
        })
        .await;
        let first = actions.syntax_highlights().unwrap();
        assert!(std::sync::Arc::ptr_eq(
            &first,
            &actions.syntax_highlights().unwrap()
        ));
        assert_eq!(
            first.as_ref(),
            &openwebide_core::highlight::share_token_rows(
                openwebide_core::highlight::highlight_lines(
                    source,
                    openwebide_core::highlight::Language::Json
                )
            )
        );
        assert!(actions.syntax_structure(|| true).is_none());
        assert!(
            mounted
                .state
                .workspace
                .editor_syntax
                .with_untracked(|cache| cache.is_empty())
        );
        let revised = source.replace("42", "7");
        mounted.state.workspace.content.set(revised.clone());
        assert!(actions.syntax_highlights().is_none());
        wait_until("revised lexical request", || {
            !transport.pending.borrow().is_empty()
        })
        .await;
        let request: serde_json::Value =
            serde_json::from_str(&transport.pending.borrow().front().unwrap().message).unwrap();
        assert!(
            request["base_ticket"].is_u64(),
            "updates advertise a scoped published base"
        );
        assert!(
            request["source"].is_object(),
            "updates send only the changed source span"
        );
        transport.respond(true);
        assert!(
            transport.source_delta.get(),
            "updates publish only the changed source span"
        );
        wait_until("revised lexical paint", || {
            actions.syntax_highlights().is_some()
        })
        .await;
        assert!(!std::sync::Arc::ptr_eq(
            &first,
            &actions.syntax_highlights().unwrap()
        ));
        let updated = actions.syntax_highlights().unwrap();
        assert!(
            std::sync::Arc::ptr_eq(&first[0], &updated[0]),
            "worker replies share unchanged row allocations in the receiver"
        );
        assert!(!std::sync::Arc::ptr_eq(&first[1], &updated[1]));
        mounted
            .state
            .auth
            .generation
            .update(|generation| *generation += 1);
        wait_until("new account requests a standalone result", || {
            !transport.pending.borrow().is_empty()
        })
        .await;
        let request: serde_json::Value =
            serde_json::from_str(&transport.pending.borrow().front().unwrap().message).unwrap();
        assert!(
            request.get("base_ticket").is_none(),
            "publication bases must not cross accounts"
        );
        transport.respond(true);
        wait_until("new account publishes standalone paint", || {
            if !transport.pending.borrow().is_empty() {
                transport.respond(true);
            }
            actions.syntax_highlights().is_some()
        })
        .await;
        wait_until("revised number painted", || {
            mounted
                .root
                .query_selector(".tok-number")
                .unwrap()
                .is_some_and(|number| number.text_content().as_deref() == Some("7"))
        })
        .await;
        assert_eq!(
            mounted
                .element(".editor-highlight-content")
                .text_content()
                .unwrap(),
            revised.replace("\r\n", "\n")
        );
    }
}

#[wasm_bindgen_test]
async fn worker_source_resync_is_bounded_and_rejects_superseded_ownership_in_both_modes() {
    use openwebide_core::{
        WorkspaceMode,
        editor::{SYNTAX_PROTOCOL_VERSION, SyntaxReply, SyntaxStatus},
    };
    use openwebide_frontend::state_actions::editor::EditorActions;
    for (mode, changed_scope) in [
        (WorkspaceMode::Local, 0),
        (WorkspaceMode::Remote, 0),
        (WorkspaceMode::Local, 1),
        (WorkspaceMode::Remote, 1),
        (WorkspaceMode::Local, 2),
        (WorkspaceMode::Remote, 2),
    ] {
        let source = "{\r\n  \"name\": \"文😀 unchanged source text keeps the publication large enough to test replacement spans and full-source retries without changing the raw CRLF document\", \"value\": 42\r\n}\r\n";
        let transport = std::rc::Rc::new(DeferredSyntax::default());
        let installed = transport.clone();
        let captured = std::rc::Rc::new(std::cell::Cell::new(None::<EditorActions>));
        let capture = captured.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("resync.json".into()));
            state.workspace.content.set(source.into());
            let actions = EditorActions::new(state.workspace);
            actions.install_syntax_transport(installed);
            capture.set(Some(actions));
            editor_view(state)
        });
        let actions = captured.get().unwrap();
        wait_until("initial resync document published", || {
            if !transport.pending.borrow().is_empty() {
                transport.respond(true);
            }
            actions.syntax_highlights().is_some()
        })
        .await;
        let revised = source.replace("42", "7");
        mounted.state.workspace.content.set(revised.clone());
        wait_until("delta after worker eviction", || {
            !transport.pending.borrow().is_empty()
        })
        .await;
        transport.service.borrow_mut().clear();
        transport.respond(true);
        wait_until("one full-source retry", || {
            !transport.pending.borrow().is_empty()
        })
        .await;
        let retry: serde_json::Value =
            serde_json::from_str(&transport.pending.borrow().front().unwrap().message).unwrap();
        assert_eq!(retry["source"], revised);
        assert!(retry.get("base_ticket").is_none());
        assert!(
            actions.syntax_highlights().is_none(),
            "resync is never published as preparation"
        );
        transport.respond(true);
        wait_until("resync preparation accepted", || {
            actions.syntax_highlights().is_some()
        })
        .await;
        assert!(mounted.state.workspace.editor_worker_active.get_untracked());
        let second = revised.replace("7", "8");
        mounted.state.workspace.content.set(second.clone());
        wait_until("second delta", || !transport.pending.borrow().is_empty()).await;
        transport.service.borrow_mut().clear();
        transport.respond(true);
        wait_until("retry awaiting changed ownership", || {
            !transport.pending.borrow().is_empty()
        })
        .await;
        let newest = second.replace("8", "9");
        mounted.state.workspace.content.set(newest.clone());
        match changed_scope {
            0 => mounted
                .state
                .auth
                .generation
                .update(|generation| *generation += 1),
            1 => mounted.state.workspace.active_project.set(Some(2)),
            _ => mounted
                .state
                .workspace
                .editor_read_revision
                .update(|revision| *revision += 1),
        }
        let generation = mounted.state.auth.generation.get_untracked();
        let project = mounted
            .state
            .workspace
            .active_project
            .get_untracked()
            .unwrap();
        let read_revision = mounted.state.workspace.editor_read_revision.get_untracked();
        transport.respond(true);
        wait_until(
            "superseded retry ignored and latest source published",
            || {
                if !transport.pending.borrow().is_empty() {
                    transport.respond(true);
                }
                mounted
                    .state
                    .workspace
                    .editor_preparation
                    .with_untracked(|prepared| {
                        prepared.as_ref().is_some_and(|prepared| {
                            prepared.scope.account_generation == generation
                                && prepared.scope.key.0 == project
                                && prepared.scope.read_revision == read_revision
                                && prepared.scope.source.as_ref() == newest
                                && prepared
                                    .analysis
                                    .as_ref()
                                    .is_some_and(|analysis| analysis.matches_source(&newest))
                        })
                    })
            },
        )
        .await;
        // A broken worker that asks for a base even after a full retry must stop;
        // it cannot turn a cache miss into an unbounded request loop.
        mounted
            .state
            .workspace
            .content
            .set(newest.replace("9", "10"));
        wait_until("last delta", || !transport.pending.borrow().is_empty()).await;
        transport.service.borrow_mut().clear();
        transport.respond(true);
        wait_until("last full retry", || !transport.pending.borrow().is_empty()).await;
        let DeferredSyntaxReply { message, sender } =
            transport.pending.borrow_mut().pop_front().unwrap();
        let request: openwebide_core::editor::SyntaxRequest =
            serde_json::from_str(&message).unwrap();
        let reply = SyntaxReply {
            version: SYNTAX_PROTOCOL_VERSION,
            ticket: request.ticket,
            status: SyntaxStatus::NeedsSource,
            analysis: None,
        };
        sender
            .send(Ok(serde_json::to_string(&reply).unwrap()))
            .unwrap();
        wait_until("repeated resync stops transport", || {
            transport.stopped.get()
        })
        .await;
        assert!(!mounted.state.workspace.editor_worker_active.get_untracked());
        assert!(transport.pending.borrow().is_empty());
        wait_until("shared fallback prepares current text", || {
            actions
                .syntax_paint()
                .1
                .iter()
                .flat_map(|row| row.iter())
                .any(|token| token.text == "10")
        })
        .await;
    }
}

#[wasm_bindgen_test]
async fn unwrapped_viewport_bounds_paint_and_maps_scrolled_unicode_carets_in_both_modes() {
    use openwebide_core::WorkspaceMode;
    let source = (0..10_000)
        .map(|line| format!("row {line} 文😀\r\n"))
        .collect::<String>();
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let text = source.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("viewport.txt".into()));
            state.workspace.content.set(text);
            state
                .settings
                .editor_preferences
                .update(|preferences| preferences.word_wrap = false);
            editor_view(state)
        });
        let style = document().create_element("style").unwrap();
        style.set_text_content(Some(&format!(
            "{}\n.editor-code {{width:440px;height:180px;flex:none}}",
            include_str!("../../styles.css")
        )));
        mounted.root.append_child(&style).unwrap();
        frame().await;
        wait_until("viewport paint mounted", || {
            mounted
                .root
                .query_selector(".editor-source-line")
                .unwrap()
                .is_some()
        })
        .await;
        let textarea: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        assert_eq!(textarea.value(), source.replace("\r\n", "\n"));
        assert!(
            mounted
                .root
                .query_selector_all(".editor-source-line")
                .unwrap()
                .length()
                < 80
        );
        for line in [5000, 9990, 0, 2000] {
            textarea.set_scroll_top(f64::from(line) * 19.5);
            textarea
                .dispatch_event(&web_sys::Event::new("scroll").unwrap())
                .unwrap();
            wait_until("scrolled row window", || {
                let Some(row) = mounted.root.query_selector(".editor-source-line").unwrap() else {
                    return false;
                };
                let first = row
                    .get_attribute("data-line")
                    .unwrap()
                    .parse::<i32>()
                    .unwrap()
                    - 1;
                (first - line).abs() <= 12
            })
            .await;
            assert!(
                mounted
                    .root
                    .query_selector_all(".editor-source-line")
                    .unwrap()
                    .length()
                    < 80
            );
            assert!(
                mounted
                    .root
                    .query_selector_all(".editor-fold-row")
                    .unwrap()
                    .length()
                    < 80
            );
            let paint = mounted.element(".editor-highlight-content");
            let offset = paint
                .get_attribute("data-textarea-start")
                .unwrap()
                .parse::<usize>()
                .unwrap();
            let first = mounted.element(".editor-source-line");
            let first_line = first
                .get_attribute("data-line")
                .unwrap()
                .parse::<usize>()
                .unwrap()
                - 1;
            let expected = source
                .split_inclusive('\n')
                .take(first_line)
                .collect::<String>()
                .replace("\r\n", "\n")
                .encode_utf16()
                .count();
            assert_eq!(offset, expected);
            assert_eq!(
                first
                    .get_attribute("data-textarea-start")
                    .unwrap()
                    .parse::<usize>()
                    .unwrap(),
                expected
            );
            let fragment = first
                .query_selector(":scope > .editor-source-fragment")
                .unwrap()
                .unwrap();
            assert_eq!(
                fragment.get_attribute("data-paint-start").as_deref(),
                Some("0")
            );
            assert_eq!(
                first
                    .get_attribute("data-paint-length")
                    .unwrap()
                    .parse::<usize>()
                    .unwrap(),
                first.text_content().unwrap().encode_utf16().count()
            );
            let visible = mounted.element(&format!(
                ".editor-source-line[data-line='{}']",
                (line + 2).min(10_000)
            ));
            let rect = visible.get_bounding_client_rect();
            let caret = openwebide_frontend::viewport::editor_caret_from_point(
                &textarea,
                rect.left() + 3.0,
                rect.top() + 5.0,
            )
            .unwrap();
            let scope = paint.get_attribute("data-editor-scope").unwrap();
            paint.set_attribute("data-editor-scope", "stale").unwrap();
            assert!(
                openwebide_frontend::viewport::editor_caret_from_point(
                    &textarea,
                    rect.left() + 3.0,
                    rect.top() + 5.0
                )
                .is_none()
            );
            paint.set_attribute("data-editor-scope", &scope).unwrap();
            assert_eq!(
                openwebide_frontend::viewport::editor_caret_from_point(
                    &textarea,
                    rect.left() + 3.0,
                    rect.top() + 5.0
                ),
                Some(caret)
            );
            let target = usize::try_from((line + 1).min(9999)).unwrap();
            let target_start = source
                .split_inclusive('\n')
                .take(target)
                .collect::<String>()
                .replace("\r\n", "\n")
                .encode_utf16()
                .count();
            let target_end = target_start
                + source
                    .split('\n')
                    .nth(target)
                    .unwrap()
                    .trim_end_matches('\r')
                    .encode_utf16()
                    .count();
            assert!(
                (target_start..=target_end).contains(&(caret as usize)),
                "caret maps to the clicked source row"
            );
        }
        mounted.click("button[aria-label^='Find in file']");
        settle().await;
        let search: web_sys::HtmlInputElement =
            mounted.element(".editor-find input").unchecked_into();
        search.set_value("row 9000");
        search
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
        wait_until("find reveals a row outside the paint window", || {
            textarea.scroll_top() > 170_000.0
                && mounted
                    .root
                    .query_selector(".editor-source-line[data-line='9001']")
                    .unwrap()
                    .is_some()
        })
        .await;
        mounted.click("button[aria-label='Close find']");
        mounted.state.workspace.content.set("short 文😀\r\n".into());
        textarea.set_scroll_top(0.0);
        textarea
            .dispatch_event(&web_sys::Event::new("scroll").unwrap())
            .unwrap();
        wait_until("shrinking file resets row window", || {
            mounted
                .element(".editor-highlight-content")
                .text_content()
                .as_deref()
                == Some("short 文😀\n")
        })
        .await;
    }
}

#[wasm_bindgen_test]
async fn large_file_viewer_bounds_pages_without_constructing_documents_in_both_modes() {
    use openwebide_core::{
        WorkspaceMode,
        editor::{MAX_EDITOR_LINE_BYTES, MAX_EDITOR_LINES, TEXT_PAGE_BYTES},
    };
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        for source in [
            "\n".repeat(MAX_EDITOR_LINES),
            "😀".repeat(MAX_EDITOR_LINE_BYTES / 4 + 1),
        ] {
            let mounted = mount_test({
                let source = source.clone();
                move |state| {
                    state.seed_project();
                    state
                        .projects
                        .projects
                        .update(|projects| projects[0].mode = mode);
                    state.workspace.open_file.set(Some("large.txt".into()));
                    state.workspace.content.set(source);
                    editor_view(state)
                }
            });
            wait_until("bounded read-only page", || {
                mounted
                    .root
                    .query_selector(".editor-large-file-page")
                    .unwrap()
                    .is_some()
            })
            .await;
            assert!(
                mounted
                    .root
                    .query_selector(".editor-textarea")
                    .unwrap()
                    .is_none()
            );
            assert!(
                mounted
                    .state
                    .workspace
                    .editor_documents
                    .with_untracked(std::collections::HashMap::is_empty)
            );
            openwebide_frontend::util::sleep_ms(10).await;
            let mut rebuilt = String::new();
            loop {
                let text = mounted
                    .element(".editor-large-file-page")
                    .text_content()
                    .unwrap();
                assert!(text.len() <= TEXT_PAGE_BYTES + 3);
                rebuilt.push_str(&text);
                let next = mounted.element(".editor-page-next");
                if next.has_attribute("disabled") {
                    break;
                }
                let previous = mounted
                    .element(".editor-large-file-page")
                    .get_attribute("data-page");
                next.click();
                wait_until("next text page painted", || {
                    mounted
                        .element(".editor-large-file-page")
                        .get_attribute("data-page")
                        != previous
                })
                .await;
                settle().await;
            }
            assert!(
                rebuilt == source,
                "read-only pages must reproduce the complete source ({} vs {} bytes)",
                rebuilt.len(),
                source.len()
            );
            mounted.click(".editor-page-previous");
            settle().await;
            assert_eq!(mounted.state.workspace.content.get_untracked(), source);
            assert!(!mounted.state.workspace.dirty.get_untracked());
            let project = mounted.state.projects.projects.get_untracked()[0].clone();
            let recovery = mounted
                .state
                .workspace
                .editor_recovery(&project, false)
                .unwrap();
            assert!(recovery.files[0].document.is_none());
            mounted.state.workspace.content.set("small again\n".into());
            wait_until("regular editor after source changes", || {
                mounted
                    .root
                    .query_selector(".editor-textarea")
                    .unwrap()
                    .is_some()
            })
            .await;
            assert!(
                mounted
                    .root
                    .query_selector(".editor-large-file-page")
                    .unwrap()
                    .is_none()
            );
            assert_eq!(
                mounted.state.workspace.content.get_untracked(),
                "small again\n"
            );
        }
    }
}

#[wasm_bindgen_test]
async fn editor_capacity_rejection_preserves_native_document_in_both_modes() {
    use openwebide_core::{
        WorkspaceMode,
        editor::{EditError, EditorLimit, MAX_EDITOR_LINE_BYTES, Selection},
    };
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("small.txt".into()));
            state.workspace.content.set("kept".into());
            editor_view(state)
        });
        wait_until("editor admitted", || {
            mounted
                .root
                .query_selector(".editor-textarea")
                .unwrap()
                .is_some()
        })
        .await;
        let actions = EditorActions::new(mounted.state.workspace);
        actions.record_selection(Selection::caret(4)).unwrap();
        let before = mounted.state.workspace.editor_documents.get_untracked();
        assert_eq!(
            actions.native_input(
                "x".repeat(MAX_EDITOR_LINE_BYTES + 1),
                Selection::caret(1),
                "insertFromPaste",
                0.0
            ),
            Err(EditError::Capacity(EditorLimit::LineBytes))
        );
        assert_eq!(mounted.state.workspace.content.get_untracked(), "kept");
        assert_eq!(
            mounted.state.workspace.editor_documents.get_untracked(),
            before
        );
        assert!(!mounted.state.workspace.dirty.get_untracked());
    }
}

#[wasm_bindgen_test]
async fn composition_replicas_obey_editor_admission_before_preview_in_both_modes() {
    use openwebide_core::{
        WorkspaceMode,
        editor::{EditError, EditorLimit, MAX_EDITOR_LINE_BYTES, Selection},
    };
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let source = "x".repeat(MAX_EDITOR_LINE_BYTES - 1);
        let initial = source.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("near-limit.txt".into()));
            state.workspace.content.set(initial);
            // Exercise facade input policy without measuring a 1 MiB DOM row.
            view! { <div>"Composition admission"</div> }
        });
        let actions = EditorActions::new(mounted.state.workspace);
        actions.record_selection(Selection::caret(0)).unwrap();
        mounted
            .state
            .workspace
            .editor_documents
            .update(|documents| {
                let document = documents.get_mut(&(1, "near-limit.txt".into())).unwrap();
                document
                    .set_selections(vec![Selection::caret(0), Selection::caret(source.len())])
                    .unwrap();
            });
        let before = mounted.state.workspace.editor_documents.get_untracked();
        actions.begin_composition();
        assert_eq!(
            actions.projected_input(
                format!("X{source}"),
                Selection::caret(1),
                "insertCompositionText",
                0.0
            ),
            Err(EditError::Capacity(EditorLimit::LineBytes))
        );
        assert_eq!(mounted.state.workspace.content.get_untracked(), source);
        assert_eq!(
            mounted.state.workspace.editor_documents.get_untracked(),
            before
        );
        assert!(!actions.is_composing());
        assert!(!mounted.state.workspace.dirty.get_untracked());
    }
}

#[wasm_bindgen_test]
async fn oversized_change_review_retains_before_and_after_pages_in_both_modes() {
    use openwebide_core::{FileDiff, WorkspaceMode, editor::MAX_EDITOR_LINE_BYTES};
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("review.txt".into()));
            let text = "😀".repeat(MAX_EDITOR_LINE_BYTES / 4 + 1);
            state.workspace.content.set(text.clone());
            state.workspace.pending_edits.update(|edits| {
                edits.insert(
                    "review.txt".into(),
                    FileDiff {
                        path: "review.txt".into(),
                        old: Some("\noriginal\n".into()),
                        new: text,
                        old_unavailable: false,
                        backup_path: None,
                    },
                );
            });
            editor_view(state)
        });
        wait_until("bounded change review", || {
            mounted
                .root
                .query_selector(".editor-large-file-page")
                .unwrap()
                .is_some()
        })
        .await;
        openwebide_frontend::util::sleep_ms(10).await;
        let after = mounted
            .element(".editor-large-file-page")
            .text_content()
            .unwrap();
        assert!(after.starts_with('😀'));
        mounted.click(".editor-large-file .ui-seg-btn:first-child");
        wait_until("before source page painted", || {
            mounted
                .element(".editor-large-file-page")
                .text_content()
                .as_deref()
                == Some("\noriginal\n")
        })
        .await;
        assert_eq!(
            mounted
                .element(".editor-large-file-page")
                .text_content()
                .unwrap(),
            "\noriginal\n"
        );
        mounted.click(".editor-large-file .ui-seg-btn:last-child");
        wait_until("after source page painted", || {
            mounted
                .element(".editor-large-file-page")
                .text_content()
                .as_deref()
                == Some(after.as_str())
        })
        .await;
        assert_eq!(
            mounted
                .element(".editor-large-file-page")
                .text_content()
                .unwrap(),
            after
        );
        assert!(
            mounted
                .state
                .workspace
                .pending_diff
                .get_untracked()
                .is_some()
        );
    }
}

#[wasm_bindgen_test]
async fn wrapped_row_windows_keep_exact_heights_carets_and_offscreen_cursors_in_both_modes() {
    #[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
    export function watch_cursor_probes() {
        let batches = 0, rows = 0, bytes = 0;
        const consume = records => {
            for (const record of records) {
                if (!record.target.closest?.('.editor-row-measure')) continue;
                const added = [...record.addedNodes].filter(node => node.nodeType === 1 && node.matches('.editor-source-line'));
                if (!added.length) continue;
                ++batches; rows = Math.max(rows, added.length);
                bytes = Math.max(bytes, added.reduce((sum, node) => sum + new TextEncoder().encode(node.textContent).length, 0));
            }
        };
        const observer = new MutationObserver(consume);
        observer.observe(document.body, {childList:true, subtree:true});
        return () => {consume(observer.takeRecords()); observer.disconnect(); return [batches,rows,bytes];};
    }
    "#)]
    extern "C" {
        fn watch_cursor_probes() -> js_sys::Function;
    }
    use openwebide_core::{
        WorkspaceMode,
        editor::{Selection, byte_to_textarea, line_column},
    };
    use openwebide_frontend::state_actions::editor::EditorActions;
    let source = (0..1200)
        .map(|line| {
            format!(
                "row {line} 文😀 café\t{}\r\n",
                "wrapped words ".repeat(8 + line % 3)
            )
        })
        .collect::<String>();
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = mount_test({
            let source = source.clone();
            move |state| {
                state.seed_project();
                state
                    .projects
                    .projects
                    .update(|projects| projects[0].mode = mode);
                state
                    .workspace
                    .open_file
                    .set(Some("wrapped-window.txt".into()));
                state.workspace.content.set(source);
                state
                    .settings
                    .editor_preferences
                    .update(|preferences| preferences.word_wrap = true);
                view! { <style>{include_str!("../../styles.css")}</style><div class="wrapped-window-fixture" style="display:flex;width:420px;height:320px">{editor_view(state)}</div> }
            }
        });
        frame().await;
        let actions = EditorActions::new(mounted.state.workspace);
        let input: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        wait_until("exact wrapped height table and paint", || {
            actions.measured_rows().is_some()
                && mounted
                    .root
                    .query_selector(".editor-code.highlight-ready .editor-source-line")
                    .unwrap()
                    .is_some()
        })
        .await;
        let measured = actions.measured_rows().unwrap();
        assert_eq!(measured.rows.len(), 1201);
        assert_editor_native_source(&input, mounted.state.workspace, &source);
        let selector = ".editor-highlight .editor-source-line";
        for target in [500, 1100, 0, 700] {
            openwebide_frontend::viewport::set_editor_scroll_top(
                &input,
                measured.rows.top(target).unwrap() + 12.0,
            );
            input
                .dispatch_event(&web_sys::Event::new("scroll").unwrap())
                .unwrap();
            wait_until("bounded wrapped paint at target", || {
                let Some(row) = mounted.root.query_selector(selector).unwrap() else {
                    return false;
                };
                let first = row
                    .get_attribute("data-line")
                    .unwrap()
                    .parse::<usize>()
                    .unwrap()
                    - 1;
                first.abs_diff(target) <= 8
                    && mounted.root.query_selector_all(selector).unwrap().length() < 80
                    && mounted
                        .root
                        .query_selector(".editor-code.highlight-ready")
                        .unwrap()
                        .is_some()
            })
            .await;
            let first = mounted.element(selector);
            let index = first
                .get_attribute("data-line")
                .unwrap()
                .parse::<usize>()
                .unwrap()
                - 1;
            let expected_top =
                input.get_bounding_client_rect().top() + 12.0 + measured.rows.top(index).unwrap()
                    - openwebide_frontend::viewport::editor_scroll(&input).scroll_top();
            assert!((first.get_bounding_client_rect().top() - expected_top).abs() < 0.5);
            assert!(
                (f64::from(openwebide_frontend::viewport::editor_scroll(&input).scroll_height())
                    - measured.rows.height()
                    - 24.0)
                    .abs()
                    < 2.0
            );
            let visible = mounted.element(&format!(
                ".editor-highlight .editor-source-line[data-line='{}']",
                target + 1
            ));
            let rect = visible.get_bounding_client_rect();
            let caret = openwebide_frontend::viewport::editor_caret_from_point(
                &input,
                rect.left() + 3.0,
                rect.top() + 5.0,
            )
            .unwrap() as usize;
            let start = source
                .split_inclusive('\n')
                .take(target)
                .map(str::len)
                .sum::<usize>();
            let start = byte_to_textarea(&source, start).unwrap();
            assert!((start..=start + 1).contains(&caret));
        }
        let distant = source
            .split_inclusive('\n')
            .take(900)
            .map(str::len)
            .sum::<usize>()
            + 4;
        actions.record_selection(Selection::caret(4)).unwrap();
        actions
            .toggle_cursor(1, "wrapped-window.txt", &source, distant)
            .unwrap();
        // Disjoint cursor neighborhoods exceed one 128-row probe batch. They
        // must not become a persistent hidden DOM copy alongside visible paint.
        let projection = actions.projection().unwrap();
        for row in (20..1200).step_by(20) {
            let at = projection.lines()[row].source.start + 4;
            if at != distant {
                actions
                    .toggle_cursor(1, "wrapped-window.txt", &source, at)
                    .unwrap();
            }
        }
        let before = actions.selections(&source);
        assert!(before.len() > 50);
        let native = u32::try_from(byte_to_textarea(&source, before[0].head).unwrap()).unwrap();
        input.set_selection_range(native, native).unwrap();
        assert!(
            mounted
                .root
                .query_selector(".editor-caret-measure")
                .unwrap()
                .is_none()
        );
        assert!(
            mounted
                .root
                .query_selector_all(".editor-source-line")
                .unwrap()
                .length()
                < 80
        );
        let probes = watch_cursor_probes();
        assert!(editor_key(&input, "ArrowDown", false, false).default_prevented());
        wait_until("both wrapped cursors moved", || {
            actions
                .selections(&source)
                .iter()
                .zip(&before)
                .all(|(after, before)| after.head > before.head)
        })
        .await;
        for (after, before) in actions.selections(&source).iter().zip(&before) {
            assert_eq!(
                line_column(&source, after.head).0,
                line_column(&source, before.head).0
            );
        }
        assert!(editor_key(&input, "ArrowUp", false, false).default_prevented());
        wait_until("offscreen cursor probes restore exact selections", || {
            actions.selections(&source) == before
        })
        .await;
        let observed = js_sys::Array::from(&probes.call0(&wasm_bindgen::JsValue::NULL).unwrap());
        assert!(observed.get(0).as_f64().unwrap() >= 4.0);
        assert!(observed.get(1).as_f64().unwrap() <= 128.0);
        assert!(observed.get(2).as_f64().unwrap() <= 65_536.0);
        wasm_bindgen_test::console_log!(
            "{mode:?}: {} cursors, {} temporary batches, peak {} rows / {} bytes",
            before.len(),
            observed.get(0).as_f64().unwrap(),
            observed.get(1).as_f64().unwrap(),
            observed.get(2).as_f64().unwrap()
        );
        assert!(
            document()
                .query_selector(".editor-row-measure")
                .unwrap()
                .is_none()
        );
        assert!(
            mounted
                .root
                .query_selector_all(".editor-source-line")
                .unwrap()
                .length()
                < 80
        );
        assert_eq!(actions.source(), source);
        assert!(!mounted.state.workspace.dirty.get_untracked());
        mounted
            .element(".wrapped-window-fixture")
            .style()
            .set_property("width", "580px")
            .unwrap();
        wait_until("new exact wrap heights after resize", || {
            actions.measured_rows().is_some_and(|next| {
                next.metrics != measured.metrics && next.rows.height() < measured.rows.height()
            })
        })
        .await;
        assert!(!actions.publish_measured_rows(measured.revision, measured.metrics, measured.rows));
        mounted.state.workspace.content.set("small\r\n".into());
        wait_until("shrunken document height table", || {
            actions
                .measured_rows()
                .is_some_and(|next| next.rows.len() == 2)
        })
        .await;
        assert_eq!(input.value(), "small\n");
        assert!(mounted.root.query_selector_all(selector).unwrap().length() <= 2);
    }
}

#[wasm_bindgen_test]
async fn cold_wrapped_preparation_keeps_input_visible_and_rejects_superseded_batches_in_both_modes()
{
    #[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
    export function without_scheduler() {
        const descriptor = Object.getOwnPropertyDescriptor(globalThis, 'scheduler');
        Object.defineProperty(globalThis, 'scheduler', {value: undefined, configurable: true});
        return () => {
            if (descriptor) Object.defineProperty(globalThis, 'scheduler', descriptor);
            else delete globalThis.scheduler;
        };
    }
    "#)]
    extern "C" {
        fn without_scheduler() -> js_sys::Function;
    }
    struct SchedulerRestore(js_sys::Function);
    impl Drop for SchedulerRestore {
        fn drop(&mut self) {
            self.0.call0(&wasm_bindgen::JsValue::NULL).unwrap();
        }
    }
    use openwebide_core::WorkspaceMode;
    use openwebide_frontend::state_actions::editor::EditorActions;
    let source = "row 文😀 café\t words for wrapping more words\n".repeat(12_000);
    for (mode, fallback) in [
        (WorkspaceMode::Local, false),
        (WorkspaceMode::Remote, false),
        (WorkspaceMode::Local, true),
        (WorkspaceMode::Remote, true),
    ] {
        let _scheduler = fallback.then(|| SchedulerRestore(without_scheduler()));
        let mounted = mount_test({
            let source = source.clone();
            move |state| {
                state.seed_project();
                state
                    .projects
                    .projects
                    .update(|projects| projects[0].mode = mode);
                state
                    .workspace
                    .open_file
                    .set(Some("cold-window.txt".into()));
                state.workspace.content.set(source);
                state
                    .settings
                    .editor_preferences
                    .update(|preferences| preferences.word_wrap = true);
                view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:420px;height:320px">{editor_view(state)}</div> }
            }
        });
        let actions = EditorActions::new(mounted.state.workspace);
        wait_until("cold row preparation advances", || {
            mounted
                .state
                .workspace
                .editor_row_preparation
                .get_untracked()
                .is_some_and(|preparation| {
                    preparation.completed > 0 && preparation.completed < preparation.total
                })
        })
        .await;
        assert!(
            !actions.syntax_is_pending(),
            "cooperative fallback must finish before full cold measurements start"
        );
        let previous = mounted
            .state
            .workspace
            .editor_row_preparation
            .get_untracked()
            .unwrap();
        assert_eq!(
            mounted
                .root
                .query_selector_all(".editor-source-line")
                .unwrap()
                .length(),
            0
        );
        assert!(
            !mounted
                .element(".editor-code")
                .class_list()
                .contains("highlight-ready")
        );
        let textarea: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        assert_eq!(textarea.value(), source);
        assert_ne!(
            web_sys::window()
                .unwrap()
                .get_computed_style(&textarea)
                .unwrap()
                .unwrap()
                .get_property_value("color")
                .unwrap(),
            "rgba(0, 0, 0, 0)"
        );
        let changed = format!("z{source}");
        input(&mounted, &changed);
        assert_eq!(actions.source(), changed);
        assert!(mounted.state.workspace.dirty.get_untracked());
        assert!(!actions.row_preparation_current(previous.ticket));
        actions.report_row_preparation(previous.ticket, usize::MAX);
        wait_until("new cold preparation advances", || {
            mounted
                .state
                .workspace
                .editor_row_preparation
                .get_untracked()
                .is_some_and(|preparation| {
                    preparation.ticket != previous.ticket
                        && preparation.completed > 0
                        && preparation.completed < preparation.total
                })
        })
        .await;
        actions
            .record_selection(openwebide_core::editor::Selection::caret(5))
            .unwrap();
        let distant = 1
            + source
                .split_inclusive('\n')
                .take(9_000)
                .map(str::len)
                .sum::<usize>()
            + 4;
        actions
            .toggle_cursor(1, "cold-window.txt", &changed, distant)
            .unwrap();
        let before = actions.selections(&changed);
        let native = u32::try_from(
            openwebide_core::editor::byte_to_textarea(&changed, before[0].head).unwrap(),
        )
        .unwrap();
        textarea.set_selection_range(native, native).unwrap();
        assert!(editor_key(&textarea, "ArrowDown", false, false).default_prevented());
        assert!(actions.queued_motion_ticket().is_some());
        assert_eq!(actions.selections(&changed), before);
        wait_until("replacement source owns measured paint", || {
            actions
                .measured_rows()
                .is_some_and(|rows| rows.rows.len() == 12_001)
                && mounted
                    .root
                    .query_selector(".editor-code.highlight-ready .editor-source-line")
                    .unwrap()
                    .is_some()
        })
        .await;
        wait_until("cold queued cursors retain their movement", || {
            actions.queued_motion_ticket().is_none()
                && actions
                    .selections(&changed)
                    .iter()
                    .zip(&before)
                    .all(|(after, before)| after.head > before.head)
        })
        .await;
        assert_eq!(actions.source(), changed);
        assert_editor_native_source(&textarea, mounted.state.workspace, &changed);
        assert!(
            mounted
                .root
                .query_selector_all(".editor-source-line")
                .unwrap()
                .length()
                < 80
        );
        wait_until("temporary probes are released", || {
            web_sys::window()
                .unwrap()
                .document()
                .unwrap()
                .query_selector(".editor-row-measure")
                .unwrap()
                .is_none()
        })
        .await;
        assert!(
            mounted
                .root
                .query_selector("[role='alert']")
                .unwrap()
                .is_none()
        );
    }
}

#[wasm_bindgen_test]
async fn localized_wrapped_edits_reuse_exact_row_heights_in_both_modes() {
    #[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
    export function watch_row_measurements() {
        let count = 0;
        const consume = records => {
            for (const record of records) {
                if (!record.target.closest?.('.editor-row-measure')) continue;
                for (const node of record.addedNodes) {
                    if (node.nodeType !== 1) continue;
                    count += Number(node.matches('.editor-source-line'));
                    count += node.querySelectorAll('.editor-source-line').length;
                }
            }
        };
        const observer = new MutationObserver(consume);
        observer.observe(document.body, {childList: true, subtree: true});
        return () => {consume(observer.takeRecords()); observer.disconnect(); return count;};
    }
    "#)]
    extern "C" {
        fn watch_row_measurements() -> js_sys::Function;
    }
    use openwebide_core::WorkspaceMode;
    use openwebide_frontend::state_actions::editor::EditorActions;
    let source = (0..1200)
        .map(|row| format!("row {row} 文😀 café\t words for wrapping\r\n"))
        .collect::<String>();
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = mount_test({
            let source = source.clone();
            move |state| {
                state.seed_project();
                state
                    .projects
                    .projects
                    .update(|projects| projects[0].mode = mode);
                state.workspace.open_file.set(Some("row-reuse.txt".into()));
                state.workspace.content.set(source);
                state
                    .settings
                    .editor_preferences
                    .update(|preferences| preferences.word_wrap = true);
                view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:420px;height:320px">{editor_view(state)}</div> }
            }
        });
        frame().await;
        let actions = EditorActions::new(mounted.state.workspace);
        wait_until("initial reusable wrapped rows", || {
            !actions.syntax_is_pending() && actions.measured_rows().is_some()
        })
        .await;
        let old = actions.measured_rows().unwrap().rows;
        let observed = watch_row_measurements();
        let changed = source.replace(
            "row 500 ",
            &format!("row 500 {} ", "more words ".repeat(30)),
        );
        let at = source.find("row 500 ").unwrap() + "row 500 ".len();
        actions
            .paste(
                &format!("{} ", "more words ".repeat(30)),
                openwebide_core::editor::Selection::caret(at),
            )
            .unwrap();
        assert_eq!(actions.source(), changed);
        wait_until("changed row measured with reused neighbors", || {
            actions.measured_rows().is_some_and(|rows| {
                rows.rows.top(501).unwrap() - rows.rows.top(500).unwrap()
                    > old.top(501).unwrap() - old.top(500).unwrap()
            })
        })
        .await;
        let rows = actions.measured_rows().unwrap().rows;
        assert_eq!(rows.len(), old.len());
        for row in [0, 499, 501, 1199, 1200] {
            assert!(
                (rows.top(row + 1).unwrap()
                    - rows.top(row).unwrap()
                    - (old.top(row + 1).unwrap() - old.top(row).unwrap()))
                .abs()
                    < f64::EPSILON
            );
        }
        settle().await;
        let count = observed
            .call0(&wasm_bindgen::JsValue::NULL)
            .unwrap()
            .as_f64()
            .unwrap();
        assert!(
            count > 0.0 && count < 10.0,
            "measured {count} logical rows after a one-row edit"
        );
        let observed = watch_row_measurements();
        let textarea: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        assert!(editor_key(&textarea, "z", true, false).default_prevented());
        wait_until("undo restores exact reusable height table", || {
            actions.source() == source
                && actions.measured_rows().is_some_and(|rows| rows.rows == old)
        })
        .await;
        settle().await;
        let count = observed
            .call0(&wasm_bindgen::JsValue::NULL)
            .unwrap()
            .as_f64()
            .unwrap();
        assert!(
            count > 0.0 && count < 10.0,
            "measured {count} logical rows after undo"
        );
        let observed = watch_row_measurements();
        let inserted = format!("new row\r\n{source}");
        actions
            .paste("new row\r\n", openwebide_core::editor::Selection::caret(0))
            .unwrap();
        assert_eq!(actions.source(), inserted);
        wait_until("inserted row preserves suffix measurements", || {
            actions
                .measured_rows()
                .is_some_and(|rows| rows.rows.len() == old.len() + 1)
        })
        .await;
        let shifted = actions.measured_rows().unwrap().rows;
        for row in [0, 500, 1199, 1200] {
            assert!(
                (shifted.top(row + 2).unwrap()
                    - shifted.top(row + 1).unwrap()
                    - (old.top(row + 1).unwrap() - old.top(row).unwrap()))
                .abs()
                    < f64::EPSILON
            );
        }
        settle().await;
        let count = observed
            .call0(&wasm_bindgen::JsValue::NULL)
            .unwrap()
            .as_f64()
            .unwrap();
        assert!(
            count > 0.0 && count < 10.0,
            "measured {count} rows after insertion"
        );
        let observed = watch_row_measurements();
        actions
            .paste(
                "",
                openwebide_core::editor::Selection {
                    anchor: 0,
                    head: "new row\r\n".len(),
                },
            )
            .unwrap();
        assert_eq!(actions.source(), source);
        wait_until("deleted row restores suffix measurements", || {
            actions.measured_rows().is_some_and(|rows| rows.rows == old)
        })
        .await;
        settle().await;
        let count = observed
            .call0(&wasm_bindgen::JsValue::NULL)
            .unwrap()
            .as_f64()
            .unwrap();
        assert!(count < 10.0, "measured {count} rows after deletion");
        let observed = watch_row_measurements();
        actions.invalidate_measured_font();
        wait_until("font invalidation prepares all rows again", || {
            actions.measured_rows().is_some()
        })
        .await;
        settle().await;
        let count = observed
            .call0(&wasm_bindgen::JsValue::NULL)
            .unwrap()
            .as_f64()
            .unwrap();
        assert!(
            count >= 1201.0,
            "reused rows after font invalidation: {count}"
        );
        let cache = mounted
            .state
            .workspace
            .editor_row_cache
            .get_untracked()
            .unwrap();
        actions.invalidate_measured_rows();
        for owner in 0..4 {
            let mut stale = cache.paint.clone();
            match owner {
                0 => stale.key.0 += 1,
                1 => stale.epoch += 1,
                2 => stale.read_revision += 1,
                _ => stale.account_generation += 1,
            }
            let ticket = actions
                .begin_row_preparation(actions.view_revision(), old.len())
                .unwrap();
            assert!(
                actions
                    .finish_row_preparation(ticket, stale, Ok(Some(cache.rows.clone())))
                    .is_none()
            );
            assert!(
                actions.measured_rows().is_none(),
                "published stale owner {owner}"
            );
        }
        let observed = watch_row_measurements();
        let disjoint = source
            .replace("row 200 ", "row 200 additional words additional words ")
            .replace("row 1000 ", "row 1000 additional words additional words ");
        actions
            .record_selection(openwebide_core::editor::Selection::caret(
                source.find("row 200 ").unwrap() + "row 200 ".len(),
            ))
            .unwrap();
        actions
            .toggle_cursor(
                1,
                "row-reuse.txt",
                &source,
                source.find("row 1000 ").unwrap() + "row 1000 ".len(),
            )
            .unwrap();
        actions
            .paste(
                "additional words additional words ",
                actions.selection(&source).unwrap(),
            )
            .unwrap();
        assert_eq!(actions.source(), disjoint);
        wait_until("disjoint edits reuse unchanged interior rows", || {
            actions.measured_rows().is_some()
        })
        .await;
        settle().await;
        let count = observed
            .call0(&wasm_bindgen::JsValue::NULL)
            .unwrap()
            .as_f64()
            .unwrap();
        assert!(
            count > 0.0 && count < 10.0,
            "measured {count} rows between disjoint edits"
        );
    }
}

#[wasm_bindgen_test]
async fn measured_neighborhoods_move_queued_cursors_through_the_shared_facade_in_both_modes() {
    use openwebide_core::{
        WorkspaceMode,
        editor::{Selection, SelectionMotion, VisualCaret, VisualLayout, VisualLineRows},
    };
    use openwebide_frontend::state_actions::editor::EditorActions;
    let source = "文😀ab\r\nxy\r\nabcdef";
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let saved = std::rc::Rc::new(std::cell::Cell::new(None));
        let slot = saved.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("neighbors.txt".into()));
            state.workspace.content.set(source.into());
            state
                .settings
                .editor_preferences
                .update(|preferences| preferences.word_wrap = true);
            slot.set(Some(EditorActions::new(state.workspace)));
            editor_view(state)
        });
        settle().await;
        let actions = saved.get().unwrap();
        actions.record_selection(Selection::caret(8)).unwrap();
        actions
            .toggle_cursor(1, "neighbors.txt", source, 16)
            .unwrap();
        assert_eq!(
            actions.selections(source),
            [Selection::caret(16), Selection::caret(8)]
        );
        let mut carets = Vec::new();
        for (row, offsets) in [
            (0, vec![0, 3, 7]),
            (1, vec![7, 8, 9]),
            (11, vec![11, 12, 13]),
            (15, vec![15, 16, 17, 18]),
            (16, vec![18, 19, 20, 21]),
        ] {
            for (column, offset) in offsets.into_iter().enumerate() {
                carets.push(VisualCaret {
                    row,
                    offset,
                    column: i64::try_from(column * 64).unwrap(),
                });
            }
        }
        let geometry = [
            VisualLineRows { line: 0, rows: 2 },
            VisualLineRows { line: 1, rows: 1 },
            VisualLineRows { line: 2, rows: 2 },
        ];
        let layout = VisualLayout::neighborhood(
            source,
            actions.projection().unwrap(),
            "sampled rows".into(),
            &geometry,
            carets,
        )
        .unwrap();
        let (ticket, _) = actions
            .queue_motion(1, "neighbors.txt", source, SelectionMotion::Down, false)
            .unwrap()
            .unwrap();
        let moved = actions
            .apply_queued_motion(ticket, Some(&layout))
            .unwrap()
            .unwrap();
        assert_eq!(moved, [Selection::caret(19), Selection::caret(12)]);
        assert!(actions.queued_motion_ticket().is_none());
        assert_eq!(actions.source(), source);
        assert!(!mounted.state.workspace.dirty.get_untracked());
    }
}

#[wasm_bindgen_test]
async fn cold_neighborhoods_flush_arrows_before_native_edits_composition_and_clipboard_in_both_modes()
 {
    use openwebide_core::WorkspaceMode;
    use openwebide_frontend::state_actions::editor::EditorActions;
    let row = "line row 文😀 café\t words for wrapping more words\r\n";
    let source = row.repeat(12_000);
    let second = row.len() * 9_000;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        for operation in 0..4 {
            let mut eager: Option<(String, Vec<openwebide_core::editor::Selection>)> = None;
            for cold in [false, true] {
                let mounted = mount_test({
                    let source = source.clone();
                    move |state| {
                        state.seed_project();
                        state
                            .projects
                            .projects
                            .update(|projects| projects[0].mode = mode);
                        state.workspace.open_file.set(Some("cold-edits.txt".into()));
                        state.workspace.content.set(source);
                        state
                            .settings
                            .editor_preferences
                            .update(|preferences| preferences.word_wrap = true);
                        view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:420px;height:320px">{editor_view(state)}</div> }
                    }
                });
                let actions = EditorActions::new(mounted.state.workspace);
                if cold {
                    wait_until("cold layout is still preparing", || {
                        mounted
                            .state
                            .workspace
                            .editor_row_preparation
                            .get_untracked()
                            .is_some_and(|job| job.completed > 0 && job.completed < job.total)
                            && actions.measured_rows().is_none()
                    })
                    .await;
                    assert!(
                        !mounted
                            .element(".editor-code")
                            .class_list()
                            .contains("highlight-ready")
                    );
                } else {
                    wait_until("eager layout is painted", || {
                        actions.measured_rows().is_some()
                            && mounted
                                .element(".editor-code")
                                .class_list()
                                .contains("highlight-ready")
                    })
                    .await;
                }
                seed_wrapped_carets(&mounted, &source, second);
                let textarea: web_sys::HtmlTextAreaElement =
                    mounted.element(".editor-textarea").unchecked_into();
                editor_key(&textarea, "ArrowDown", false, false);
                editor_key(&textarea, "ArrowDown", false, operation == 3);
                if cold {
                    assert!(actions.queued_motion_ticket().is_some());
                }
                match operation {
                    0 => {
                        let init = web_sys::InputEventInit::new();
                        init.set_bubbles(true);
                        init.set_cancelable(true);
                        init.set_input_type("insertText");
                        init.set_data(Some("X"));
                        let before =
                            web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init)
                                .unwrap();
                        textarea.dispatch_event(&before).unwrap();
                        assert!(!before.default_prevented());
                        editorNativeInput(&textarea, "X", "insertText", false);
                    }
                    1 => {
                        textarea
                            .dispatch_event(
                                &web_sys::CompositionEvent::new("compositionstart").unwrap(),
                            )
                            .unwrap();
                        assert!(actions.is_composing());
                        editorNativeInput(&textarea, "文😀", "insertCompositionText", true);
                        textarea
                            .dispatch_event(
                                &web_sys::CompositionEvent::new("compositionend").unwrap(),
                            )
                            .unwrap();
                        assert!(!actions.is_composing());
                    }
                    2 => {
                        assert!(editorClipboardPaste(&textarea, "pasted 文😀").default_prevented());
                    }
                    _ => {
                        assert!(editorClipboardCut(&textarea).default_prevented());
                    }
                }
                assert!(actions.queued_motion_ticket().is_none());
                assert!(
                    mounted
                        .root
                        .query_selector("[role='alert']")
                        .unwrap()
                        .is_none()
                );
                let result = (actions.source(), actions.selections(&actions.source()));
                assert!(
                    result.0 != source,
                    "operation {operation} did not edit in {mode:?}, cold={cold}"
                );
                if let Some(expected) = &eager {
                    assert!(
                        result.0 == expected.0,
                        "cold source differs for operation {operation} in {mode:?}"
                    );
                    assert_eq!(
                        result.1, expected.1,
                        "cold selection differs for operation {operation} in {mode:?}"
                    );
                } else {
                    eager = Some(result);
                }
            }
        }
    }
}

#[wasm_bindgen_test]
async fn wrapped_fragment_scroll_and_find_preserve_source_coordinates_in_both_modes() {
    use openwebide_core::WorkspaceMode;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let source = format!(
            "{}needle-end\r\nshort\r\n",
            "文😀e\u{301}\t words ".repeat(10_000)
        );
        let original = source.clone();
        let normalized: Vec<_> = source.replace("\r\n", "\n").encode_utf16().collect();
        let audit_source = js_sys::Function::new_no_args(
            r#"
            const state = {max: 0, cold: 0, paint: 0};
            const old = Range.prototype.getClientRects;
            Range.prototype.getClientRects = function(...args) {
                const node = this.startContainer;
                const element = node.nodeType === 1 ? node : node.parentElement;
                const row = element?.closest('.editor-row-measure .editor-source-line');
                if (row) {
                    state.max = Math.max(state.max, row.textContent.length);
                    const kind = row.closest('.editor-height-measure') ? 'cold' : 'paint';
                    state[kind] = Math.max(state[kind], row.textContent.length);
                }
                return old.apply(this, args);
            };
            state.restore = () => { Range.prototype.getClientRects = old; };
            return state;
        "#,
        );
        let cold_source = audit_source.call0(&wasm_bindgen::JsValue::NULL).unwrap();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("window.txt".into()));
            state.workspace.content.set(source.clone());
            state
                .settings
                .editor_preferences
                .update(|preferences| preferences.word_wrap = true);
            view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:340px;height:380px">{editor_view(state)}</div> }
        });
        wait_until("fine wrapped paint", || {
            mounted
                .root
                .query_selector(".editor-source-line[data-paint-top]")
                .unwrap()
                .is_some()
        })
        .await;
        let input: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        let height = mounted
            .element(".editor-source-line")
            .get_bounding_client_rect()
            .height();
        assert!(height > 10_000.0);
        let cold = js_sys::Reflect::get(&cold_source, &"cold".into())
            .unwrap()
            .as_f64()
            .unwrap();
        let paint = js_sys::Reflect::get(&cold_source, &"paint".into())
            .unwrap()
            .as_f64()
            .unwrap();
        js_sys::Reflect::get(&cold_source, &"restore".into())
            .unwrap()
            .dyn_into::<js_sys::Function>()
            .unwrap()
            .call0(&wasm_bindgen::JsValue::NULL)
            .unwrap();
        assert!(
            cold > 65_536.0,
            "cold height measurement must supply full-row anchors"
        );
        assert!(
            paint > 0.0 && paint <= 65_536.0,
            "first wrapped paint must reuse cold anchors: {paint}"
        );
        let actions =
            openwebide_frontend::state_actions::editor::EditorActions::new(mounted.state.workspace);
        let old_epoch = actions.layout_epoch();
        js_sys::Function::new_no_args("document.fonts.dispatchEvent(new Event('loadingdone'))")
            .call0(&wasm_bindgen::JsValue::NULL)
            .unwrap();
        wait_until("same computed font starts fresh cold preparation", || {
            actions.layout_epoch() != old_epoch
                && actions.measured_rows().is_some()
                && mounted
                    .root
                    .query_selector(".editor-code.highlight-ready")
                    .unwrap()
                    .is_some()
        })
        .await;
        // Settle initial font/syntax scope before measuring unseen intervals.
        for top in [10_000.0, 0.0] {
            openwebide_frontend::viewport::set_editor_scroll_top(&input, top);
            input
                .dispatch_event(&web_sys::Event::new("scroll").unwrap())
                .unwrap();
            wait_until("wrapped anchor scope settled", || {
                mounted
                    .root
                    .query_selector(".editor-source-line")
                    .unwrap()
                    .and_then(|row| row.get_attribute("data-paint-top"))
                    .and_then(|value| value.parse::<f64>().ok())
                    .is_some_and(|paint_top| {
                        (paint_top
                            - openwebide_frontend::viewport::editor_scroll(&input).scroll_top())
                        .abs()
                            < 200.0
                    })
            })
            .await;
            settle().await;
        }
        openwebide_frontend::components::take_highlight_source_bytes();
        let measured_source = audit_source.call0(&wasm_bindgen::JsValue::NULL).unwrap();
        for top in [height / 2.0, height - 1000.0, 0.0] {
            openwebide_frontend::viewport::set_editor_scroll_top(&input, top);
            input
                .dispatch_event(&web_sys::Event::new("scroll").unwrap())
                .unwrap();
            wait_until("fragment follows scrolling inside one logical row", || {
                mounted
                    .root
                    .query_selector(".editor-source-line")
                    .unwrap()
                    .and_then(|row| row.get_attribute("data-paint-top"))
                    .and_then(|value| value.parse::<f64>().ok())
                    .is_some_and(|paint_top| {
                        (paint_top
                            - openwebide_frontend::viewport::editor_scroll(&input).scroll_top())
                        .abs()
                            < 200.0
                    })
            })
            .await;
            let generated = openwebide_frontend::components::take_highlight_source_bytes();
            assert!(
                generated <= 65_536,
                "new intervals must slice source before HTML generation: {generated}"
            );
            let row = mounted.element(".editor-source-line");
            assert!((row.get_bounding_client_rect().height() - height).abs() < 0.5);
            let fragment = row
                .query_selector(":scope > .editor-source-fragment")
                .unwrap()
                .unwrap();
            let start: usize = fragment
                .get_attribute("data-paint-start")
                .unwrap()
                .parse()
                .unwrap();
            let end: usize = fragment
                .get_attribute("data-paint-end")
                .unwrap()
                .parse()
                .unwrap();
            assert_eq!(
                fragment.text_content().unwrap(),
                String::from_utf16(&normalized[start..end]).unwrap()
            );
            assert!(fragment.text_content().unwrap().len() < 16_384);
            assert_editor_native_source(&input, mounted.state.workspace, &original);
            let bounds = input.get_bounding_client_rect();
            let hit = openwebide_frontend::viewport::editor_caret_from_point(
                &input,
                bounds.left() + 6.0,
                bounds.top() + 26.0,
            )
            .unwrap() as usize;
            assert!((start..end).contains(&hit));
            assert_eq!(
                web_sys::window()
                    .unwrap()
                    .document()
                    .unwrap()
                    .query_selector_all(".editor-row-measure")
                    .unwrap()
                    .length(),
                0
            );
        }
        let largest = js_sys::Reflect::get(&measured_source, &"max".into())
            .unwrap()
            .as_f64()
            .unwrap();
        js_sys::Reflect::get(&measured_source, &"restore".into())
            .unwrap()
            .dyn_into::<js_sys::Function>()
            .unwrap()
            .call0(&wasm_bindgen::JsValue::NULL)
            .unwrap();
        assert!(
            largest > 0.0 && largest <= 65_536.0,
            "new wrapped intervals must measure bounded source: {largest}"
        );
        mounted.click("button[aria-label^='Find in file']");
        settle().await;
        let search: web_sys::HtmlInputElement =
            mounted.element(".editor-find input").unchecked_into();
        search.set_value("needle-end");
        search
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
        wait_until(
            "find reveals omitted text inside a long logical row",
            || {
                if openwebide_frontend::viewport::editor_scroll(&input).scroll_top()
                    <= height - 1000.0
                {
                    return false;
                }
                let Some(fragment) = mounted
                    .root
                    .query_selector(".editor-source-fragment")
                    .unwrap()
                else {
                    return false;
                };
                let mut pending = vec![web_sys::Node::from(fragment)];
                while let Some(node) = pending.pop() {
                    if node.node_type() == web_sys::Node::TEXT_NODE
                        && let Some(value) = node.node_value()
                        && let Some(byte) = value.find("needle-end")
                    {
                        let at = u32::try_from(value[..byte].encode_utf16().count()).unwrap();
                        let range = web_sys::window()
                            .unwrap()
                            .document()
                            .unwrap()
                            .create_range()
                            .unwrap();
                        range.set_start(&node, at).unwrap();
                        range.set_end(&node, at + 10).unwrap();
                        let rect = range.get_bounding_client_rect();
                        let pane = input.get_bounding_client_rect();
                        return rect.top() >= pane.top() && rect.bottom() <= pane.bottom();
                    }
                    let children = node.child_nodes();
                    pending.extend((0..children.length()).filter_map(|index| children.item(index)));
                }
                false
            },
        )
        .await;
        assert!(
            openwebide_frontend::viewport::editor_scroll(&input).scroll_top() > height - 1000.0
        );
        assert_eq!(mounted.state.workspace.content.get_untracked(), original);
        assert!(!mounted.state.workspace.dirty.get_untracked());
        assert!(
            mounted
                .root
                .query_selector("[role='alert']")
                .unwrap()
                .is_none()
        );
        mounted.click("button[aria-label='Close find']");
        let bidi = format!("{}א", "LTR words ".repeat(10_000));
        let audit = audit_source.call0(&wasm_bindgen::JsValue::NULL).unwrap();
        mounted.state.workspace.content.set(bidi.clone());
        wait_until(
            "wrapped bidi source uses complete paragraph geometry",
            || {
                mounted.state.workspace.content.get_untracked() == bidi
                    && mounted
                        .root
                        .query_selector(".editor-source-line")
                        .unwrap()
                        .and_then(|row| row.get_attribute("data-paint-length"))
                        .and_then(|length| length.parse::<usize>().ok())
                        == Some(bidi.encode_utf16().count())
                    && mounted
                        .root
                        .query_selector(".editor-code.highlight-ready")
                        .unwrap()
                        .is_some()
            },
        )
        .await;
        openwebide_frontend::viewport::set_editor_scroll_top(
            &input,
            f64::from(openwebide_frontend::viewport::editor_scroll(&input).scroll_height()) / 2.0,
        );
        input
            .dispatch_event(&web_sys::Event::new("scroll").unwrap())
            .unwrap();
        wait_until("wrapped bidi fallback follows the destination", || {
            mounted
                .root
                .query_selector(".editor-source-line")
                .unwrap()
                .and_then(|row| row.get_attribute("data-paint-top"))
                .and_then(|top| top.parse::<f64>().ok())
                .is_some_and(|top| {
                    (top - openwebide_frontend::viewport::editor_scroll(&input).scroll_top()).abs()
                        < 200.0
                })
        })
        .await;
        let largest = js_sys::Reflect::get(&audit, &"max".into())
            .unwrap()
            .as_f64()
            .unwrap();
        js_sys::Reflect::get(&audit, &"restore".into())
            .unwrap()
            .dyn_into::<js_sys::Function>()
            .unwrap()
            .call0(&wasm_bindgen::JsValue::NULL)
            .unwrap();
        assert!(
            largest > 65_536.0,
            "bidi paragraphs retain full-source validation"
        );
        assert_editor_native_source(&input, mounted.state.workspace, &bidi);
    }
}

#[wasm_bindgen_test]
async fn horizontal_fragments_preserve_tabs_scroll_extent_and_native_hits_in_both_modes() {
    use openwebide_core::WorkspaceMode;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let source = format!(
            "{}needle-end\r\n{}\r\n",
            "文😀e\u{301}\t words ".repeat(10_000),
            "文😀e\u{301}\t words ".repeat(5000)
        );
        let original = source.clone();
        let normalized: Vec<_> = source.replace("\r\n", "\n").encode_utf16().collect();
        let initial_source = js_sys::Function::new_no_args(
            r#"
            const state = {cold: 0, paint: 0};
            const old = Range.prototype.getClientRects;
            Range.prototype.getClientRects = function(...args) {
                const node = this.startContainer;
                const element = node.nodeType === 1 ? node : node.parentElement;
                const row = element?.closest('.editor-row-measure .editor-source-line');
                if (row) {
                    const kind = row.closest('.editor-height-measure') ? 'cold' : 'paint';
                    state[kind] = Math.max(state[kind], row.textContent.length);
                }
                return old.apply(this, args);
            };
            state.restore = () => { Range.prototype.getClientRects = old; };
            return state;
        "#,
        )
        .call0(&wasm_bindgen::JsValue::NULL)
        .unwrap();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("horizontal.txt".into()));
            state.workspace.content.set(source.clone());
            view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:340px;height:380px">{editor_view(state)}</div> }
        });
        wait_until("horizontal fragment ready", || {
            mounted
                .root
                .query_selector(".editor-source-line[data-paint-left]")
                .unwrap()
                .is_some()
        })
        .await;
        wait_until("horizontal source dimensions prepared", || {
            mounted
                .state
                .workspace
                .editor_rows
                .get_untracked()
                .is_some()
        })
        .await;
        let actions =
            openwebide_frontend::state_actions::editor::EditorActions::new(mounted.state.workspace);
        wait_until("horizontal fallback syntax ready", || {
            !actions.syntax_is_pending()
        })
        .await;
        frame().await;
        let input: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        // Initial font/syntax readiness can replace the paint scope. Establish
        // its first measured anchors before auditing previously unseen intervals.
        for x in [10_000.0, 0.0] {
            openwebide_frontend::viewport::set_editor_scroll_left(&input, x);
            input
                .dispatch_event(&web_sys::Event::new("scroll").unwrap())
                .unwrap();
            wait_until("initial horizontal geometry settled", || {
                mounted
                    .root
                    .query_selector(".editor-source-line")
                    .unwrap()
                    .and_then(|row| row.get_attribute("data-paint-left"))
                    .and_then(|left| left.parse::<f64>().ok())
                    .is_some_and(|left| if x == 0.0 { left == 0.0 } else { left > 9000.0 })
            })
            .await;
            settle().await;
        }
        let measured = |kind: &str| {
            js_sys::Reflect::get(&initial_source, &kind.into())
                .unwrap()
                .as_f64()
                .unwrap()
        };
        let cold = measured("cold");
        let paint = measured("paint");
        js_sys::Reflect::get(&initial_source, &"restore".into())
            .unwrap()
            .dyn_into::<js_sys::Function>()
            .unwrap()
            .call0(&wasm_bindgen::JsValue::NULL)
            .unwrap();
        assert!(
            cold > 65_536.0,
            "cold horizontal probes retain exact anchors"
        );
        assert!(
            paint > 0.0 && paint <= 65_536.0,
            "initial horizontal paint must reuse cold anchors: {paint}"
        );
        // Audit the source extent after initial geometry and native-window
        // installation settle, rather than a transient cold native extent.
        let width = openwebide_frontend::viewport::editor_scroll(&input).scroll_width();
        assert!(width > 100_000);
        let measured_source = js_sys::Function::new_no_args(
            r#"
            const state = {max: 0};
            const old = Range.prototype.getClientRects;
            Range.prototype.getClientRects = function(...args) {
                const node = this.startContainer;
                const element = node.nodeType === 1 ? node : node.parentElement;
                const row = element?.closest('.editor-row-measure .editor-source-line');
                if (row) state.max = Math.max(state.max, row.textContent.length);
                return old.apply(this, args);
            };
            state.restore = () => { Range.prototype.getClientRects = old; };
            return state;
        "#,
        )
        .call0(&wasm_bindgen::JsValue::NULL)
        .unwrap();
        openwebide_frontend::components::take_highlight_source_bytes();
        for x in [
            10_000.0,
            f64::from(width) / 2.0,
            f64::from(width - input.client_width()) - 20.0,
            0.0,
        ] {
            openwebide_frontend::viewport::set_editor_scroll_left(&input, x);
            input
                .dispatch_event(&web_sys::Event::new("scroll").unwrap())
                .unwrap();
            wait_until(
                "horizontal fragment follows viewport-sized intervals",
                || {
                    mounted
                        .root
                        .query_selector(".editor-source-line")
                        .unwrap()
                        .and_then(|row| row.get_attribute("data-paint-left"))
                        .and_then(|value| value.parse::<f64>().ok())
                        .is_some_and(|left| {
                            left <= openwebide_frontend::viewport::editor_scroll(&input)
                                .scroll_left()
                                + 30.0
                                && openwebide_frontend::viewport::editor_scroll(&input)
                                    .scroll_left()
                                    - left
                                    < f64::from(input.client_width()) * 2.0 + 30.0
                        })
                },
            )
            .await;
            let generated = openwebide_frontend::components::take_highlight_source_bytes();
            assert!(
                generated <= 65_536,
                "horizontal intervals must slice source before HTML generation: {generated}"
            );
            let row = mounted.element(".editor-source-line");
            let fragment = row
                .query_selector(":scope > .editor-source-fragment")
                .unwrap()
                .unwrap();
            let start: usize = fragment
                .get_attribute("data-paint-start")
                .unwrap()
                .parse()
                .unwrap();
            let end: usize = fragment
                .get_attribute("data-paint-end")
                .unwrap()
                .parse()
                .unwrap();
            assert_eq!(
                fragment.text_content().unwrap(),
                String::from_utf16(&normalized[start..end]).unwrap()
            );
            assert!(fragment.text_content().unwrap().len() < 16_384);
            assert_eq!(
                openwebide_frontend::viewport::editor_scroll(&input).scroll_width(),
                width
            );
            assert_editor_native_source(&input, mounted.state.workspace, &original);
            let bounds = input.get_bounding_client_rect();
            let hit = openwebide_frontend::viewport::editor_caret_from_point(
                &input,
                bounds.left() + 8.0,
                bounds.top() + 20.0,
            )
            .unwrap() as usize;
            assert!((start..end).contains(&hit));
            if x > f64::from(width) * 0.75 {
                assert_eq!(
                    mounted
                        .element(".editor-source-line[data-line='2']")
                        .text_content()
                        .as_deref(),
                    Some("")
                );
            }
            assert_eq!(
                web_sys::window()
                    .unwrap()
                    .document()
                    .unwrap()
                    .query_selector_all(".editor-row-measure")
                    .unwrap()
                    .length(),
                0
            );
        }
        let largest_measured_source = js_sys::Reflect::get(&measured_source, &"max".into())
            .unwrap()
            .as_f64()
            .unwrap();
        js_sys::Reflect::get(&measured_source, &"restore".into())
            .unwrap()
            .dyn_into::<js_sys::Function>()
            .unwrap()
            .call0(&wasm_bindgen::JsValue::NULL)
            .unwrap();
        assert!(
            largest_measured_source > 0.0 && largest_measured_source <= 65_536.0,
            "new horizontal intervals must measure source slices, not the complete logical row: {largest_measured_source}"
        );
        mounted.click("button[aria-label^='Find in file']");
        settle().await;
        let search: web_sys::HtmlInputElement =
            mounted.element(".editor-find input").unchecked_into();
        search.set_value("needle-end");
        search
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
        wait_until("find reveals omitted horizontal text", || {
            openwebide_frontend::viewport::editor_scroll(&input).scroll_left()
                > f64::from(width) - 1000.0
                && mounted
                    .element(".editor-source-fragment")
                    .text_content()
                    .is_some_and(|text| text.contains("needle-end"))
        })
        .await;
        assert_eq!(mounted.state.workspace.content.get_untracked(), original);
        assert!(!mounted.state.workspace.dirty.get_untracked());
        assert!(
            mounted
                .root
                .query_selector("[role='alert']")
                .unwrap()
                .is_none()
        );
        mounted.click("button[aria-label='Close find']");
        let bidi = format!("{}א", "LTR words ".repeat(10_000));
        mounted.state.workspace.content.set(bidi.clone());
        wait_until(
            "bidirectional paragraph keeps complete source paint",
            || {
                let row = mounted.element(".editor-source-line");
                !row.has_attribute("data-paint-left")
                    && row.text_content().as_deref() == Some(bidi.as_str())
            },
        )
        .await;
        mounted.state.workspace.content.set("short 🦀\r\n".into());
        wait_until(
            "short replacement discards the old horizontal interval",
            || {
                mounted
                    .element(".editor-highlight-content")
                    .text_content()
                    .as_deref()
                    == Some("short 🦀\n")
            },
        )
        .await;
        assert!(
            openwebide_frontend::viewport::editor_scroll(&input)
                .scroll_left()
                .abs()
                < 0.5
        );
    }
}

#[wasm_bindgen_test]
async fn repeated_fragment_windows_reuse_validated_paint_and_font_changes_remeasure_in_both_modes()
{
    use openwebide_core::WorkspaceMode;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let source = "文😀e\u{301} words ".repeat(10_000);
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("cached.txt".into()));
            state.workspace.content.set(source.clone());
            view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:340px;height:380px">{editor_view(state)}</div> }
        });
        wait_until("cached horizontal paint", || {
            mounted
                .root
                .query_selector(".editor-source-line[data-paint-left]")
                .unwrap()
                .is_some()
        })
        .await;
        wait_until("initial source dimensions prepared", || {
            mounted
                .state
                .workspace
                .editor_rows
                .get_untracked()
                .is_some()
        })
        .await;
        frame().await;
        let input: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        let observer = js_sys::Function::new_no_args(r#"
            const state = {count: 0};
            const observer = new MutationObserver(records => {
                for (const record of records) for (const node of record.addedNodes)
                    if (node.nodeType === 1 && node.classList.contains('editor-row-measure')) state.count++;
            });
            observer.observe(document.body, {childList: true});
            state.stop = () => observer.disconnect();
            return state;
        "#).call0(&wasm_bindgen::JsValue::NULL).unwrap();
        let count = || {
            js_sys::Reflect::get(&observer, &"count".into())
                .unwrap()
                .as_f64()
                .unwrap()
        };
        // Warm both intervals after native scroll extents have settled.
        for _ in 0..2 {
            for x in [10_000.0, 0.0] {
                openwebide_frontend::viewport::set_editor_scroll_left(&input, x);
                input
                    .dispatch_event(&web_sys::Event::new("scroll").unwrap())
                    .unwrap();
                wait_until("cached window follows input", || {
                    let Some(left) = mounted
                        .root
                        .query_selector(".editor-source-line")
                        .unwrap()
                        .and_then(|row| row.get_attribute("data-paint-left"))
                        .and_then(|left| left.parse::<f64>().ok())
                    else {
                        return false;
                    };
                    if x == 0.0 {
                        left == 0.0
                    } else {
                        left > 9000.0 && left < 10_100.0
                    }
                })
                .await;
                settle().await;
            }
        }
        let before = count();
        for x in [10_000.0, 0.0] {
            openwebide_frontend::viewport::set_editor_scroll_left(&input, x);
            input
                .dispatch_event(&web_sys::Event::new("scroll").unwrap())
                .unwrap();
            wait_until("retained fragment revisited", || {
                let Some(left) = mounted
                    .root
                    .query_selector(".editor-source-line")
                    .unwrap()
                    .and_then(|row| row.get_attribute("data-paint-left"))
                    .and_then(|left| left.parse::<f64>().ok())
                else {
                    return false;
                };
                if x == 0.0 { left == 0.0 } else { left > 9000.0 }
            })
            .await;
            settle().await;
        }
        assert!(
            (count() - before).abs() < 0.5,
            "revisiting exact intervals must avoid styled probes: before={before} after={}",
            count()
        );
        input
            .unchecked_ref::<web_sys::HtmlElement>()
            .style()
            .set_property("letter-spacing", "1px")
            .unwrap();
        openwebide_frontend::viewport::set_editor_scroll_left(&input, 10_000.0);
        input
            .dispatch_event(&web_sys::Event::new("scroll").unwrap())
            .unwrap();
        wait_until("changed shaping invalidates fragments", || count() > before).await;
        openwebide_frontend::viewport::set_editor_scroll_left(&input, 0.0);
        input
            .dispatch_event(&web_sys::Event::new("scroll").unwrap())
            .unwrap();
        wait_until("shaped origin", || {
            mounted
                .root
                .query_selector(".editor-source-line")
                .unwrap()
                .and_then(|row| row.get_attribute("data-paint-left"))
                .as_deref()
                == Some("0")
        })
        .await;
        settle().await;
        let before_source = count();
        mounted
            .state
            .workspace
            .content
            .update(|source| source.replace_range(0..3, "界"));
        wait_until("source replacement rejects retained paint", || {
            mounted
                .root
                .query_selector(".editor-source-fragment")
                .unwrap()
                .is_some_and(|fragment| {
                    fragment
                        .text_content()
                        .is_some_and(|text| text.starts_with('界'))
                })
        })
        .await;
        assert!(
            count() > before_source,
            "changed source must be measured afresh"
        );

        js_sys::Reflect::get(&observer, &"stop".into())
            .unwrap()
            .dyn_into::<js_sys::Function>()
            .unwrap()
            .call0(&wasm_bindgen::JsValue::NULL)
            .unwrap();
    }
}

#[wasm_bindgen_test]
fn cold_geometry_cannot_populate_a_new_paint_scope_in_either_mode() {
    use openwebide_core::{
        WorkspaceMode,
        editor::{GlyphRectangle, Indentation, MeasuredRowGeometry, Selection, WrappedGeometry},
        highlight::{Language, highlight_lines},
    };
    use openwebide_frontend::state_actions::editor::{EditorActions, EditorFragmentCache};
    use std::sync::Arc;
    let source = "a".repeat(70_000);
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        for change in 0..12 {
            let action_slot = std::rc::Rc::new(std::cell::RefCell::new(None));
            let mounted = mount_test({
                let source = source.clone();
                let action_slot = action_slot.clone();
                move |state| {
                    state.seed_project();
                    state
                        .projects
                        .projects
                        .update(|projects| projects[0].mode = mode);
                    state.workspace.open_file.set(Some("scope.txt".into()));
                    state.workspace.content.set(source);
                    *action_slot.borrow_mut() = Some(EditorActions::new(state.workspace));
                    view! { <div/> }
                }
            });
            let actions = action_slot.borrow_mut().take().unwrap();
            actions.prepare_edit(Selection::caret(0)).unwrap();
            let tokens = Arc::new(if change == 11 {
                Vec::new()
            } else {
                openwebide_core::highlight::share_token_rows(highlight_lines(
                    &source,
                    Language::Plain,
                ))
            });
            let guides: Arc<[usize]> = Arc::from([0]);
            let mut metrics = "styled width/font".to_string();
            let mut cache = EditorFragmentCache::default();
            assert!(actions.fragment_scope(
                &mut cache,
                metrics.clone(),
                (false, tokens.clone()),
                guides.clone(),
                Indentation::default(),
                false
            ));
            let (paint, _) = actions
                .prepare_row_measurements(
                    metrics.clone(),
                    actions.projection().unwrap(),
                    (false, tokens.clone()),
                    guides.clone(),
                    Indentation::default(),
                    false,
                )
                .unwrap();
            let geometry = MeasuredRowGeometry::Wrapped(
                WrappedGeometry::new(
                    70_000,
                    100.0,
                    1000.0,
                    vec![
                        GlyphRectangle {
                            glyph: 0,
                            left: 0.0,
                            top: 2.0,
                            width: 8.0,
                            height: 15.0,
                        },
                        GlyphRectangle {
                            glyph: 69_999,
                            left: 0.0,
                            top: 982.0,
                            width: 8.0,
                            height: 15.0,
                        },
                    ],
                )
                .unwrap(),
            );
            actions.retain_preparation_geometry(&mut cache, &paint, 0, geometry.clone());
            assert!(actions.measured_row_geometry(&mut cache, 0).is_some());
            if change == 9 || change == 11 {
                let ticket = actions
                    .begin_row_preparation(paint.view_revision, 1)
                    .unwrap();
                assert!(
                    actions
                        .finish_row_preparation(
                            ticket,
                            paint.clone(),
                            Ok(Some(
                                openwebide_core::editor::MeasuredRows::layout([1000.0], [100.0])
                                    .unwrap()
                            ))
                        )
                        .is_none()
                );
                let equivalent = Arc::new(openwebide_core::highlight::share_token_rows(
                    highlight_lines(&source, Language::Plain),
                ));
                assert!(actions.fragment_scope(
                    &mut cache,
                    metrics.clone(),
                    (true, equivalent.clone()),
                    guides.clone(),
                    Indentation::default(),
                    false
                ));
                assert!(actions.measured_row_geometry(&mut cache, 0).is_some());
                actions.invalidate_measured_rows();
                let (_, plan) = actions
                    .prepare_row_measurements(
                        metrics.clone(),
                        actions.projection().unwrap(),
                        (true, equivalent.clone()),
                        guides.clone(),
                        Indentation::default(),
                        false,
                    )
                    .unwrap();
                assert_eq!(
                    plan.completed(),
                    1,
                    "identical pending/plain paint reuses measured dimensions"
                );
                assert!(actions.fragment_scope(
                    &mut cache,
                    metrics.clone(),
                    (true, equivalent),
                    guides.clone(),
                    Indentation::default(),
                    false
                ));
                assert!(
                    actions.measured_row_geometry(&mut cache, 0).is_some(),
                    "equivalent syntax paint followed by height reconciliation retains anchors"
                );
                if change == 11 {
                    use openwebide_core::highlight::{Token, TokenKind};
                    let split = Arc::new(vec![Arc::from(vec![
                        Token {
                            kind: TokenKind::Plain,
                            text: source[..100].into(),
                        },
                        Token {
                            kind: TokenKind::Plain,
                            text: source[100..].into(),
                        },
                    ])]);
                    let (_, plan) = actions
                        .prepare_row_measurements(
                            metrics.clone(),
                            actions.projection().unwrap(),
                            (true, split.clone()),
                            guides.clone(),
                            Indentation::default(),
                            false,
                        )
                        .unwrap();
                    assert_eq!(
                        plan.completed(),
                        0,
                        "changed shaping boundaries need new dimensions"
                    );
                    assert!(actions.fragment_scope(
                        &mut cache,
                        metrics.clone(),
                        (true, split),
                        guides.clone(),
                        Indentation::default(),
                        false,
                    ));
                    assert!(
                        actions.measured_row_geometry(&mut cache, 0).is_none(),
                        "same text with different token spans cannot reuse anchors"
                    );
                }
                continue;
            }
            if change == 8 {
                assert!(actions.fragment_scope(
                    &mut cache,
                    metrics.clone(),
                    (true, Arc::new((*tokens).clone())),
                    guides.clone(),
                    Indentation::default(),
                    false
                ));
                assert!(
                    actions.measured_row_geometry(&mut cache, 0).is_some(),
                    "equivalent syntax retains geometry without a wrapped height table"
                );
                actions.invalidate_measured_font();
                assert!(actions.fragment_scope(
                    &mut cache,
                    metrics.clone(),
                    (true, Arc::new((*tokens).clone())),
                    guides.clone(),
                    Indentation::default(),
                    false
                ));
                assert!(
                    actions.measured_row_geometry(&mut cache, 0).is_none(),
                    "font loading rejects geometry without height provenance"
                );
                continue;
            }
            if change == 6 {
                let ticket = actions
                    .begin_row_preparation(actions.view_revision(), 1)
                    .unwrap();
                assert!(
                    actions
                        .finish_row_preparation(
                            ticket,
                            paint.clone(),
                            Ok(Some(
                                openwebide_core::editor::MeasuredRows::new([1000.0]).unwrap()
                            ))
                        )
                        .is_none()
                );
                actions.invalidate_measured_rows();
                let equivalent = Arc::new(openwebide_core::highlight::share_token_rows(
                    highlight_lines(&source, Language::Plain),
                ));
                assert!(actions.fragment_scope(
                    &mut cache,
                    metrics.clone(),
                    (true, equivalent.clone()),
                    guides.clone(),
                    Indentation::default(),
                    false
                ));
                assert!(
                    (actions
                        .measured_row_geometry(&mut cache, 0)
                        .unwrap()
                        .height()
                        - 1000.0)
                        .abs()
                        < 0.001,
                    "identical styled rows reuse proven geometry"
                );
                let stale = MeasuredRowGeometry::Wrapped(
                    WrappedGeometry::new(
                        70_000,
                        100.0,
                        2000.0,
                        vec![
                            GlyphRectangle {
                                glyph: 0,
                                left: 0.0,
                                top: 2.0,
                                width: 8.0,
                                height: 15.0,
                            },
                            GlyphRectangle {
                                glyph: 69_999,
                                left: 0.0,
                                top: 982.0,
                                width: 8.0,
                                height: 15.0,
                            },
                        ],
                    )
                    .unwrap(),
                );
                actions.retain_preparation_geometry(&mut cache, &paint, 0, stale);
                assert!(
                    (actions
                        .measured_row_geometry(&mut cache, 0)
                        .unwrap()
                        .height()
                        - 1000.0)
                        .abs()
                        < 0.001,
                    "stale primitives cannot replace transferred geometry"
                );
                actions.invalidate_measured_font();
                assert!(actions.fragment_scope(
                    &mut cache,
                    metrics,
                    (true, equivalent),
                    guides,
                    Indentation::default(),
                    false
                ));
                assert!(
                    actions.measured_row_geometry(&mut cache, 0).is_none(),
                    "font loading clears measurement provenance even with identical font text"
                );
                continue;
            }
            match change {
                0 => mounted
                    .state
                    .workspace
                    .content
                    .set(format!("changed {source}")),
                1 => mounted
                    .state
                    .workspace
                    .editor_layout_epoch
                    .update(|epoch| *epoch += 1),
                2 => mounted
                    .state
                    .workspace
                    .editor_read_revision
                    .update(|revision| *revision += 1),
                3 => mounted
                    .state
                    .workspace
                    .pending_epoch
                    .update(|epoch| *epoch += 1),
                4 => mounted
                    .state
                    .auth
                    .generation
                    .update(|generation| *generation += 1),
                5 => mounted
                    .state
                    .workspace
                    .open_file
                    .set(Some("other.txt".into())),
                7 => metrics = "new font".into(),
                10 => mounted
                    .state
                    .workspace
                    .editor_font_epoch
                    .update(|epoch| *epoch += 1),
                _ => unreachable!(),
            }
            // A stale primitive must not populate a valid new scope, even when
            // the same file, source, token allocation or computed font survives.
            cache = EditorFragmentCache::default();
            actions.prepare_edit(Selection::caret(0)).unwrap();
            assert!(actions.fragment_scope(
                &mut cache,
                metrics,
                (false, tokens),
                guides,
                Indentation::default(),
                false
            ));
            actions.forget_measured_row_geometry(&mut cache, 0);
            actions.retain_preparation_geometry(&mut cache, &paint, 0, geometry);
            assert!(
                actions.measured_row_geometry(&mut cache, 0).is_none(),
                "{mode:?} stale scope {change}"
            );
        }
    }
}

#[wasm_bindgen_test]
async fn overflowing_file_tabs_keep_height_scroll_and_nodes_stable_in_both_modes() {
    use openwebide_core::WorkspaceMode;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state
                .workspace
                .open_file
                .set(Some("long-file-name-0.txt".into()));
            state.workspace.content.set("hello".into());
            for index in 0..20 {
                state
                    .workspace
                    .register_editor_tab(1, format!("long-file-name-{index}.txt"));
            }
            view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:300px;height:320px">{editor_view(state)}</div> }
        });
        wait_until("file tabs overflow horizontally", || {
            mounted
                .root
                .query_selector(".editor-file-tabs")
                .unwrap()
                .is_some_and(|tabs| tabs.scroll_width() > tabs.client_width())
        })
        .await;
        let tabs: web_sys::HtmlElement = mounted.element(".editor-file-tabs").unchecked_into();
        let first = mounted.element("[data-editor-tab='long-file-name-0.txt']");
        let badge = first.query_selector(".editor-tab-dirty").unwrap().unwrap();
        let width = first.get_bounding_client_rect().width();
        tabs.style().set_property("overflow-x", "scroll").unwrap();
        settle().await;
        let height = tabs.get_bounding_client_rect().height();
        tabs.set_scroll_left(120.0);
        // Force a classic scrollbar and vary the available width: a percentage
        // button height must not feed back into the strip's intrinsic height.
        tabs.style().set_property("overflow-x", "scroll").unwrap();
        for dirty in [true, false, true, false] {
            tabs.style()
                .set_property("max-width", if dirty { "250px" } else { "300px" })
                .unwrap();
            mounted.state.workspace.dirty.set(dirty);
            mounted
                .state
                .workspace
                .register_editor_tab(1, "long-file-name-0.txt".into());
            mounted
                .state
                .workspace
                .register_editor_tab(2, "other-project.txt".into());
            settle().await;
            assert!(first.is_same_node(Some(
                &mounted.element("[data-editor-tab='long-file-name-0.txt']")
            )));
            assert!(badge.is_same_node(Some(
                &first.query_selector(".editor-tab-dirty").unwrap().unwrap()
            )));
            assert!((first.get_bounding_client_rect().width() - width).abs() < 0.5);
            assert!((tabs.get_bounding_client_rect().height() - height).abs() < 0.5);
            assert!(
                tabs.scroll_height() <= tabs.client_height(),
                "file tabs must not overflow vertically"
            );
            assert!((tabs.scroll_left() - 120.0).abs() < 0.5);
            tabs.set_scroll_top(10.0);
            assert!(tabs.scroll_top().abs() < 0.5);
            assert_eq!(
                badge.get_attribute("aria-hidden").as_deref(),
                Some(if dirty { "false" } else { "true" })
            );
        }
    }
}

#[wasm_bindgen_test]
async fn source_slice_measurement_failure_restores_full_source_in_both_modes() {
    use openwebide_core::WorkspaceMode;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let source = "word 文😀\t ".repeat(15_000);
        let original = source.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("slice.txt".into()));
            state.workspace.content.set(source.clone());
            state
                .settings
                .editor_preferences
                .update(|preferences| preferences.word_wrap = true);
            view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:340px;height:380px">{editor_view(state)}</div> }
        });
        let input: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        wait_until("wrapped source anchors ready", || {
            mounted
                .root
                .query_selector(".editor-source-line[data-paint-top]")
                .unwrap()
                .is_some()
        })
        .await;
        for top in [10_000.0, 0.0] {
            openwebide_frontend::viewport::set_editor_scroll_top(&input, top);
            input
                .dispatch_event(&web_sys::Event::new("scroll").unwrap())
                .unwrap();
            wait_until("source anchor scope settled", || {
                mounted
                    .root
                    .query_selector(".editor-source-line")
                    .unwrap()
                    .and_then(|row| row.get_attribute("data-paint-top"))
                    .and_then(|top| top.parse::<f64>().ok())
                    .is_some_and(|top| {
                        (top - openwebide_frontend::viewport::editor_scroll(&input).scroll_top())
                            .abs()
                            < 200.0
                    })
            })
            .await;
            settle().await;
        }
        let audit = js_sys::Function::new_no_args(r#"
            const state = {failed: false};
            const old = Range.prototype.getClientRects;
            Range.prototype.getClientRects = function(...args) {
                const node = this.startContainer;
                const element = node.nodeType === 1 ? node : node.parentElement;
                if (!state.failed && element?.closest('.editor-row-measure .editor-source-line[data-source-start]')) {
                    state.failed = true;
                    return {length: 0, item() {return null;}};
                }
                return old.apply(this, args);
            };
            state.restore = () => {Range.prototype.getClientRects = old;};
            return state;
        "#).call0(&wasm_bindgen::JsValue::NULL).unwrap();
        openwebide_frontend::viewport::set_editor_scroll_top(&input, 20_000.0);
        input
            .dispatch_event(&web_sys::Event::new("scroll").unwrap())
            .unwrap();
        wait_until(
            "failed partial measurement restores complete source",
            || {
                js_sys::Reflect::get(&audit, &"failed".into())
                    .unwrap()
                    .as_bool()
                    == Some(true)
                    && mounted
                        .root
                        .query_selector(".editor-source-line")
                        .unwrap()
                        .and_then(|row| row.text_content())
                        .as_deref()
                        == Some(original.as_str())
            },
        )
        .await;
        js_sys::Reflect::get(&audit, &"restore".into())
            .unwrap()
            .dyn_into::<js_sys::Function>()
            .unwrap()
            .call0(&wasm_bindgen::JsValue::NULL)
            .unwrap();
        assert_editor_native_source(&input, mounted.state.workspace, &original);
        assert!(
            mounted
                .element(".editor-source-line")
                .get_attribute("data-source-start")
                .is_none()
        );
        openwebide_frontend::viewport::set_editor_scroll_top(&input, 30_000.0);
        input
            .dispatch_event(&web_sys::Event::new("scroll").unwrap())
            .unwrap();
        wait_until("fresh full probe restores bounded paint", || {
            mounted
                .root
                .query_selector(".editor-source-line")
                .unwrap()
                .and_then(|row| row.get_attribute("data-paint-top"))
                .and_then(|top| top.parse::<f64>().ok())
                .is_some_and(|top| {
                    (top - openwebide_frontend::viewport::editor_scroll(&input).scroll_top()).abs()
                        < 200.0
                })
        })
        .await;
        assert!(
            mounted
                .element(".editor-source-line")
                .text_content()
                .unwrap()
                .len()
                < 65_536
        );
        assert_editor_native_source(&input, mounted.state.workspace, &original);
    }
}

#[wasm_bindgen_test]
async fn cold_font_loading_waits_boundedly_and_rejects_old_jobs_in_both_modes() {
    struct FontLoadGuard(wasm_bindgen::JsValue);
    impl Drop for FontLoadGuard {
        fn drop(&mut self) {
            let _ = js_sys::Reflect::get(&self.0, &"restore".into())
                .unwrap()
                .unchecked_into::<js_sys::Function>()
                .call0(&wasm_bindgen::JsValue::NULL);
        }
    }
    for mode in [
        openwebide_core::WorkspaceMode::Local,
        openwebide_core::WorkspaceMode::Remote,
    ] {
        for outcome in ["loaded", "failed", "timeout", "source-change"] {
            let audit = FontLoadGuard(
                js_sys::Function::new_no_args(
                    r#"
                const fonts = document.fonts, check = fonts.check;
                fonts.check = () => true;
                const sink = window.__openwebideEditorProbeTiming;
                const state = {calls:0, ready:false, phases:[]};
                let resolve, reject;
                const pending = new Promise((yes, no) => { resolve = yes; reject = no; });
                const face = new FontFace('Monaspace Neon', 'url(data:font/woff2;base64,AA==)');
                Object.defineProperty(face, 'status', {get: () => state.ready ? (state.failed ? 'error' : 'loaded') : 'loading'});
                face.load = () => { ++state.calls; return pending; };
                fonts.add(face);
                state.complete = failed => {
                    state.ready = true; state.failed = failed;
                    if (failed) reject(new Error('fixture font load failure')); else resolve([]);
                    fonts.dispatchEvent(new Event(failed ? 'loadingerror' : 'loadingdone'));
                };
                window.__openwebideEditorProbeTiming = (paint, phase) => {
                    if (phase === 'render') state.phases.push({
                        font:paint.parentElement.dataset.measureFont,
                        view:paint.parentElement.dataset.measureView
                    });
                };
                state.restore = () => {
                    fonts.delete(face); fonts.check = check;
                    if (sink === undefined) delete window.__openwebideEditorProbeTiming;
                    else window.__openwebideEditorProbeTiming = sink;
                    state.ready = true; resolve([]);
                };
                return state;
            "#,
                )
                .call0(&wasm_bindgen::JsValue::NULL)
                .unwrap(),
            );
            let mounted = mount_test(move |state| {
                state.seed_project();
                state
                    .projects
                    .projects
                    .update(|projects| projects[0].mode = mode);
                state
                    .workspace
                    .open_file
                    .set(Some("loading-font.txt".into()));
                state
                    .workspace
                    .content
                    .set("initial words 文😀\r\n".repeat(400));
                view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:420px;height:300px">{editor_view(state)}</div> }
            });
            let actions = openwebide_frontend::state_actions::editor::EditorActions::new(
                mounted.state.workspace,
            );
            wait_until("font primitive started before allocating cold rows", || {
                js_sys::Reflect::get(&audit.0, &"calls".into())
                    .unwrap()
                    .as_f64()
                    .unwrap()
                    > 0.0
            })
            .await;
            let phases = js_sys::Reflect::get(&audit.0, &"phases".into())
                .unwrap()
                .unchecked_into::<js_sys::Array>();
            assert_eq!(
                phases.length(),
                0,
                "pending fonts must not allocate a fallback row probe immediately"
            );
            let input: web_sys::HtmlTextAreaElement =
                mounted.element(".editor-textarea").unchecked_into();
            let old_font = openwebide_frontend::viewport::editor_font_identity(&input);
            let old_revision = actions.view_revision();
            if outcome == "source-change" {
                mounted
                    .state
                    .workspace
                    .content
                    .set("replacement words 文😀\r\n".repeat(400));
            }
            if outcome != "timeout" {
                js_sys::Reflect::get(&audit.0, &"complete".into())
                    .unwrap()
                    .unchecked_into::<js_sys::Function>()
                    .call1(&wasm_bindgen::JsValue::NULL, &(outcome == "failed").into())
                    .unwrap();
            }
            if outcome != "timeout" {
                assert_ne!(
                    openwebide_frontend::viewport::editor_font_identity(&input),
                    old_font,
                    "actual font availability must change measurement identity even with identical CSS"
                );
            }
            wait_until(
                "font completion, failure or deadline permits current source geometry",
                || actions.measured_rows().is_some(),
            )
            .await;
            assert!(phases.length() > 0);
            if outcome == "loaded" || outcome == "failed" {
                assert!(
                    phases.iter().all(|phase| {
                        js_sys::Reflect::get(&phase, &"font".into())
                            .unwrap()
                            .as_string()
                            .as_deref()
                            != Some("0")
                    }),
                    "font notifications must obsolete the original cold job"
                );
            }
            if outcome == "source-change" {
                assert!(
                    phases.iter().all(|phase| {
                        js_sys::Reflect::get(&phase, &"view".into())
                            .unwrap()
                            .as_string()
                            != Some(old_revision.to_string())
                    }),
                    "source replaced during the wait cannot be measured by the old job"
                );
                assert!(actions.source().starts_with("replacement"));
            }
            if outcome == "timeout" {
                assert!(
                    !js_sys::Reflect::get(&audit.0, &"ready".into())
                        .unwrap()
                        .as_bool()
                        .unwrap(),
                    "fallback must work while the font request remains unresolved"
                );
            }
        }
    }
}

#[wasm_bindgen_test]
async fn settled_fonts_do_not_invalidate_initial_editor_geometry_in_both_modes() {
    JsFuture::from(
        js_sys::Function::new_no_args("return document.fonts.ready")
            .call0(&wasm_bindgen::JsValue::NULL)
            .unwrap()
            .unchecked_into::<js_sys::Promise>(),
    )
    .await
    .unwrap();
    for mode in [
        openwebide_core::WorkspaceMode::Local,
        openwebide_core::WorkspaceMode::Remote,
    ] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("fonts.txt".into()));
            state.workspace.content.set("hello".into());
            editor_view(state)
        });
        settle().await;
        settle().await;
        JsFuture::from(js_sys::Function::new_no_args("return new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))")
            .call0(&wasm_bindgen::JsValue::NULL).unwrap().unchecked_into::<js_sys::Promise>())
            .await.unwrap();
        assert_eq!(
            mounted.state.workspace.editor_font_epoch.get_untracked(),
            0,
            "already settled fonts must not discard initial geometry: {mode:?}"
        );
        for event in ["loadingdone", "loadingerror"] {
            let epoch = mounted.state.workspace.editor_font_epoch.get_untracked();
            js_sys::Function::new_with_args(
                "event",
                "document.fonts.dispatchEvent(new Event(event))",
            )
            .call1(&wasm_bindgen::JsValue::NULL, &event.into())
            .unwrap();
            wait_until("real font notification invalidates geometry", || {
                mounted.state.workspace.editor_font_epoch.get_untracked() > epoch
            })
            .await;
        }
    }
}

#[wasm_bindgen_test]
async fn cancellable_text_groups_unicode_multi_cursor_edits_and_keeps_native_fallback_in_both_modes()
 {
    use openwebide_core::editor::{Document, Selection};
    for mode in [
        openwebide_core::WorkspaceMode::Local,
        openwebide_core::WorkspaceMode::Remote,
    ] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("typing.txt".into()));
            state.workspace.content.set("a\r\na".into());
            let mut document = Document::new("a\r\na");
            document
                .set_selections(vec![Selection::caret(0), Selection::caret(3)])
                .unwrap();
            state.workspace.editor_documents.update(|documents| {
                documents.insert((1, "typing.txt".into()), document);
            });
            editor_view(state)
        });
        settle().await;
        let input: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        for text in ["文😀", "!"] {
            let init = web_sys::InputEventInit::new();
            init.set_bubbles(true);
            init.set_cancelable(true);
            init.set_input_type("insertText");
            init.set_data(Some(text));
            let before =
                web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();
            input.dispatch_event(&before).unwrap();
            assert!(!before.default_prevented());
            editorNativeInput(&input, text, "insertText", false);
            settle().await;
        }
        let expected = "文😀!a\r\n文😀!a";
        assert_eq!(mounted.state.workspace.content.get_untracked(), expected);
        assert_eq!(input.value(), expected.replace("\r\n", "\n"));
        editor_key(&input, "z", true, false);
        settle().await;
        assert_eq!(mounted.state.workspace.content.get_untracked(), "a\r\na");
        editor_key(&input, "y", true, false);
        settle().await;
        assert_eq!(mounted.state.workspace.content.get_untracked(), expected);
        let init = web_sys::InputEventInit::new();
        init.set_bubbles(true);
        init.set_input_type("insertText");
        init.set_data(Some("?"));
        let before = web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();
        input.dispatch_event(&before).unwrap();
        assert!(!before.default_prevented());
        assert_eq!(mounted.state.workspace.content.get_untracked(), expected);
        editorNativeInput(&input, "?", "insertText", false);
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            "文😀!?a\r\n文😀!?a"
        );
        let source_before = mounted.state.workspace.content.get_untracked();
        let input_before = input.value();
        let oversized = "x".repeat(openwebide_core::editor::MAX_EDITOR_LINE_BYTES + 1);
        init.set_cancelable(true);
        init.set_data(Some(&oversized));
        let before = web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();
        input.dispatch_event(&before).unwrap();
        assert!(!before.default_prevented());
        editorNativeInput(&input, &oversized, "insertText", false);
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            source_before
        );
        assert_eq!(input.value(), input_before);
        settle().await;
        assert!(
            mounted
                .root
                .query_selector(".editor-error[role='alert']")
                .unwrap()
                .is_some()
        );
    }
}

#[wasm_bindgen_test]
async fn bounded_native_declarations_keep_global_offsets_and_full_selection_edits_in_both_modes() {
    use openwebide_core::editor::{Document, Indentation, Selection, byte_to_textarea};
    use openwebide_frontend::state_actions::editor::{EditorActions, EditorCommand};
    let row = "row 文😀\r\n";
    let source = row.repeat(4000);
    let start = row.len() * 1200;
    let end = row.len() * 3000;
    for mode in [
        openwebide_core::WorkspaceMode::Local,
        openwebide_core::WorkspaceMode::Remote,
    ] {
        for backward in [false, true] {
            let source = source.clone();
            let original = source.clone();
            let selection = if backward {
                Selection {
                    anchor: end,
                    head: start,
                }
            } else {
                Selection {
                    anchor: start,
                    head: end,
                }
            };
            let mounted = mount_test(move |state| {
                state.seed_project();
                state
                    .projects
                    .projects
                    .update(|projects| projects[0].mode = mode);
                state
                    .workspace
                    .open_file
                    .set(Some("native-context.txt".into()));
                state.workspace.content.set(source.clone());
                let mut document = Document::new(source);
                document.set_selections(vec![selection]).unwrap();
                state.workspace.editor_documents.update(|documents| {
                    documents.insert((1, "native-context.txt".into()), document);
                });
                editor_view(state)
            });
            settle().await;
            let input: web_sys::HtmlTextAreaElement =
                mounted.element(".editor-textarea").unchecked_into();
            input
                .set_selection_range_with_direction(
                    byte_to_textarea(&original, start)
                        .unwrap()
                        .try_into()
                        .unwrap(),
                    byte_to_textarea(&original, end)
                        .unwrap()
                        .try_into()
                        .unwrap(),
                    if backward { "backward" } else { "forward" },
                )
                .unwrap();
            let init = web_sys::InputEventInit::new();
            init.set_bubbles(true);
            init.set_cancelable(true);
            init.set_input_type("insertText");
            init.set_data(Some("文😀"));
            let before =
                web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();
            input.dispatch_event(&before).unwrap();
            assert!(!before.default_prevented());
            let insertion = mounted
                .state
                .workspace
                .editor_text_insertion
                .get_untracked()
                .unwrap();
            assert!(insertion.projection.is_windowed());
            assert!(insertion.projection.text().len() <= 16 * 1024);
            assert!(insertion.projection.textarea_origin() > 0);
            assert_eq!(
                insertion.native_caret,
                byte_to_textarea(&original, start).unwrap() + "文😀".encode_utf16().count()
            );
            assert_eq!(insertion.selections, vec![selection]);
            assert!(insertion.retain_native_value);
            editorNativeInput(&input, "文😀", "insertText", false);
            let expected = format!("{}文😀{}", &original[..start], &original[end..]);
            assert_eq!(mounted.state.workspace.content.get_untracked(), expected);
            assert_eq!(input.value(), expected.replace("\r\n", "\n"));
            let actions = EditorActions::new(mounted.state.workspace);
            let caret = actions.selections(&expected)[0];
            assert_eq!(caret, Selection::caret(start + "文😀".len()));
            actions
                .command(EditorCommand::Undo, caret, Indentation::default())
                .unwrap()
                .unwrap();
            assert_eq!(mounted.state.workspace.content.get_untracked(), original);
            assert_eq!(actions.selections(&original), vec![selection]);
        }
    }
}

#[wasm_bindgen_test]
async fn bounded_native_declarations_validate_fold_origins_and_full_input_retention_in_both_modes()
{
    use openwebide_core::editor::{Document, EditError, FoldRange, Selection};
    use openwebide_frontend::state_actions::editor::EditorActions;
    let row = "row 文😀\r\n";
    let source = row.repeat(4000);
    let at = row.len() * 2000;
    for mode in [
        openwebide_core::WorkspaceMode::Local,
        openwebide_core::WorkspaceMode::Remote,
    ] {
        for change_prefix in [false, true] {
            let source = source.clone();
            let original = source.clone();
            let mounted = mount_test(move |state| {
                state.seed_project();
                state
                    .projects
                    .projects
                    .update(|projects| projects[0].mode = mode);
                state
                    .workspace
                    .open_file
                    .set(Some("native-context.txt".into()));
                state.workspace.content.set(source.clone());
                let mut document = Document::new(source);
                document.set_selections(vec![Selection::caret(at)]).unwrap();
                document.set_fold_ranges(vec![
                    FoldRange {
                        start_line: 0,
                        end_line: 2,
                    },
                    FoldRange {
                        start_line: 3800,
                        end_line: 3802,
                    },
                ]);
                document.fold_state_mut().toggle(0);
                state.workspace.editor_documents.update(|documents| {
                    documents.insert((1, "native-context.txt".into()), document);
                });
                view! { <div /> }
            });
            let actions = EditorActions::new(mounted.state.workspace);
            actions.begin_native_text("X".into());
            let insertion = mounted
                .state
                .workspace
                .editor_text_insertion
                .get_untracked()
                .unwrap();
            assert!(insertion.projection.is_windowed());
            assert!(
                !insertion.projection.is_folded(),
                "the nearby context excludes the distant folds"
            );
            assert!(
                !insertion.retain_native_value,
                "full folded input still requires reconciliation"
            );
            mounted
                .state
                .workspace
                .editor_documents
                .update(|documents| {
                    documents
                        .get_mut(&(1, "native-context.txt".into()))
                        .unwrap()
                        .fold_state_mut()
                        .toggle(if change_prefix { 0 } else { 3800 });
                });
            let result = actions.finish_native_text(
                Some("X"),
                Selection::caret(insertion.native_caret),
                1.0,
            );
            if change_prefix {
                assert_eq!(result, Err(EditError::StaleContext));
                assert_eq!(mounted.state.workspace.content.get_untracked(), original);
                assert_eq!(actions.selections(&original), vec![Selection::caret(at)]);
            } else {
                let commit = result.unwrap().unwrap();
                assert!(!commit.retain_native_value);
                assert_eq!(commit.selection, Selection::caret(at + 1));
                assert_eq!(
                    mounted.state.workspace.content.get_untracked(),
                    format!("{}X{}", &original[..at], &original[at..])
                );
            }
            assert!(
                mounted
                    .state
                    .workspace
                    .editor_text_insertion
                    .get_untracked()
                    .is_none()
            );
        }
    }
}

#[wasm_bindgen_test]
async fn declared_native_commits_keep_native_value_and_reject_stale_scopes_in_both_modes() {
    for mode in [
        openwebide_core::WorkspaceMode::Local,
        openwebide_core::WorkspaceMode::Remote,
    ] {
        for change in 0..8 {
            let mounted = mount_test(move |state| {
                state.seed_project();
                state
                    .projects
                    .projects
                    .update(|projects| projects[0].mode = mode);
                state.workspace.open_file.set(Some("native.txt".into()));
                state.workspace.content.set("hello".into());
                editor_view(state)
            });
            settle().await;
            let input: web_sys::HtmlTextAreaElement =
                mounted.element(".editor-textarea").unchecked_into();
            input.set_selection_range(0, 0).unwrap();
            let audit = js_sys::Function::new_with_args("input", r#"
                const descriptor = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value');
                const state = {sets: 0};
                Object.defineProperty(input, 'value', {configurable: true,
                    get() { return descriptor.get.call(this); },
                    set(value) { state.sets++; descriptor.set.call(this, value); }});
                return state;
            "#).call1(&wasm_bindgen::JsValue::NULL, &input).unwrap();
            let init = web_sys::InputEventInit::new();
            init.set_bubbles(true);
            init.set_cancelable(true);
            init.set_input_type("insertText");
            init.set_data(Some("X"));
            let before =
                web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();
            input.dispatch_event(&before).unwrap();
            assert!(!before.default_prevented());
            assert!(
                mounted
                    .state
                    .workspace
                    .editor_text_insertion
                    .get_untracked()
                    .is_some()
            );
            match change {
                1 => mounted.state.workspace.content.set("new source".into()),
                2 => mounted
                    .state
                    .workspace
                    .editor_read_revision
                    .update(|revision| *revision += 1),
                3 => mounted
                    .state
                    .auth
                    .generation
                    .update(|generation| *generation += 1),
                4 => mounted
                    .state
                    .workspace
                    .editor_fold_revision
                    .update(|revision| *revision += 1),
                6 => mounted
                    .state
                    .workspace
                    .pending_epoch
                    .update(|epoch| *epoch += 1),
                7 => mounted
                    .state
                    .workspace
                    .open_file
                    .set(Some("other.txt".into())),
                _ => {}
            }
            editorNativeInput(
                &input,
                if change == 5 { "Y" } else { "X" },
                "insertText",
                false,
            );
            settle().await;
            assert_eq!(
                mounted.state.workspace.content.get_untracked(),
                match change {
                    0 | 4 => "Xhello",
                    1 => "new source",
                    5 => "Yhello",
                    _ => "hello",
                }
            );
            if change == 0 || change == 4 {
                assert_eq!(input.value(), "Xhello");
                assert_eq!(
                    js_sys::Reflect::get(&audit, &"sets".into())
                        .unwrap()
                        .as_f64(),
                    Some(0.0),
                    "native insertion must not reset the whole textarea value"
                );
            }
            js_sys::Function::new_with_args("input", "delete input.value")
                .call1(&wasm_bindgen::JsValue::NULL, &input)
                .unwrap();
        }
    }
}

#[wasm_bindgen_test]
async fn trusted_native_commits_skip_full_value_reads_and_other_inputs_reconcile_in_both_modes() {
    use openwebide_core::editor::{Document, Selection};
    for mode in [
        openwebide_core::WorkspaceMode::Local,
        openwebide_core::WorkspaceMode::Remote,
    ] {
        for multiple in [false, true] {
            let mounted = mount_test(move |state| {
                state.seed_project();
                state
                    .projects
                    .projects
                    .update(|projects| projects[0].mode = mode);
                state
                    .workspace
                    .open_file
                    .set(Some("native-read.txt".into()));
                state.workspace.content.set("a\r\na".into());
                let mut document = Document::new("a\r\na");
                document
                    .set_selections(if multiple {
                        vec![Selection::caret(0), Selection::caret(3)]
                    } else {
                        vec![Selection::caret(0)]
                    })
                    .unwrap();
                state.workspace.editor_documents.update(|documents| {
                    documents.insert((1, "native-read.txt".into()), document);
                });
                editor_view(state)
            });
            settle().await;
            let input: web_sys::HtmlTextAreaElement =
                mounted.element(".editor-textarea").unchecked_into();
            input.focus().unwrap();
            input.set_selection_range(0, 0).unwrap();
            let result = js_sys::Function::new_with_args("input", r#"
                const descriptor = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value');
                const audit = {reads: 0, trusted: false};
                Object.defineProperty(input, 'value', {configurable: true,
                    get() { audit.reads++; return descriptor.get.call(this); },
                    set(value) { descriptor.set.call(this, value); }});
                const capture = event => { audit.reads = 0; audit.trusted = event.isTrusted; };
                input.addEventListener('input', capture, true);
                input.dispatchEvent(new InputEvent('beforeinput', {
                    bubbles: true, cancelable: true, inputType: 'insertText', data: '文😀'
                }));
                const inserted = document.execCommand('insertText', false, '文😀');
                const result = {inserted, ...audit};
                input.removeEventListener('input', capture, true);
                delete input.value;
                return result;
            "#).call1(&wasm_bindgen::JsValue::NULL, &input).unwrap();
            assert_eq!(
                js_sys::Reflect::get(&result, &"inserted".into())
                    .unwrap()
                    .as_bool(),
                Some(true)
            );
            assert_eq!(
                js_sys::Reflect::get(&result, &"trusted".into())
                    .unwrap()
                    .as_bool(),
                Some(true)
            );
            let reads = js_sys::Reflect::get(&result, &"reads".into())
                .unwrap()
                .as_f64();
            if multiple {
                assert!(
                    reads.is_some_and(|reads| reads > 0.0),
                    "multiple cursors must reconcile their additional edits"
                );
            } else {
                assert_eq!(
                    reads,
                    Some(0.0),
                    "trusted native commit must not read the full DOM value"
                );
            }
            settle().await;
            assert_eq!(
                mounted.state.workspace.content.get_untracked(),
                if multiple {
                    "文😀a\r\n文😀a"
                } else {
                    "文😀a\r\na"
                }
            );
            assert_eq!(
                input.value(),
                if multiple {
                    "文😀a\n文😀a"
                } else {
                    "文😀a\na"
                }
            );
            // Synthetic input has no browser-owned mutation guarantee and must
            // retain full-value reconciliation even when its declared data matches.
            let reads = js_sys::Function::new_with_args("input", r#"
                const descriptor = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value');
                let reads = 0;
                Object.defineProperty(input, 'value', {configurable: true,
                    get() { reads++; return descriptor.get.call(this); },
                    set(value) { descriptor.set.call(this, value); }});
                input.dispatchEvent(new InputEvent('beforeinput', {
                    bubbles: true, cancelable: true, inputType: 'insertText', data: '!'
                }));
                input.setRangeText('!', input.selectionStart, input.selectionEnd, 'end');
                reads = 0;
                input.dispatchEvent(new InputEvent('input', {bubbles: true, inputType: 'insertText', data: '!'}));
                delete input.value;
                return reads;
            "#).call1(&wasm_bindgen::JsValue::NULL, &input).unwrap().as_f64().unwrap();
            assert!(
                reads > 0.0,
                "synthetic events must reconcile the complete value"
            );
        }
    }
}

#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
export function watchNativeExtentReads(input) {
    let reads = 0;
    for (const name of ['scrollWidth', 'scrollHeight']) {
        const getter = Object.getOwnPropertyDescriptor(Element.prototype, name).get;
        Object.defineProperty(input, name, {configurable:true, get() { ++reads; return getter.call(this); }});
    }
    return () => reads;
}
export function fullNativeDimensions(input, source) {
    const probe = input.cloneNode(false);
    probe.removeAttribute('data-editor-native-bound');
    probe.removeAttribute('data-editor-native-generation');
    probe.style.transform = 'none';
    probe.style.visibility = 'hidden';
    input.parentElement.append(probe);
    try {
        probe.value = source;
        return [probe.scrollWidth, probe.scrollHeight];
    } finally { probe.remove(); }
}
export function restoreNativeExtentReads(input) { delete input.scrollWidth; delete input.scrollHeight; }
export async function editorFontsReady() { document.body.getBoundingClientRect(); await document.fonts.ready; await new Promise(resolve => requestAnimationFrame(resolve)); }
export async function loadEditorFont(name, bytes) {
    const font = new FontFace('Monaspace ' + name, bytes, {weight:'200 800', stretch:'100% 125%', style:'oblique -11deg 0deg'});
    await font.load(); document.fonts.add(font); return font;
}
export function removeEditorFont(font) { document.fonts.delete(font); }
"#)]
extern "C" {
    fn watchNativeExtentReads(input: &web_sys::HtmlTextAreaElement) -> js_sys::Function;
    fn fullNativeDimensions(input: &web_sys::HtmlTextAreaElement, source: &str) -> Vec<i32>;
    fn restoreNativeExtentReads(input: &web_sys::HtmlTextAreaElement);
    #[wasm_bindgen(catch)]
    async fn editorFontsReady() -> Result<(), wasm_bindgen::JsValue>;
    #[wasm_bindgen(catch)]
    async fn loadEditorFont(
        name: &str,
        bytes: &[u8],
    ) -> Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue>;
    fn removeEditorFont(font: &wasm_bindgen::JsValue);
}

#[wasm_bindgen_test]
async fn switching_loaded_font_families_invalidates_source_layout_in_both_modes() {
    use openwebide_core::{WorkspaceMode, editor::EditorFont};
    let neon = loadEditorFont(
        "Neon",
        include_bytes!("../../fonts/MonaspaceNeon-v1.400.woff2"),
    )
    .await
    .unwrap();
    let radon = loadEditorFont(
        "Radon",
        include_bytes!("../../fonts/MonaspaceRadon-v1.400.woff2"),
    )
    .await
    .unwrap();
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state
                .workspace
                .open_file
                .set(Some("font-layout.txt".into()));
            state.workspace.content.set(
                "!= => === 文😀
"
                .repeat(160),
            );
            state
                .settings
                .editor_preferences
                .update(|preferences| preferences.word_wrap = false);
            view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:420px;height:300px">{editor_view(state)}</div> }
        });
        frame().await;
        wait_until("initial source font dimensions", || {
            mounted
                .state
                .workspace
                .editor_rows
                .get_untracked()
                .is_some()
        })
        .await;
        let before = mounted.state.workspace.editor_rows.get_untracked().unwrap();
        let epoch = mounted.state.workspace.editor_font_epoch.get_untracked();
        let visible_paint = openwebide_frontend::components::viewport_highlight_count();
        let textarea: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        let identity = openwebide_frontend::viewport::editor_font_identity(&textarea);
        mounted
            .state
            .settings
            .editor_preferences
            .update(|preferences| preferences.font = EditorFont::Radon);
        wait_until("new loaded font dimensions", || {
            mounted.state.workspace.editor_font_epoch.get_untracked() > epoch
                && mounted
                    .state
                    .workspace
                    .editor_rows
                    .get_untracked()
                    .is_some_and(|rows| rows.metrics != before.metrics)
        })
        .await;
        frame().await;
        assert_ne!(
            openwebide_frontend::viewport::editor_font_identity(&textarea),
            identity
        );
        wait_until("settled source extent after font loading", || {
            mounted
                .state
                .workspace
                .editor_rows
                .get_untracked()
                .is_some_and(|rows| {
                    rows.metrics != before.metrics
                        && openwebide_frontend::components::viewport_highlight_count()
                            > visible_paint
                        && mounted
                            .element(".editor-scroll-extent")
                            .get_attribute("data-editor-view")
                            == Some(rows.revision.to_string())
                })
        })
        .await;
        let rows = mounted.state.workspace.editor_rows.get_untracked().unwrap();
        let scroll = openwebide_frontend::viewport::editor_scroll(&textarea);
        assert!((f64::from(scroll.scroll_height()) - rows.rows.height() - 24.0).abs() <= 2.0);
        assert!(
            mounted
                .state
                .settings
                .editor_preferences
                .get_untracked()
                .texture_healing
        );
        assert!(
            mounted
                .state
                .settings
                .editor_preferences
                .get_untracked()
                .ligatures
        );
    }
    removeEditorFont(&neon);
    removeEditorFont(&radon);
}

#[wasm_bindgen_test]
async fn monaspace_families_and_independent_features_share_native_and_paint_metrics_in_both_modes()
{
    use openwebide_core::{WorkspaceMode, editor::EditorFont};
    let mut loaded = Vec::new();
    for (font, bytes) in [
        (
            EditorFont::Neon,
            include_bytes!("../../fonts/MonaspaceNeon-v1.400.woff2").as_slice(),
        ),
        (
            EditorFont::Argon,
            include_bytes!("../../fonts/MonaspaceArgon-v1.400.woff2").as_slice(),
        ),
        (
            EditorFont::Xenon,
            include_bytes!("../../fonts/MonaspaceXenon-v1.400.woff2").as_slice(),
        ),
        (
            EditorFont::Radon,
            include_bytes!("../../fonts/MonaspaceRadon-v1.400.woff2").as_slice(),
        ),
        (
            EditorFont::Krypton,
            include_bytes!("../../fonts/MonaspaceKrypton-v1.400.woff2").as_slice(),
        ),
    ] {
        loaded.push(loadEditorFont(font.name(), bytes).await.unwrap());
    }
    let source = "fn example() { let result = a != b && c <= d; }\n// 文😀 => -> ===\n";
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("font.rs".into()));
            state.workspace.content.set(source.into());
            view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:700px;height:400px">{editor_view(state)}</div> }
        });
        for font in EditorFont::ALL {
            for healing in [false, true] {
                for ligatures in [false, true] {
                    mounted
                        .state
                        .settings
                        .editor_preferences
                        .update(|preferences| {
                            preferences.font = font;
                            preferences.texture_healing = healing;
                            preferences.ligatures = ligatures;
                        });
                    settle().await;
                    let input = mounted
                        .root
                        .query_selector(".editor-textarea")
                        .unwrap()
                        .unwrap()
                        .unchecked_into::<web_sys::HtmlTextAreaElement>();
                    let native = web_sys::window()
                        .unwrap()
                        .get_computed_style(&input)
                        .unwrap()
                        .unwrap();
                    assert!(
                        native
                            .get_property_value("font-family")
                            .unwrap()
                            .contains(font.name())
                    );
                    let features = native.get_property_value("font-feature-settings").unwrap();
                    assert!(features.contains(if healing { "\"calt\"" } else { "\"calt\" 0" }));
                    assert!(features.contains(if ligatures { "\"ss01\"" } else { "\"ss01\" 0" }));
                    for selector in [".editor-highlight", ".editor-scroll-surface"] {
                        let element = mounted.root.query_selector(selector).unwrap().unwrap();
                        let painted = web_sys::window()
                            .unwrap()
                            .get_computed_style(&element)
                            .unwrap()
                            .unwrap();
                        for property in [
                            "font-family",
                            "font-size",
                            "font-feature-settings",
                            "font-variant-ligatures",
                        ] {
                            assert_eq!(
                                native.get_property_value(property).unwrap(),
                                painted.get_property_value(property).unwrap(),
                                "{font:?} {property}"
                            );
                        }
                    }
                    assert_eq!(input.value(), source);
                    let end = u32::try_from(source.encode_utf16().count()).unwrap();
                    input.set_selection_range(end, end).unwrap();
                    input
                        .dispatch_event(&web_sys::Event::new("select").unwrap())
                        .unwrap();
                    settle().await;
                    assert_eq!(
                        openwebide_frontend::state_actions::editor::EditorActions::new(
                            mounted.state.workspace
                        )
                        .selection(source)
                        .unwrap()
                        .head,
                        source.len()
                    );
                    assert_eq!(
                        input.selection_start().unwrap(),
                        Some(u32::try_from(source.encode_utf16().count()).unwrap())
                    );
                    assert_eq!(mounted.state.workspace.content.get_untracked(), source);
                    assert!(!mounted.state.workspace.dirty.get_untracked());
                }
            }
        }
    }
    for font in loaded {
        removeEditorFont(&font);
    }
}

#[wasm_bindgen_test]
async fn file_tab_context_actions_share_tree_actions_and_guard_bulk_closes_in_both_modes() {
    use openwebide_frontend::state_actions::{
        file_tree::FileTreeActions, workspace::WorkspaceActions,
    };
    for mode in [
        openwebide_core::WorkspaceMode::Local,
        openwebide_core::WorkspaceMode::Remote,
    ] {
        let slot = std::rc::Rc::new(std::cell::Cell::new(None::<WorkspaceActions>));
        let captured = slot.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            for path in ["a.txt", "b.txt", "c.txt"] {
                state.workspace.register_editor_tab(1, path.into());
                state.workspace.open_file.set(Some(path.into()));
                state.workspace.content.set(path.into());
                state.workspace.retain_editor_buffer(false);
            }
            state.workspace.open_file.set(Some("a.txt".into()));
            state.workspace.content.set("a.txt".into());
            let view = editor_view(state.clone());
            captured.set(Some(expect_context::<WorkspaceActions>()));
            expect_context::<FileTreeActions>()
                .send_prompt
                .set(Some(Callback::new(move |prompt| {
                    state.chat.notice.set(Some(prompt));
                })));
            view
        });
        frame().await;
        let actions = slot.get().unwrap();
        let tab = mounted.element("[data-editor-tab='a.txt']");
        tab.dispatch_event(&web_sys::MouseEvent::new("contextmenu").unwrap())
            .unwrap();
        settle().await;
        for label in [
            "Rename",
            "Move",
            "Delete",
            "Stage",
            "Explain in chat",
            "Close others",
            "Move right",
        ] {
            assert!(
                mounted
                    .element(".ui-dropdown-menu")
                    .text_content()
                    .unwrap()
                    .contains(label)
            );
        }
        mounted.click_text("Move right");
        settle().await;
        assert_eq!(
            mounted.state.workspace.editor_tabs.get_untracked()[&1],
            ["b.txt", "a.txt", "c.txt"]
        );
        assert_eq!(
            mounted.state.workspace.open_file.get_untracked().as_deref(),
            Some("a.txt")
        );
        mounted
            .element("[data-editor-tab='a.txt']")
            .dispatch_event(&web_sys::MouseEvent::new("contextmenu").unwrap())
            .unwrap();
        settle().await;
        mounted.click_text("Explain in chat");
        assert_eq!(
            mounted.state.chat.notice.get_untracked().as_deref(),
            Some("Explain how this works: @file:\"a.txt\"")
        );
        mounted
            .state
            .workspace
            .editor_buffers
            .update(|buffers| buffers.get_mut(&(1, "b.txt".into())).unwrap().dirty = true);
        actions.file_tab_action.run((
            "a.txt".into(),
            openwebide_frontend::tabs::TabAction::CloseOthers,
        ));
        let confirmation = mounted.state.ui.confirm.get_untracked().unwrap();
        assert!(confirmation.message.contains("b.txt"));
        assert_eq!(
            mounted.state.workspace.editor_tabs.get_untracked()[&1].len(),
            3
        );
        mounted.state.workspace.editor_buffers.update(|buffers| {
            buffers
                .get_mut(&(1, "b.txt".into()))
                .unwrap()
                .content
                .push('!');
        });
        confirmation.action.run(());
        assert_eq!(
            mounted.state.workspace.editor_tabs.get_untracked()[&1].len(),
            3
        );
        mounted.state.ui.confirm.set(None);
        actions.file_tab_action.run((
            "a.txt".into(),
            openwebide_frontend::tabs::TabAction::CloseOthers,
        ));
        mounted
            .state
            .ui
            .confirm
            .get_untracked()
            .unwrap()
            .action
            .run(());
        assert_eq!(
            mounted.state.workspace.editor_tabs.get_untracked()[&1],
            ["a.txt"]
        );
        assert_eq!(mounted.state.workspace.content.get_untracked(), "a.txt");
        mounted
            .state
            .workspace
            .register_editor_tab(1, "b.txt".into());
        mounted
            .state
            .workspace
            .register_editor_tab(1, "c.txt".into());
        settle().await;
        mounted
            .element("[data-editor-tab='a.txt']")
            .dispatch_event(&web_sys::MouseEvent::new("contextmenu").unwrap())
            .unwrap();
        settle().await;
        let stale_move = mounted.element(".ui-dropdown-menu button:nth-of-type(5)");
        mounted
            .state
            .auth
            .generation
            .update(|generation| *generation += 1);
        stale_move.click();
        assert_eq!(
            mounted.state.workspace.editor_tabs.get_untracked()[&1],
            ["a.txt", "b.txt", "c.txt"]
        );
    }
}

#[wasm_bindgen_test]
async fn bounded_native_context_replaces_complete_selections_and_preserves_history_in_both_modes() {
    use openwebide_core::{WorkspaceMode, editor::Selection};
    use openwebide_frontend::state_actions::editor::{EditorActions, EditorCommand};
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        for backward in [false, true] {
            let original = format!("before\r\n{}after\r\nlast", "row α🦀 kept\r\n".repeat(2000));
            let start = original.find('α').unwrap();
            let end = original.rfind("kept").unwrap() + 4;
            let selected = if backward {
                Selection {
                    anchor: end,
                    head: start,
                }
            } else {
                Selection {
                    anchor: start,
                    head: end,
                }
            };
            let source = original.clone();
            let slot = std::rc::Rc::new(std::cell::Cell::new(None::<EditorActions>));
            let captured = slot.clone();
            let mounted = mount_test(move |state| {
                state.seed_project();
                state
                    .projects
                    .projects
                    .update(|projects| projects[0].mode = mode);
                state.workspace.open_file.set(Some("context.txt".into()));
                state.workspace.content.set(source.clone());
                let actions = EditorActions::new(state.workspace);
                actions.record_selection(selected).unwrap();
                captured.set(Some(actions));
                view! { <div /> }
            });
            let actions = slot.get().unwrap();
            let context = actions.native_context().unwrap();
            assert!(context.projection().is_windowed());
            assert!(context.projection().textarea_text().len() <= 16 * 1024);
            let native = context.native_selection().unwrap();
            assert_eq!(context.source_selection(native).unwrap(), selected);
            assert_eq!(actions.selection(&original), Some(selected));
            let range = native.range();
            let native_start = openwebide_core::editor::utf16_to_byte(
                context.projection().textarea_text(),
                range.start,
            );
            let native_end = openwebide_core::editor::utf16_to_byte(
                context.projection().textarea_text(),
                range.end,
            );
            let mut value = context.projection().textarea_text().to_string();
            value.replace_range(native_start..native_end, "日\n🦀");
            actions
                .native_context_input(
                    &context,
                    &value,
                    Selection::caret(native_start + "日\n🦀".len()),
                    "insertReplacementText",
                    1.0,
                )
                .unwrap();
            let expected = format!("{}日\r\n🦀{}", &original[..start], &original[end..]);
            assert_eq!(mounted.state.workspace.content.get_untracked(), expected);
            assert!(mounted.state.workspace.dirty.get_untracked());
            let after = actions.selection(&expected).unwrap();
            assert_eq!(after, Selection::caret(start + "日\r\n🦀".len()));
            actions
                .command(EditorCommand::Undo, after, actions.rules().indentation)
                .unwrap();
            assert_eq!(mounted.state.workspace.content.get_untracked(), original);
            assert_eq!(actions.selection(&original), Some(selected));
            actions
                .command(EditorCommand::Redo, selected, actions.rules().indentation)
                .unwrap();
            assert_eq!(mounted.state.workspace.content.get_untracked(), expected);
            assert_eq!(actions.selection(&expected), Some(after));
            assert!(
                actions
                    .native_context_input(&context, &value, Selection::caret(0), "insertText", 4.0)
                    .is_err()
            );
            assert_eq!(mounted.state.workspace.content.get_untracked(), expected);
        }
    }
}

#[wasm_bindgen_test]
async fn bounded_native_context_rejects_stale_source_selection_and_owner_scopes_in_both_modes() {
    use openwebide_core::{
        WorkspaceMode,
        editor::{Document, EditError, Selection},
    };
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        for changed in 0..9 {
            let original = "α🦀 row\r\n".repeat(3000);
            let source = original.clone();
            let slot = std::rc::Rc::new(std::cell::Cell::new(None::<EditorActions>));
            let captured = slot.clone();
            let mounted = mount_test(move |state| {
                state.seed_project();
                state
                    .projects
                    .projects
                    .update(|projects| projects[0].mode = mode);
                state.workspace.open_file.set(Some("scope.txt".into()));
                state.workspace.content.set(source.clone());
                let actions = EditorActions::new(state.workspace);
                actions
                    .record_selection(Selection::caret(source.len()))
                    .unwrap();
                captured.set(Some(actions));
                view! { <div /> }
            });
            let actions = slot.get().unwrap();
            let context = actions.native_context().unwrap();
            match changed {
                0 => mounted.state.workspace.content.set(format!("{original}x")),
                1 => actions.record_selection(Selection::caret(0)).unwrap(),
                2 => mounted
                    .state
                    .workspace
                    .open_file
                    .set(Some("other.txt".into())),
                3 => mounted.state.workspace.active_project.set(Some(2)),
                4 => mounted
                    .state
                    .workspace
                    .editor_read_revision
                    .update(|revision| *revision += 1),
                5 => mounted
                    .state
                    .workspace
                    .pending_epoch
                    .update(|epoch| *epoch += 1),
                6 => mounted
                    .state
                    .auth
                    .generation
                    .update(|generation| *generation += 1),
                7 => mounted
                    .state
                    .workspace
                    .editor_documents
                    .update(|documents| {
                        let mut replacement = Document::new(original.clone());
                        replacement
                            .set_selections(vec![Selection::caret(original.len())])
                            .unwrap();
                        documents.insert((1, "scope.txt".into()), replacement);
                    }),
                _ => {
                    mounted
                        .state
                        .workspace
                        .editor_documents
                        .update(|documents| {
                            documents
                                .get_mut(&(1, "scope.txt".into()))
                                .unwrap()
                                .set_fold_ranges(vec![openwebide_core::editor::FoldRange {
                                    start_line: 0,
                                    end_line: 1,
                                }]);
                        });
                    actions.fold_command(openwebide_core::editor::FoldCommand::Toggle(0));
                }
            }
            let before = mounted.state.workspace.content.get_untracked();
            let dirty = mounted.state.workspace.dirty.get_untracked();
            let mut value = context.projection().textarea_text().to_string();
            value.push('x');
            assert_eq!(
                actions.native_context_input(
                    &context,
                    &value,
                    Selection::caret(value.len()),
                    "insertText",
                    1.0
                ),
                Err(EditError::StaleContext)
            );
            assert_eq!(mounted.state.workspace.content.get_untracked(), before);
            assert_eq!(mounted.state.workspace.dirty.get_untracked(), dirty);
        }
    }
}

#[wasm_bindgen_test]
async fn bounded_native_context_multicursor_replay_and_failures_share_source_contract_in_both_modes()
 {
    use openwebide_core::{
        WorkspaceMode,
        editor::{EditError, Selection},
    };
    use openwebide_frontend::state_actions::editor::{EditorActions, EditorCommand};
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let body = "α🦀 row\r\n".repeat(3000);
        let original = format!("A:{body}:B:ZZZ");
        let selections = vec![
            Selection {
                anchor: 2,
                head: 2 + body.len(),
            },
            Selection {
                anchor: original.len() - 3,
                head: original.len(),
            },
        ];
        let source = original.clone();
        let selected = selections.clone();
        let slot = std::rc::Rc::new(std::cell::Cell::new(None::<EditorActions>));
        let captured = slot.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("multiple.txt".into()));
            state.workspace.content.set(source.clone());
            let actions = EditorActions::new(state.workspace);
            actions.record_selection(selected[0]).unwrap();
            state.workspace.editor_documents.update(|documents| {
                documents
                    .get_mut(&(1, "multiple.txt".into()))
                    .unwrap()
                    .set_selections(selected.clone())
                    .unwrap();
            });
            captured.set(Some(actions));
            view! { <div /> }
        });
        let actions = slot.get().unwrap();
        let context = actions.native_context().unwrap();
        let range = context.native_selection().unwrap().range();
        let native_start = openwebide_core::editor::utf16_to_byte(
            context.projection().textarea_text(),
            range.start,
        );
        let native_end =
            openwebide_core::editor::utf16_to_byte(context.projection().textarea_text(), range.end);
        let mut value = context.projection().textarea_text().to_string();
        value.replace_range(native_start..native_end, "日\n🦀");
        let mut invalid = value.clone();
        invalid.push('!');
        assert_eq!(
            actions.native_context_input(
                &context,
                &invalid,
                Selection::caret(native_start + "日\n🦀".len()),
                "insertText",
                1.0
            ),
            Err(EditError::InvalidRange)
        );
        assert_eq!(mounted.state.workspace.content.get_untracked(), original);
        assert!(!mounted.state.workspace.dirty.get_untracked());
        assert_eq!(actions.selections(&original), selections);
        actions
            .native_context_input(
                &context,
                &value,
                Selection::caret(native_start + "日\n🦀".len()),
                "insertText",
                2.0,
            )
            .unwrap();
        assert_eq!(
            mounted.state.workspace.content.get_untracked(),
            "A:日\r\n🦀:B:日\r\n🦀"
        );
        actions
            .command(
                EditorCommand::Undo,
                actions.selection(&actions.source()).unwrap(),
                actions.rules().indentation,
            )
            .unwrap();
        assert_eq!(mounted.state.workspace.content.get_untracked(), original);
        assert_eq!(actions.selections(&original), selections);
        assert!(!mounted.state.workspace.dirty.get_untracked());
    }
}

#[wasm_bindgen_test]
async fn bounded_native_context_keeps_composition_values_through_clipped_replacements_in_both_modes()
 {
    use openwebide_core::{WorkspaceMode, editor::Selection};
    use openwebide_frontend::state_actions::editor::{EditorActions, EditorCommand};
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        for backward in [false, true] {
            let original = format!("before\r\n{}after\r\nlast", "row α🦀 kept\r\n".repeat(3000));
            let begin = original.find('α').unwrap();
            let end = original.rfind("kept").unwrap() + 4;
            let selected = if backward {
                Selection {
                    anchor: end,
                    head: begin,
                }
            } else {
                Selection {
                    anchor: begin,
                    head: end,
                }
            };
            let source = original.clone();
            let slot = std::rc::Rc::new(std::cell::Cell::new(None::<EditorActions>));
            let captured = slot.clone();
            let mounted = mount_test(move |state| {
                state.seed_project();
                state
                    .projects
                    .projects
                    .update(|projects| projects[0].mode = mode);
                state
                    .workspace
                    .open_file
                    .set(Some("composition.txt".into()));
                state.workspace.content.set(source.clone());
                let actions = EditorActions::new(state.workspace);
                actions.record_selection(selected).unwrap();
                captured.set(Some(actions));
                view! { <div /> }
            });
            let actions = slot.get().unwrap();
            actions.begin_composition();
            let context = actions.native_context().unwrap();
            let range = context.native_selection().unwrap().range();
            let start = openwebide_core::editor::utf16_to_byte(
                context.projection().textarea_text(),
                range.start,
            );
            let stop = openwebide_core::editor::utf16_to_byte(
                context.projection().textarea_text(),
                range.end,
            );
            let mut value = context.projection().textarea_text().to_string();
            value.replace_range(start..stop, "日");
            let first = actions
                .native_context_input(
                    &context,
                    &value,
                    Selection::caret(start + "日".len()),
                    "insertCompositionText",
                    1.0,
                )
                .unwrap();
            assert!(first.retain_native_value);
            assert_eq!(first.context.projection().textarea_text(), value);
            assert!(first.context.projection().textarea_text().len() <= 16 * 1024);
            assert_eq!(
                mounted.state.workspace.content.get_untracked(),
                format!("{}日{}", &original[..begin], &original[end..])
            );
            value.replace_range(start..start + "日".len(), "文🦀");
            let second = actions
                .native_context_input(
                    &first.context,
                    &value,
                    Selection::caret(start + "文🦀".len()),
                    "insertCompositionText",
                    2.0,
                )
                .unwrap();
            assert!(second.retain_native_value);
            assert_eq!(second.context.projection().textarea_text(), value);
            assert_eq!(
                mounted.state.workspace.content.get_untracked(),
                format!("{}文🦀{}", &original[..begin], &original[end..])
            );
            assert!(actions.end_composition().unwrap().is_some());
            actions
                .command(
                    EditorCommand::Undo,
                    actions.selection(&actions.source()).unwrap(),
                    actions.rules().indentation,
                )
                .unwrap();
            assert_eq!(mounted.state.workspace.content.get_untracked(), original);
            assert_eq!(actions.selection(&original), Some(selected));
            assert!(!mounted.state.workspace.dirty.get_untracked());
        }
    }
}

#[wasm_bindgen_test]
async fn prepared_editor_binds_native_windows_and_edits_full_source_in_both_modes() {
    use openwebide_core::{WorkspaceMode, editor::Selection};
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        for wrap in [false, true] {
            let source = format!(
                "{}tail",
                "a row with 文 and 🦀 and some words\r\n".repeat(800)
            );
            let original = source.clone();
            let mounted = mount_test(move |state| {
                state.seed_project();
                state
                    .projects
                    .projects
                    .update(|projects| projects[0].mode = mode);
                state
                    .settings
                    .editor_preferences
                    .update(|preferences| preferences.word_wrap = wrap);
                state.workspace.open_file.set(Some("window.txt".into()));
                state.workspace.content.set(source);
                let commands = super::support::command_actions(state.clone());
                view! { <button class="capture-window" on:click=move |_| commands.run.run(openwebide_frontend::commands::Command::CaptureEditor)>"Capture"</button><style>{include_str!("../../styles.css")}</style><div style="display:flex;width:520px;height:280px">{editor_view(state)}</div> }
            });
            let input: web_sys::HtmlTextAreaElement =
                mounted.element(".editor-textarea").unchecked_into();
            wait_until("bounded native editor input", || {
                input.get_attribute("data-editor-native-bound").as_deref() == Some("true")
            })
            .await;
            assert!(input.value().len() <= 12 * 1024);
            assert!(input.value().len() < original.len());
            let native = input.value();
            input
                .set_attribute("data-editor-native-generation", "0")
                .unwrap();
            editorNativeInput(&input, "discarded", "insertText", false);
            assert_eq!(mounted.state.workspace.content.get_untracked(), original);
            assert_eq!(input.value(), native);
            assert!(!mounted.state.workspace.dirty.get_untracked());
            let actions = EditorActions::new(mounted.state.workspace);
            assert!(editor_key(&input, "PageDown", false, false).default_prevented());
            wait_until("source page movement", || {
                actions
                    .selection(&original)
                    .is_some_and(|selection| selection.head > 0)
            })
            .await;
            editor_key(&input, "Home", true, false);
            assert!(editor_key(&input, "PageDown", false, true).default_prevented());
            wait_until("source page selection", || {
                actions
                    .selection(&original)
                    .is_some_and(|selection| selection.anchor == 0 && selection.head > 0)
            })
            .await;
            let selected = actions.selection(&original).unwrap();
            assert_eq!(editorClipboardCopy(&input), &original[..selected.head]);
            editor_key(&input, "End", true, false);
            assert_eq!(
                actions.selection(&original),
                Some(Selection::caret(original.len()))
            );
            assert!(input.value().ends_with("tail"));
            assert_eq!(
                actions.input_selection(
                    Selection::caret(input.selection_end().unwrap().unwrap() as usize),
                    || panic!("bound selection mapping must not read the native value"),
                ),
                Selection::caret(original.len()),
            );
            mounted.click(".capture-window");
            let captured = mounted
                .state
                .chat
                .active_editor_context
                .get_untracked()
                .unwrap();
            assert_eq!(captured.cursor_line, 801);
            assert_eq!(captured.cursor_col, 5);
            assert!(captured.selection.is_none());

            editorNativeInput(&input, "日🦀", "insertText", false);
            let expected = format!("{original}日🦀");
            assert_eq!(actions.source(), expected);
            assert!(input.value().len() <= 16 * 1024);
            assert!(input.value().ends_with("tail日🦀"));
            editor_key(&input, "z", true, false);
            assert_eq!(actions.source(), original);
            editor_key(&input, "a", true, false);
            assert_eq!(
                actions.selection(&original),
                Some(Selection {
                    anchor: 0,
                    head: original.len()
                })
            );
            assert_eq!(editorClipboardCopy(&input), original);
            editorNativeInput(&input, "replacement 文\n🦀", "insertReplacementText", false);
            assert_eq!(actions.source(), "replacement 文\r\n🦀");
            wait_until("replacement source paint", || {
                mounted
                    .root
                    .query_selector(".editor-highlight")
                    .unwrap()
                    .and_then(|paint| paint.text_content())
                    .is_some_and(|text| text.contains("replacement 文"))
            })
            .await;
            editor_key(&input, "z", true, false);
            assert_eq!(actions.source(), original);
            assert_eq!(
                actions.selection(&original),
                Some(Selection {
                    anchor: 0,
                    head: original.len()
                })
            );
        }
    }
}

#[wasm_bindgen_test]
async fn bound_editor_composition_keeps_native_ownership_across_provider_metadata_in_both_modes() {
    use openwebide_core::{
        WorkspaceMode,
        editor::{FoldRange, Selection},
    };
    use openwebide_frontend::state_actions::editor::EditorActions;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let source = "a source row with 文🦀\r\n".repeat(900);
        let original = source.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("ime-window.txt".into()));
            state.workspace.content.set(source);
            view! { <style>{include_str!("../../styles.css")}</style><div style="display:flex;width:520px;height:280px">{editor_view(state)}</div> }
        });
        let input: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        wait_until("bounded composition input", || {
            input.get_attribute("data-editor-native-bound").as_deref() == Some("true")
        })
        .await;
        let actions = EditorActions::new(mounted.state.workspace);
        editor_key(&input, "a", true, false);
        input
            .dispatch_event(&web_sys::CompositionEvent::new("compositionstart").unwrap())
            .unwrap();
        editorNativeInput(&input, "first\n日", "insertCompositionText", true);
        assert_eq!(actions.source(), "first\r\n日");
        assert_eq!(input.value(), "first\n日");
        // New provider descriptors do not alter visible native text or its mapping.
        mounted
            .state
            .workspace
            .editor_documents
            .update(|documents| {
                assert!(
                    documents
                        .get_mut(&(1, "ime-window.txt".into()))
                        .unwrap()
                        .set_fold_ranges(vec![FoldRange {
                            start_line: 0,
                            end_line: 1
                        }])
                );
            });
        mounted
            .state
            .workspace
            .editor_fold_revision
            .update(|revision| *revision += 1);
        input.set_selection_range(6, 7).unwrap();
        editorNativeInput(&input, "文🦀", "insertCompositionText", true);
        assert_eq!(actions.source(), "first\r\n文🦀");
        assert_eq!(input.value(), "first\n文🦀");
        input
            .dispatch_event(&web_sys::CompositionEvent::new("compositionend").unwrap())
            .unwrap();
        editor_key(&input, "z", true, false);
        assert_eq!(actions.source(), original);
        assert_eq!(
            actions.selection(&original),
            Some(Selection {
                anchor: 0,
                head: original.len()
            })
        );
        input
            .dispatch_event(&web_sys::CompositionEvent::new("compositionstart").unwrap())
            .unwrap();
        editorNativeInput(&input, "reloaded 文", "insertCompositionText", true);
        assert_eq!(actions.source(), "reloaded 文");
        mounted
            .state
            .workspace
            .editor_read_revision
            .update(|revision| *revision += 1);
        assert_eq!(
            actions.native_input("discarded".into(), Selection::caret(9), "insertText", 9.0),
            Err(openwebide_core::editor::EditError::StaleContext)
        );
        assert_eq!(actions.source(), "reloaded 文");
        input
            .dispatch_event(&web_sys::CompositionEvent::new("compositionend").unwrap())
            .unwrap();
        assert_eq!(
            actions.source(),
            "reloaded 文",
            "an old composition must not restore its snapshot into a new read"
        );
        assert!(
            mounted
                .state
                .workspace
                .editor_composition
                .get_untracked()
                .is_none()
        );
        editor_key(&input, "a", true, false);
        input
            .dispatch_event(&web_sys::CompositionEvent::new("compositionstart").unwrap())
            .unwrap();
        editorNativeInput(&input, "account preview", "insertCompositionText", true);
        assert_eq!(actions.source(), "account preview");
        mounted
            .state
            .auth
            .generation
            .update(|generation| *generation += 1);
        editorNativeInput(&input, "discarded", "insertCompositionText", true);
        input
            .dispatch_event(&web_sys::CompositionEvent::new("compositionend").unwrap())
            .unwrap();
        settle().await;
        assert_eq!(
            actions.source(),
            "account preview",
            "an old editor node must not publish into another account"
        );
        assert!(
            mounted
                .state
                .workspace
                .editor_composition
                .get_untracked()
                .is_none()
        );
    }
}
