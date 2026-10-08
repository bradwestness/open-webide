//! Filename relationships for the explorer; filesystem paths remain unchanged.
use crate::FileEntry;
use std::collections::HashMap;

/// Group sibling variants under the closest existing base filename.
/// For example, `compose.ssh.yml` belongs to `compose.yml`, and
/// `app.ts.map` belongs to `app.ts`. Missing bases remain ordinary files.
pub fn parents(entries: &[FileEntry]) -> HashMap<String, String> {
    let files: HashMap<_, _> = entries
        .iter()
        .filter(|entry| !entry.is_dir)
        .map(|entry| {
            (
                (
                    crate::workspace_entries::parent(&entry.path),
                    entry.name.as_str(),
                ),
                entry,
            )
        })
        .collect();
    let mut result = HashMap::new();
    for child in entries.iter().filter(|entry| !entry.is_dir) {
        let directory = crate::workspace_entries::parent(&child.path);
        let extension = child.name.rsplit_once('.').map(|(_, extension)| extension);
        let mut closest: Option<&FileEntry> = None;
        for (index, _) in child
            .name
            .match_indices('.')
            .filter(|(index, _)| *index > 0)
        {
            let prefix = &child.name[..index];
            let variant = extension.map(|extension| format!("{prefix}.{extension}"));
            for name in std::iter::once(prefix).chain(variant.as_deref()) {
                if name.len() >= child.name.len() {
                    continue;
                }
                if let Some(&candidate) = files.get(&(directory, name))
                    && closest.is_none_or(|base| candidate.name.len() > base.name.len())
                {
                    closest = Some(candidate);
                }
            }
        }
        if let Some(base) = closest {
            result.insert(child.path.clone(), base.path.clone());
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str) -> FileEntry {
        FileEntry {
            name: path.rsplit('/').next().unwrap().into(),
            path: path.into(),
            is_dir: false,
            size: 0,
        }
    }

    #[test]
    fn closest_base_wins_with_same_extension_and_appended_suffixes() {
        let entries = [
            "docker-compose.yml",
            "docker-compose.ssh.yml",
            "docker-compose.ssh.dev.yml",
            "app.ts",
            "app.test.ts",
            "app.ts.map",
        ]
        .map(file);
        let groups = parents(&entries);
        assert_eq!(groups["docker-compose.ssh.yml"], "docker-compose.yml");
        assert_eq!(
            groups["docker-compose.ssh.dev.yml"],
            "docker-compose.ssh.yml"
        );
        assert_eq!(groups["app.test.ts"], "app.ts");
        assert_eq!(groups["app.ts.map"], "app.ts");
    }

    #[test]
    fn missing_bases_folders_other_directories_and_prefixes_stay_separate() {
        let mut entries = [
            "src/app.ts",
            "other/app.test.ts",
            "src/apple.test.ts",
            "compose.ssh.yml",
            "README.md",
            "README.en.md",
            ".env",
            ".env.local",
            "folder",
            "folder.test",
        ]
        .map(file);
        entries[8].is_dir = true;
        let groups = parents(&entries);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups["README.en.md"], "README.md");
        assert_eq!(groups[".env.local"], ".env");
        // Every parent is shorter, so cycles are impossible.
        for (child, parent) in groups {
            assert!(parent.len() < child.len());
        }
    }
}
