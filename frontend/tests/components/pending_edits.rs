use leptos::prelude::*;
use openwebide_core::FileDiff;
use openwebide_frontend::testing::fake_backend::Call;
use wasm_bindgen_test::*;

use super::support::{Mounted, editor_view, mount_test, settle};

fn mount_diff(diff: FileDiff) -> Mounted {
    mount_test(move |state| {
        state.seed_project();
        state
            .fake
            .files
            .borrow_mut()
            .insert((1, diff.path.clone()), diff.new.clone());
        if let Some(backup) = &diff.backup_path {
            state
                .fake
                .files
                .borrow_mut()
                .insert((1, backup.clone()), "original bytes".into());
        }
        state.workspace.open_file.set(Some(diff.path.clone()));
        state.workspace.content.set(diff.new.clone());
        state.workspace.pending_edits.update(|pending| {
            pending.insert(diff.path.clone(), diff);
        });
        editor_view(state)
    })
}

async fn reject(mounted: &Mounted) {
    settle().await;
    mounted.click_text("✕ Reject");
    settle().await;
    assert!(
        mounted
            .state
            .fake
            .calls
            .borrow()
            .iter()
            .all(|call| !matches!(
                call,
                Call::WriteFile { .. } | Call::CopyFile { .. } | Call::DeleteFile { .. }
            ))
    );
    mounted.click(".modal-footer .danger");
    settle().await;
}

fn file_calls(mounted: &Mounted) -> Vec<Call> {
    mounted
        .state
        .fake
        .calls
        .borrow()
        .iter()
        .filter(|call| {
            matches!(
                call,
                Call::WriteFile { .. } | Call::CopyFile { .. } | Call::DeleteFile { .. }
            )
        })
        .cloned()
        .collect()
}

#[wasm_bindgen_test]
async fn reject_restores_original_across_multiple_edits() {
    let mounted = mount_diff(FileDiff {
        path: "file.rs".into(),
        old: Some("v0".into()),
        new: "v1".into(),
        old_unavailable: false,
        backup_path: None,
    });
    mounted.state.workspace.merge_pending(
        1,
        FileDiff {
            path: "file.rs".into(),
            old: Some("v1".into()),
            new: "v2".into(),
            old_unavailable: false,
            backup_path: None,
        },
    );
    reject(&mounted).await;
    assert_eq!(
        file_calls(&mounted),
        vec![Call::WriteFile {
            path: "file.rs".into(),
            content: "v0".into()
        }]
    );
    assert_eq!(
        mounted
            .state
            .fake
            .files
            .borrow()
            .get(&(1, "file.rs".into()))
            .map(String::as_str),
        Some("v0")
    );
    assert!(
        mounted
            .state
            .workspace
            .pending_edits
            .get_untracked()
            .is_empty()
    );
}

#[wasm_bindgen_test]
async fn reject_new_file_deletes_it() {
    let mounted = mount_diff(FileDiff {
        path: "new.rs".into(),
        old: None,
        new: "v2".into(),
        old_unavailable: false,
        backup_path: None,
    });
    reject(&mounted).await;
    assert_eq!(
        file_calls(&mounted),
        vec![Call::DeleteFile {
            path: "new.rs".into()
        }]
    );
    assert!(
        !mounted
            .state
            .fake
            .files
            .borrow()
            .contains_key(&(1, "new.rs".into()))
    );
}

#[wasm_bindgen_test]
async fn reject_unavailable_original_restores_backup_then_deletes_backup() {
    let mounted = mount_diff(FileDiff {
        path: "file.rs".into(),
        old: None,
        new: "v2".into(),
        old_unavailable: true,
        backup_path: Some("backup.rs".into()),
    });
    reject(&mounted).await;
    assert_eq!(
        file_calls(&mounted),
        vec![
            Call::CopyFile {
                from: "backup.rs".into(),
                to: "file.rs".into()
            },
            Call::DeleteFile {
                path: "backup.rs".into()
            }
        ]
    );
    assert_eq!(
        mounted
            .state
            .fake
            .files
            .borrow()
            .get(&(1, "file.rs".into()))
            .map(String::as_str),
        Some("original bytes")
    );
}

