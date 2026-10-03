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
