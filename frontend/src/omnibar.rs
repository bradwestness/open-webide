//! Universal search catalog and bounded filename discovery, shared by both modes.
use crate::commands::{Command, CommandContext};
use openwebide_core::{
    ChatSession, FileEntry, Project, VfsError,
    vfs::{SearchOptions, skip_dir, workspace_path},
};
use std::collections::{BTreeSet, VecDeque};

pub const RESULT_LIMIT: usize = 80;
pub const FILE_LIMIT: usize = 10_000;
const DIRECTORY_LIMIT: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OmnibarAction {
    Command(Command),
    File(String),
    Project(i64),
    Session(i64),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OmnibarEntry {
    pub id: String,
    pub label: String,
    pub detail: String,
    pub kind: &'static str,
    pub shortcut: &'static str,
    pub unavailable: Option<String>,
    pub action: OmnibarAction,
}

fn score(query: &str, label: &str, detail: &str) -> Option<usize> {
    let query = query.to_lowercase();
    let label = label.to_lowercase();
    let text = format!("{label} {}", detail.to_lowercase());
    if query.is_empty() {
        return Some(0);
    }
    if label == query {
        return Some(100);
    }
    if label.starts_with(&query) {
        return Some(80);
    }
    if query.split_whitespace().all(|word| text.contains(word)) {
        return Some(50);
    }
    let mut matched = query.chars().filter(|ch| !ch.is_whitespace());
    let mut next = matched.next();
    for character in text.chars() {
        if next == Some(character) {
            next = matched.next();
        }
    }
    next.is_none().then_some(10)
}

/// Prefixes narrow the same search: > commands, / files, # projects, @ sessions.
pub fn search(
    query: &str,
    files: &[String],
    projects: &[Project],
    sessions: &[ChatSession],
    context: CommandContext,
) -> Vec<OmnibarEntry> {
    let query = query.trim();
    let (kind, query) = match query.chars().next() {
        Some('>') => (Some("Command"), query[1..].trim_start()),
        Some('/') => (Some("File"), query[1..].trim_start()),
        Some('#') => (Some("Project"), query[1..].trim_start()),
        Some('@') => (Some("Session"), query[1..].trim_start()),
        _ => (None, query),
    };
    let mut choices = Vec::new();
    if kind.is_none_or(|kind| kind == "Command") {
        for command in crate::commands::search(query) {
            choices.push((
                score(query, command.label, command.keywords).unwrap_or(0),
                OmnibarEntry {
                    id: format!("command-{}", command.id),
                    label: command.label.into(),
                    detail: "Command".into(),
                    kind: "Command",
                    shortcut: command.shortcut,
                    unavailable: command.command.unavailable(context).map(String::from),
                    action: OmnibarAction::Command(command.command),
                },
            ));
        }
    }
    if context.project && kind.is_none_or(|kind| kind == "File") {
        for path in files {
            let label = path.rsplit('/').next().unwrap_or(path);
            if let Some(score) = score(query, label, path) {
                choices.push((
                    score,
                    OmnibarEntry {
                        id: format!("file-{path}"),
                        label: label.into(),
                        detail: path.clone(),
                        kind: "File",
                        shortcut: "",
                        unavailable: None,
                        action: OmnibarAction::File(path.clone()),
                    },
                ));
            }
        }
    }
    if kind.is_none_or(|kind| kind == "Project") {
        for project in projects {
            let detail = format!(
                "{} · {}",
                project.mode.as_str(),
                project.path.as_deref().unwrap_or(&project.name)
            );
            if let Some(score) = score(query, &project.name, &detail) {
                choices.push((
                    score,
                    OmnibarEntry {
                        id: format!("project-{}", project.id),
                        label: project.name.clone(),
                        detail,
                        kind: "Project",
                        shortcut: "",
                        unavailable: None,
                        action: OmnibarAction::Project(project.id),
                    },
                ));
            }
        }
    }
    if kind.is_none_or(|kind| kind == "Session") {
        for session in sessions.iter().filter(|session| !session.archived) {
            let project = session
                .project_id
                .and_then(|id| projects.iter().find(|project| project.id == id));
            let unavailable = (session.project_id.is_some() && project.is_none())
                .then(|| "Project is no longer available".into());
            let detail = project
                .map_or("Chat without a project", |project| project.name.as_str())
                .to_string();
            if let Some(score) = score(query, &session.name, &detail) {
                choices.push((
                    score,
                    OmnibarEntry {
                        id: format!("session-{}", session.id),
                        label: session.name.clone(),
                        detail,
                        kind: "Session",
                        shortcut: "",
                        unavailable,
                        action: OmnibarAction::Session(session.id),
                    },
                ));
            }
        }
    }
    // Stable category order retains command keyboard behavior; name ties use stable ids.
    choices.sort_by(|(a, left), (b, right)| {
        b.cmp(a)
            .then_with(|| left.kind.cmp(right.kind))
            .then_with(|| {
                if left.kind == "Command" && right.kind == "Command" {
                    std::cmp::Ordering::Equal
                } else {
                    left.label.to_lowercase().cmp(&right.label.to_lowercase())
                }
            })
            .then_with(|| {
                if left.kind == "Command" && right.kind == "Command" {
                    std::cmp::Ordering::Equal
                } else {
                    left.id.cmp(&right.id)
                }
            })
    });
    choices
        .into_iter()
        .take(RESULT_LIMIT)
        .map(|(_, entry)| entry)
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileDiscovery {
    pub paths: Vec<String>,
    pub truncated: bool,
}
/// Enumeration state is pure; Workspace supplies directory-list primitives.
pub struct FileWalk {
    pending: VecDeque<String>,
    seen: BTreeSet<String>,
    discovery: FileDiscovery,
    directories: usize,
}
impl Default for FileWalk {
    fn default() -> Self {
        Self {
            pending: VecDeque::from([String::new()]),
            seen: BTreeSet::from([String::new()]),
            discovery: FileDiscovery::default(),
            directories: 0,
        }
    }
}
impl FileWalk {
    pub fn next_directory(&mut self) -> Option<String> {
        if self.directories == DIRECTORY_LIMIT || self.discovery.paths.len() == FILE_LIMIT {
            self.discovery.truncated |= !self.pending.is_empty();
            return None;
        }
        self.directories += 1;
        self.pending.pop_front()
    }
    pub fn add(&mut self, directory: &str, mut entries: Vec<FileEntry>) -> Result<(), VfsError> {
        openwebide_core::vfs::sort_file_entries(&mut entries);
        for entry in entries {
            let path = workspace_path(&entry.path)?;
            let (parent, _) = path.rsplit_once('/').unwrap_or(("", path.as_str()));
            if path.is_empty() || parent != directory {
                return Err(VfsError::PathEscape(entry.path));
            }
            if entry.is_dir {
                if !skip_dir(&entry.name, SearchOptions::default()) {
                    if path.split('/').count() > 64 || self.seen.len() == DIRECTORY_LIMIT {
                        self.discovery.truncated = true;
                    } else if self.seen.insert(path.clone()) {
                        self.pending.push_back(path);
                    }
                }
            } else if self.discovery.paths.len() < FILE_LIMIT {
                self.discovery.paths.push(path);
            } else {
                self.discovery.truncated = true;
            }
        }
        Ok(())
    }
    pub fn finish(mut self) -> FileDiscovery {
        self.discovery.paths.sort();
        self.discovery
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(path: &str, is_dir: bool) -> FileEntry {
        FileEntry {
            name: path.rsplit('/').next().unwrap().into(),
            path: path.into(),
            is_dir,
            size: 0,
        }
    }
    #[test]
    fn enumeration_skips_runtime_directories_validates_paths_and_caps_allocations() {
        let mut walk = FileWalk::default();
        assert_eq!(walk.next_directory().as_deref(), Some(""));
        walk.add(
            "",
            vec![
                entry("target", true),
                entry(".git", true),
                entry("src", true),
                entry("README.md", false),
            ],
        )
        .unwrap();
        assert_eq!(walk.next_directory().as_deref(), Some("src"));
        walk.add("src", vec![entry("src/main.rs", false)]).unwrap();
        assert!(walk.next_directory().is_none());
        assert_eq!(walk.finish().paths, ["README.md", "src/main.rs"]);
        let mut walk = FileWalk::default();
        assert!(walk.add("", vec![entry("../escape", false)]).is_err());
        let mut walk = FileWalk::default();
        walk.add(
            "",
            (0..=FILE_LIMIT)
                .map(|id| entry(&format!("file{id}"), false))
                .collect(),
        )
        .unwrap();
        let result = walk.finish();
        assert_eq!(result.paths.len(), FILE_LIMIT);
        assert!(result.truncated);
    }
    #[test]
    fn filename_fuzzy_search_prefixes_and_context_are_shared() {
        let paths = vec!["src/state/chat.rs".into(), "frontend/main.rs".into()];
        let context = CommandContext {
            project: true,
            ..Default::default()
        };
        assert_eq!(
            search("/sct", &paths, &[], &[], context)[0].action,
            OmnibarAction::File(paths[0].clone())
        );
        assert!(search("/main", &paths, &[], &[], CommandContext::default()).is_empty());
        assert!(
            search(">open local", &paths, &[], &[], context)
                .iter()
                .all(|entry| entry.kind == "Command")
        );
    }
}
