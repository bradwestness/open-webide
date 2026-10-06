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
        actions.end_composition();
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
            .element(".editor-source-line[data-line='1']")
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
        assert!(!editorClipboardCut(&textarea).default_prevented());
        assert!(textarea.value().contains("one();"));
        assert!(!textarea.value().contains("two();"));
        editorNativeInput(&textarea, "", "deleteByCut", false);
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
