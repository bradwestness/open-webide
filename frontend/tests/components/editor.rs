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
        first.set_scroll_top(450.0);
        first.set_scroll_left(120.0);
        first
            .dispatch_event(&web_sys::Event::new("scroll").unwrap())
            .unwrap();
        let expected = (first.scroll_top(), first.scroll_left());
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
        assert!(other.scroll_top().abs() < 0.5);
        other.set_selection_range(5, 5).unwrap();
        other
            .dispatch_event(&web_sys::Event::new("select").unwrap())
            .unwrap();
        // Late measurements from a detached file cannot overwrite the active file.
        first.set_scroll_top(900.0);
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
        assert_eq!((restored.scroll_top(), restored.scroll_left()), expected);
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
        assert!(second.scroll_top().abs() < 0.5);
        mounted.state.workspace.switch_project(Some(2), 1);
        settle().await;
        frame().await;
        let restored: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        assert_eq!(restored.selection_end().unwrap(), Some(320));
        assert_eq!((restored.scroll_top(), restored.scroll_left()), expected);
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
        assert_eq!((restored.scroll_top(), restored.scroll_left()), expected);
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
        assert!(compact_width < 45.0);
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
        assert!(gutter_width > 30.0 && gutter_width < 60.0);
        assert!(textarea.scroll_width() > textarea.client_width());
        assert!(textarea.scroll_height() > textarea.client_height());
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
                (overlay.get_bounding_client_rect().height() - f64::from(textarea.client_height()))
                    .abs()
                    < 0.5
            })
            .await;
            assert!(
                (overlay.get_bounding_client_rect().bottom()
                    - (textarea.get_bounding_client_rect().top()
                        + f64::from(textarea.client_height())))
                .abs()
                    < 0.5
            );
        }
    }
}