#[wasm_bindgen_test]
async fn accept_unavailable_original_only_deletes_backup() {
    let mounted = mount_diff(FileDiff {
        path: "file.rs".into(),
        old: None,
        new: "v2".into(),
        old_unavailable: true,
        backup_path: Some("backup.rs".into()),
    });
    settle().await;
    mounted.click_text("✓ Accept");
    settle().await;
    assert_eq!(
        file_calls(&mounted),
        vec![Call::DeleteFile {
            path: "backup.rs".into()
        }]
    );
    assert_eq!(
        mounted
            .state
            .fake
            .files
            .borrow()
            .get(&(1, "file.rs".into()))
            .map(String::as_str),
        Some("v2")
    );
    assert!(
        mounted
            .state
            .workspace
            .pending_edits
            .get_untracked()
            .is_empty()
    );
}

#[wasm_bindgen_test]
async fn persisted_edit_backend_contract_survives_replay_and_checks_revisions() {
    use openwebide_core::{EditDecision, ResolveEditRequest};
    use openwebide_frontend::backend::Backend;
    let mounted = mount_test(|state| {
        state.seed_project();
        state.seed_session();
        view! { <div /> }
    });
    let fake = &mounted.state.fake;
    let diff = FileDiff {
        path: "a.txt".into(),
        old: Some("a".into()),
        new: "b".into(),
        old_unavailable: false,
        backup_path: None,
    };
    fake.upsert_tool_step(1, 1, "one", "write_file", "preview", Some(&diff))
        .await
        .unwrap();
    assert!(fake.list_pending_edits(1).await.unwrap().is_empty());
    fake.complete_tool_step(1, "one", true, "written", Some(&diff))
        .await
        .unwrap();
    assert_eq!(fake.list_pending_edits(1).await.unwrap()[0].diff, diff);
    assert!(fake.list_pending_edits(2).await.unwrap().is_empty());
    let mut request = ResolveEditRequest {
        path: "a.txt".into(),
        revision: 1,
        decision: EditDecision::Accepted,
    };
    *fake.resolution_error.borrow_mut() = Some("offline".into());
    assert!(fake.resolve_pending_edit(1, &request).await.is_err());
    assert_eq!(fake.list_pending_edits(1).await.unwrap().len(), 1);
    *fake.resolution_error.borrow_mut() = None;
    fake.resolve_pending_edit(1, &request).await.unwrap();
    fake.resolve_pending_edit(1, &request).await.unwrap();
    fake.complete_tool_step(1, "one", true, "replay", Some(&diff))
        .await
        .unwrap();
    assert!(fake.list_pending_edits(1).await.unwrap().is_empty());
    let newer = FileDiff {
        old: Some("b".into()),
        new: "c".into(),
        ..diff.clone()
    };
    fake.upsert_tool_step(1, 1, "two", "write_file", "write", None)
        .await
        .unwrap();
    fake.complete_tool_step(1, "two", true, "written", Some(&newer))
        .await
        .unwrap();
    assert!(fake.resolve_pending_edit(1, &request).await.is_err());
    assert_eq!(fake.list_pending_edits(1).await.unwrap()[0].revision, 2);
    request.revision = 2;
    request.decision = EditDecision::Rejected;
    fake.resolve_pending_edit(1, &request).await.unwrap();
    assert!(fake.list_pending_edits(1).await.unwrap().is_empty());
    fake.upsert_tool_step(1, 1, "failed", "write_file", "preview", Some(&diff))
        .await
        .unwrap();
    fake.complete_tool_step(1, "failed", false, "failed", Some(&diff))
        .await
        .unwrap();
    assert!(fake.list_pending_edits(1).await.unwrap().is_empty());
}
