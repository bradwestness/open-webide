//! Behavioral contracts shared by host and browser adapter tests.

use crate::{Vfs, VfsError, vfs::VfsEntryKind};

/// Run on an empty, disposable workspace; creation must be exclusive for files and idempotent for directories.
pub async fn vfs_creation_contract(vfs: &impl Vfs) {
    vfs.create("created", VfsEntryKind::Directory)
        .await
        .unwrap();
    vfs.create("created", VfsEntryKind::Directory)
        .await
        .unwrap();
    vfs.create("created/file.txt", VfsEntryKind::File)
        .await
        .unwrap();
    assert_eq!(vfs.read("created/file.txt").await.unwrap(), "");
    vfs.write("created/file.txt", "keep these bytes\n")
        .await
        .unwrap();
    assert!(matches!(
        vfs.create("created/file.txt", VfsEntryKind::File).await,
        Err(VfsError::AlreadyExists(_))
    ));
    assert_eq!(
        vfs.read("created/file.txt").await.unwrap(),
        "keep these bytes\n"
    );
    for kind in [VfsEntryKind::File, VfsEntryKind::Directory] {
        assert!(matches!(
            vfs.create("../escape", kind).await,
            Err(VfsError::PathEscape(_))
        ));
    }
    let entries = vfs.list("created").await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, "created/file.txt");
    assert!(!entries[0].is_dir);
    vfs.delete("created").await.unwrap();
    assert!(matches!(
        vfs.read("created/file.txt").await,
        Err(VfsError::NotFound(_))
    ));
}

/// The same restoration, conflict and interrupted-retry contract runs against
/// local browser and remote workspace adapters. Uses an empty disposable root.
pub async fn rewind_contract(files: &impl crate::rewind::RewindFiles) {
    use crate::{RewindFile, RewindPlan, rewind::restore_files};
    files.write("existing.txt", "after").await.unwrap();
    files.write("created.txt", "created").await.unwrap();
    let plan = RewindPlan {
        message_id: 1,
        prompt: "change files".into(),
        files: vec![
            RewindFile {
                path: "existing.txt".into(),
                before: Some("before".into()),
                backup_path: None,
                binary_before: None,
                binary_after: None,
                deleted: false,
                after: "after".into(),
            },
            RewindFile {
                path: "created.txt".into(),
                before: None,
                backup_path: None,
                binary_before: None,
                binary_after: None,
                deleted: false,
                after: "created".into(),
            },
        ],
    };
    files.write("created.txt", "user's edit").await.unwrap();
    assert!(restore_files(files, &plan, || true).await.is_err());
    assert_eq!(
        files.read("existing.txt").await.unwrap().as_deref(),
        Some(b"after".as_slice())
    );
    files.write("created.txt", "created").await.unwrap();
    assert!(restore_files(files, &plan, || false).await.is_err());
    assert_eq!(
        files.read("existing.txt").await.unwrap().as_deref(),
        Some(b"after".as_slice())
    );
    // Simulate an interrupted restore: one file has already reached its target.
    files.write("existing.txt", "before").await.unwrap();
    restore_files(files, &plan, || true).await.unwrap();
    assert_eq!(
        files.read("existing.txt").await.unwrap().as_deref(),
        Some(b"before".as_slice())
    );
    assert_eq!(files.read("created.txt").await.unwrap(), None);
    restore_files(files, &plan, || true).await.unwrap();
    files.write_bytes("binary.dat", &[0, 254]).await.unwrap();
    let binary_plan = RewindPlan {
        message_id: 1,
        prompt: "binary and deleted files".into(),
        files: vec![
            RewindFile {
                path: "binary.dat".into(),
                before: None,
                after: String::new(),
                backup_path: None,
                binary_before: Some("AP8=".into()),
                binary_after: Some("AP4=".into()),
                deleted: false,
            },
            RewindFile {
                path: "gone/nested.txt".into(),
                before: Some("restore".into()),
                after: String::new(),
                backup_path: None,
                binary_before: None,
                binary_after: None,
                deleted: true,
            },
        ],
    };
    restore_files(files, &binary_plan, || true).await.unwrap();
    assert_eq!(files.read("binary.dat").await.unwrap(), Some(vec![0, 255]));
    assert_eq!(
        files.read("gone/nested.txt").await.unwrap(),
        Some(b"restore".to_vec())
    );
    restore_files(files, &binary_plan, || true).await.unwrap();
    files.delete("binary.dat").await.unwrap();
    files.delete("gone/nested.txt").await.unwrap();
    files.delete("existing.txt").await.unwrap();
}

/// Project checkpoints cover arbitrary bytes, deletion and creation, while
/// leaving Git metadata and generated output outside the restoration boundary.
pub async fn project_checkpoint_contract(vfs: &impl Vfs) {
    use crate::rewind::{ProjectCheckpoint, capture_project};
    vfs.write("source.txt", "before").await.unwrap();
    vfs.write_bytes("binary.dat", &[0, 255, 1]).await.unwrap();
    vfs.write("nested/deleted.txt", "restore me").await.unwrap();
    vfs.write(".git/HEAD", "first commit").await.unwrap();
    vfs.write("target/generated", "old artifact").await.unwrap();
    let before = capture_project(vfs).await.unwrap();
    vfs.write("source.txt", "after").await.unwrap();
    vfs.write_bytes("binary.dat", &[0, 254, 2]).await.unwrap();
    vfs.delete("nested").await.unwrap();
    vfs.write("created.txt", "new").await.unwrap();
    vfs.write(".git/HEAD", "second commit").await.unwrap();
    vfs.write("target/generated", "new artifact").await.unwrap();
    let after = capture_project(vfs).await.unwrap();
    let mut checkpoint = ProjectCheckpoint {
        before,
        after: Some(after),
    };
    checkpoint.compact();
    let changes = checkpoint.changes().unwrap();
    assert_eq!(changes.len(), 4);
    assert!(
        changes
            .iter()
            .all(|file| !file.path.starts_with(".git/") && !file.path.starts_with("target/"))
    );
    let binary = changes
        .iter()
        .find(|file| file.path == "binary.dat")
        .unwrap();
    assert_eq!(binary.before_bytes().unwrap(), Some(vec![0, 255, 1]));
    assert_eq!(binary.after_bytes().unwrap(), Some(vec![0, 254, 2]));
    let deleted = changes
        .iter()
        .find(|file| file.path == "nested/deleted.txt")
        .unwrap();
    assert_eq!(deleted.after_bytes().unwrap(), None);
    assert_eq!(
        deleted.before_bytes().unwrap(),
        Some(b"restore me".to_vec())
    );
    for path in ["source.txt", "binary.dat", "created.txt", ".git", "target"] {
        vfs.delete(path).await.unwrap();
    }
}