#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
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
export async function editorConfigCleanup(folder) { await folder.root.removeEntry(folder.name, {recursive:true}); }
"#)]
extern "C" {
    #[wasm_bindgen(catch)]
    async fn editorConfigFolder() -> Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue>;
    fn editorConfigHandle(folder: &wasm_bindgen::JsValue) -> wasm_bindgen::JsValue;
    #[wasm_bindgen(catch)]
    async fn editorConfigCleanup(
        folder: &wasm_bindgen::JsValue,
    ) -> Result<(), wasm_bindgen::JsValue>;
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
export function editorGesture(target, line, column, shift, moving) {
    const row = target.parentElement.querySelector(`.editor-source-line[data-line='${line}']`);
    const walker = document.createTreeWalker(row, NodeFilter.SHOW_TEXT);
    let node, offset = column;
    while ((node = walker.nextNode())) { if (offset <= node.length) break; offset -= node.length; }
    if (!node) throw new Error('Missing gesture text position');
    const range = document.createRange(); range.setStart(node, offset); range.collapse(true);
    const rect = range.getBoundingClientRect();
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
        textarea.set_scroll_top(1900.0);
        textarea.set_scroll_left(120.0);
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
        let scroll = (textarea.scroll_top(), textarea.scroll_left());
        assert!(scroll.0 > 1000.0 && scroll.1 > 50.0);
        let actions = EditorActions::new(mounted.state.workspace);
        for label in ["Collapse block at line 101", "Expand block at line 101"] {
            mounted.click(&format!("button[aria-label='{label}']"));
            settle().await;
            frame().await;
            frame().await;
            assert!(
                (textarea.scroll_top() - scroll.0).abs() < 1.0,
                "{label}: vertical position {} instead of {}",
                textarea.scroll_top(),
                scroll.0
            );
            assert!(
                (textarea.scroll_left() - scroll.1).abs() < 1.0,
                "{label}: horizontal position"
            );
            assert!((actions.scroll().top - textarea.scroll_top()).abs() < 1.0);
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
                (textarea.scroll_top() - scroll.0).abs() < 1.0,
                "menu {label}"
            );
            assert!((textarea.scroll_left() - scroll.1).abs() < 1.0);
        }
        let header = u32::try_from(expected.find("fn folded()").unwrap()).unwrap();
        textarea.set_selection_range(header, header).unwrap();
        textarea.set_scroll_top(scroll.0);
        textarea.set_scroll_left(scroll.1);
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
                (textarea.scroll_top() - scroll.0).abs() < 1.0,
                "keyboard {key}"
            );
            assert!((textarea.scroll_left() - scroll.1).abs() < 1.0);
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
            assert_eq!(
                actions.refresh_fold_ranges(|| false),
                Some(SyntaxStatus::Cancelled)
            );
            settle().await;
            assert_eq!(textarea.value(), source.replace("\r\n", "\n"));
            assert!(actions.fold_state().unwrap().ranges().is_empty());
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
        textarea
            .dispatch_event(
                &web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap(),
            )
            .unwrap();
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
        textarea
            .dispatch_event(
                &web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap(),
            )
            .unwrap();
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
        let before = highlight_count();
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
        assert_eq!(highlight_count(), before);
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
                    .query_selector_all(".editor-secondary-caret")
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
        let after = actions.selections(source);
        assert_eq!(after.len(), 2);
        assert_eq!(line_column(source, after[0].head).0, 5);
        assert_eq!(line_column(source, after[1].head).0, 4);
        assert!(editor_key(&textarea, "ArrowUp", false, false).default_prevented());
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
        sender.send(output).unwrap();
    }
    fn source(&self) -> String {
        serde_json::from_str::<openwebide_core::editor::SyntaxRequest>(
            &self.pending.borrow()[0].message,
        )
        .unwrap()
        .source
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
        let source = "{\r\n  \"name\": \"文😀\", \"value\": 42\r\n}\r\n";
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
            &openwebide_core::highlight::highlight_lines(
                source,
                openwebide_core::highlight::Language::Json
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
        transport.respond(true);
        wait_until("revised lexical paint", || {
            actions.syntax_highlights().is_some()
        })
        .await;
        assert!(!std::sync::Arc::ptr_eq(
            &first,
            &actions.syntax_highlights().unwrap()
        ));
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
                let row = mounted.element(".editor-source-line");
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
        assert_eq!(input.value(), source.replace("\r\n", "\n"));
        let selector = ".editor-highlight .editor-source-line";
        for target in [500, 1100, 0, 700] {
            input.set_scroll_top(measured.rows.top(target).unwrap() + 12.0);
            input
                .dispatch_event(&web_sys::Event::new("scroll").unwrap())
                .unwrap();
            wait_until("bounded wrapped paint at target", || {
                let first = mounted
                    .element(selector)
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
                    - input.scroll_top();
            assert!((first.get_bounding_client_rect().top() - expected_top).abs() < 0.5);
            assert!((f64::from(input.scroll_height()) - measured.rows.height() - 24.0).abs() < 2.0);
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
        assert_eq!(textarea.value(), changed);
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
        let actions = EditorActions::new(mounted.state.workspace);
        wait_until("initial reusable wrapped rows", || {
            actions.measured_rows().is_some()
        })
        .await;
        let old = actions.measured_rows().unwrap().rows;
        let observed = watch_row_measurements();
        let changed = source.replace(
            "row 500 ",
            &format!("row 500 {} ", "more words ".repeat(30)),
        );
        input(&mounted, &changed);
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
        input(&mounted, &inserted);
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
        input(&mounted, &source);
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
        input(&mounted, &disjoint);
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
        for top in [10_000.0, height - 1000.0, 0.0] {
            input.set_scroll_top(top);
            input
                .dispatch_event(&web_sys::Event::new("scroll").unwrap())
                .unwrap();
            wait_until("fragment follows scrolling inside one logical row", || {
                mounted
                    .element(".editor-source-line")
                    .get_attribute("data-paint-top")
                    .and_then(|value| value.parse::<f64>().ok())
                    .is_some_and(|paint_top| (paint_top - input.scroll_top()).abs() < 200.0)
            })
            .await;
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
            assert_eq!(input.value(), original.replace("\r\n", "\n"));
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
                if input.scroll_top() <= height - 1000.0 {
                    return false;
                }
                let fragment = mounted.element(".editor-source-fragment");
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
        assert!(input.scroll_top() > height - 1000.0);
        assert_eq!(mounted.state.workspace.content.get_untracked(), original);
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
