use std::collections::HashMap;
use openwebide_core::FileDiff;

pub fn merge_pending(map: &mut HashMap<String, FileDiff>, d: FileDiff) {
    let path = d.path.clone();
    if let Some(entry) = map.get_mut(&path) {
        entry.new = d.new;
        if !entry.old_unavailable && entry.old.as_deref() == Some(entry.new.as_str()) {
            map.remove(&path);
        }
    } else {
        map.insert(path, d);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RejectAction {
    Restore(String),
    RestoreFromBackup(String),
    Delete,
    Unavailable,
}

pub fn reject_action(d: &FileDiff) -> RejectAction {
    if d.old_unavailable {
        if let Some(backup) = &d.backup_path {
            RejectAction::RestoreFromBackup(backup.clone())
        } else {
            RejectAction::Unavailable
        }
    } else if let Some(old) = &d.old {
        RejectAction::Restore(old.clone())
    } else {
        RejectAction::Delete
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openwebide_core::FileDiff;

    #[test]
    fn test_merge_pending() {
        let mut map = HashMap::new();
        let d1 = FileDiff { path: "a.txt".into(), old: Some("a".into()), new: "b".into(), old_unavailable: false, backup_path: None };
        merge_pending(&mut map, d1.clone());
        assert_eq!(map["a.txt"].old.as_deref(), Some("a"));
        assert_eq!(map["a.txt"].new, "b");

        let d2 = FileDiff { path: "a.txt".into(), old: Some("b".into()), new: "a".into(), old_unavailable: false, backup_path: None };
        merge_pending(&mut map, d2);
        assert!(!map.contains_key("a.txt"));

        let d3 = FileDiff { path: "b.txt".into(), old: None, new: "new".into(), old_unavailable: false, backup_path: None };
        merge_pending(&mut map, d3);
        assert!(map.contains_key("b.txt"));
    }

    #[test]
    fn test_reject_action() {
        let d_rest = FileDiff { path: "a.txt".into(), old: Some("old".into()), new: "new".into(), old_unavailable: false, backup_path: None };
        assert!(matches!(reject_action(&d_rest), RejectAction::Restore(s) if s == "old"));

        let d_del = FileDiff { path: "b.txt".into(), old: None, new: "new".into(), old_unavailable: false, backup_path: None };
        assert!(matches!(reject_action(&d_del), RejectAction::Delete));

        let d_back = FileDiff { path: "c.txt".into(), old: None, new: "new".into(), old_unavailable: true, backup_path: Some("back".into()) };
        assert!(matches!(reject_action(&d_back), RejectAction::RestoreFromBackup(s) if s == "back"));

        let d_un = FileDiff { path: "d.txt".into(), old: None, new: "new".into(), old_unavailable: true, backup_path: None };
        assert!(matches!(reject_action(&d_un), RejectAction::Unavailable));
    }
}
