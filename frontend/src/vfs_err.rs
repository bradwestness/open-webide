use openwebide_core::VfsError;

pub fn map_vfs_err(msg: String) -> VfsError {
    if msg.starts_with("NotFoundError") {
        VfsError::NotFound(msg)
    } else if msg.contains("already exists") {
        VfsError::AlreadyExists(msg)
    } else if msg.contains("no such file") || msg.contains("not found") || msg.contains("not be found") {
        VfsError::NotFound(msg)
    } else if msg.contains("permission") {
        VfsError::PermissionDenied(msg)
    } else if msg.contains("escapes") {
        VfsError::PathEscape(msg)
    } else {
        VfsError::Io(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_vfs_err() {
        assert!(matches!(map_vfs_err("NotFoundError".into()), VfsError::NotFound(_)));
        assert!(matches!(map_vfs_err("already exists".into()), VfsError::AlreadyExists(_)));
        assert!(matches!(map_vfs_err("no such file".into()), VfsError::NotFound(_)));
        assert!(matches!(map_vfs_err("permission".into()), VfsError::PermissionDenied(_)));
        assert!(matches!(map_vfs_err("escapes".into()), VfsError::PathEscape(_)));
        assert!(matches!(map_vfs_err("random error".into()), VfsError::Io(_)));
    }
}
