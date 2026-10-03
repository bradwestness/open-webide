//! Turn checkpoints and filesystem restoration policy shared by every host.
use std::collections::BTreeMap;
use std::future::Future;

use crate::Vfs;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};

use crate::{ConversationEntry, Role, vfs::workspace_path};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RewindFile {
    pub path: String,
    pub before: Option<String>,
    #[serde(default)]
    pub backup_path: Option<String>,
    pub after: String,
    #[serde(default)]
    pub binary_before: Option<String>,
    #[serde(default)]
    pub binary_after: Option<String>,
    #[serde(default)]
    pub deleted: bool,
}

/// A bounded project snapshot. Contents are encoded to preserve arbitrary bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectCheckpoint {
    pub before: BTreeMap<String, String>,
    pub after: Option<BTreeMap<String, String>>,
}

/// Direct file edits also checkpoint files inside generated folders.
pub async fn capture_file(vfs: &impl Vfs, path: &str) -> Result<BTreeMap<String, String>, String> {
    let path = workspace_path(path).map_err(|e| e.to_string())?;
    let canonical = vfs.canonicalize(&path).await.map_err(|e| e.to_string())?;
    if canonical
        .split('/')
        .any(|part| part.eq_ignore_ascii_case(".git"))
    {
        return Err("Cannot checkpoint Git internals".into());
    }
    match vfs.read_bytes(&path).await {
        Ok(bytes) => Ok([(path, STANDARD.encode(bytes))].into()),
        Err(crate::VfsError::NotFound(_)) => Ok(BTreeMap::new()),
        Err(error) => Err(error.to_string()),
    }
}

/// Shared traversal policy; adapters only enumerate and read bytes. Refuse to
/// execute a mutating tool when a complete checkpoint cannot fit the limit.
pub async fn capture_project(vfs: &impl Vfs) -> Result<BTreeMap<String, String>, String> {
    let mut directories = vec![String::new()];
    let mut files = BTreeMap::new();
    let mut visited = std::collections::BTreeSet::new();
    let mut total = 0usize;
    while let Some(dir) = directories.pop() {
        let canonical = match vfs.canonicalize(&dir).await {
            Ok(path) => path,
            Err(crate::VfsError::PathEscape(_)) => continue,
            Err(error) => return Err(error.to_string()),
        };
        if canonical != dir {
            continue;
        }
        if canonical.split('/').any(|part| {
            part.eq_ignore_ascii_case(".git") || crate::vfs::SEARCH_SKIP_DIRS.contains(&part)
        }) || !visited.insert(canonical)
        {
            continue;
        }
        for entry in vfs.list(&dir).await.map_err(|e| e.to_string())? {
            if entry.name == ".spin" {
                continue;
            }
            let path = workspace_path(&entry.path).map_err(|e| e.to_string())?;
            if path.split('/').any(|part| {
                part.eq_ignore_ascii_case(".git") || part == crate::vfs::AGENT_BACKUP_DIR
            }) {
                continue;
            }
            if entry.is_dir {
                if !crate::vfs::SEARCH_SKIP_DIRS.contains(&entry.name.as_str()) {
                    directories.push(path);
                }
                continue;
            }
            // Snapshot physical project files once; aliases and escaping links
            // cannot bring external files into the rewind boundary.
            let canonical = match vfs.canonicalize(&path).await {
                Ok(path) => path,
                Err(crate::VfsError::PathEscape(_)) => continue,
                Err(error) => return Err(error.to_string()),
            };
            if canonical != path {
                continue;
            }
            if canonical
                .split('/')
                .any(|part| part.eq_ignore_ascii_case(".git"))
            {
                continue;
            }
            let bytes = vfs.read_bytes(&path).await.map_err(|e| e.to_string())?;
            total += bytes.len();
            if total > 32 * 1024 * 1024 || files.len() >= 10000 {
                return Err("Project checkpoint exceeds 32 MiB or 10,000 files".into());
            }
            files.insert(path, STANDARD.encode(bytes));
        }
    }
    Ok(files)
}

impl ProjectCheckpoint {
    /// Persist only changed paths once the tool has finished.
    pub fn compact(&mut self) {
        if let Some(after) = &mut self.after {
            let unchanged: Vec<_> = self
                .before
                .iter()
                .filter(|(path, content)| after.get(*path) == Some(*content))
                .map(|(path, _)| path.clone())
                .collect();
            for path in unchanged {
                self.before.remove(&path);
                after.remove(&path);
            }
        }
    }

    pub fn changes(&self) -> Result<Vec<RewindFile>, String> {
        let after = self
            .after
            .as_ref()
            .ok_or("Project checkpoint is incomplete")?;
        let paths: std::collections::BTreeSet<_> = self.before.keys().chain(after.keys()).collect();
        paths
            .into_iter()
            .filter(|path| self.before.get(*path) != after.get(*path))
            .map(|path| {
                let before = self
                    .before
                    .get(path)
                    .map(|s| STANDARD.decode(s))
                    .transpose()
                    .map_err(|e| e.to_string())?;
                let next = after
                    .get(path)
                    .map(|s| STANDARD.decode(s))
                    .transpose()
                    .map_err(|e| e.to_string())?;
                let (before, binary_before) = content_fields(before);
                let deleted = next.is_none();
                let (after, binary_after) = content_fields(next);
                Ok(RewindFile {
                    path: path.clone(),
                    before,
                    after: after.unwrap_or_default(),
                    backup_path: None,
                    binary_before,
                    binary_after,
                    deleted,
                })
            })
            .collect()
    }
}
fn content_fields(bytes: Option<Vec<u8>>) -> (Option<String>, Option<String>) {
    match bytes {
        None => (None, None),
        Some(bytes) => match String::from_utf8(bytes) {
            Ok(text) => (Some(text), None),
            Err(error) => (None, Some(STANDARD.encode(error.into_bytes()))),
        },
    }
}
impl RewindFile {
    pub fn before_bytes(&self) -> Result<Option<Vec<u8>>, String> {
        decode_content(&self.before, &self.binary_before)
    }
    pub fn after_bytes(&self) -> Result<Option<Vec<u8>>, String> {
        if self.deleted {
            Ok(None)
        } else {
            decode_content(&Some(self.after.clone()), &self.binary_after)
        }
    }
}
fn decode_content(
    text: &Option<String>,
    binary: &Option<String>,
) -> Result<Option<Vec<u8>>, String> {
    binary
        .as_ref()
        .map(|s| STANDARD.decode(s).map(Some).map_err(|e| e.to_string()))
        .unwrap_or_else(|| Ok(text.as_ref().map(|s| s.as_bytes().to_vec())))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RewindPlan {
    /// Restore the conversation and files to immediately before this prompt.
    pub message_id: i64,
    pub prompt: String,
    pub files: Vec<RewindFile>,
}

impl RewindPlan {
    pub fn from_conversation(
        entries: &[ConversationEntry],
        message_id: i64,
    ) -> Result<Self, String> {
        let prompt = entries
            .iter()
            .find_map(|entry| match entry {
                ConversationEntry::Message(message)
                    if message.id == message_id && message.role == Role::User =>
                {
                    Some(message.content.clone())
                }
                _ => None,
            })
            .ok_or("Checkpoint prompt is no longer available")?;
        let mut files = BTreeMap::<String, RewindFile>::new();
        for entry in entries {
            let ConversationEntry::ToolStep(step) = entry else {
                continue;
            };
            if step.anchor_message_id < message_id {
                continue;
            }
            if step.ok.is_none() {
                return Err("Wait for the run to finish before rewinding".into());
            }
            if let Some(checkpoint) = &step.checkpoint {
                for change in checkpoint.changes()? {
                    let path = workspace_path(&change.path).map_err(|e| e.to_string())?;
                    if path
                        .split('/')
                        .any(|part| part.eq_ignore_ascii_case(".git"))
                    {
                        return Err("Cannot rewind Git internals".into());
                    }
                    if let Some(file) = files.get_mut(&path) {
                        if file.after_bytes()? != change.before_bytes()? {
                            return Err(format!("{path} changed outside this conversation"));
                        }
                        file.after = change.after;
                        file.binary_after = change.binary_after;
                        file.deleted = change.deleted;
                    } else {
                        files.insert(path, change);
                    }
                }
                continue;
            }
            if matches!(
                step.name.as_str(),
                "run_command" | "git_commit" | "git_branch"
            ) {
                return Err("This older turn has no project checkpoint".into());
            }
            if step.ok != Some(true) {
                continue;
            }
            let Some(diff) = &step.diff else { continue };
            let path = workspace_path(&diff.path).map_err(|e| e.to_string())?;
            if path
                .split('/')
                .any(|part| part.eq_ignore_ascii_case(".git"))
            {
                return Err("Cannot rewind Git internals".into());
            }
            if diff.old_unavailable && diff.backup_path.is_none() {
                return Err(format!(
                    "{} has a binary backup; restore it in the edit review first",
                    diff.path
                ));
            }
            if let Some(backup) = &diff.backup_path {
                let backup = workspace_path(backup).map_err(|error| error.to_string())?;
                if !backup.starts_with(&format!("{}/", crate::vfs::AGENT_BACKUP_DIR)) {
                    return Err("Checkpoint backup is outside the agent backup folder".into());
                }
            }
            if let Some(file) = files.get_mut(&path) {
                if diff.old.as_deref() != Some(file.after.as_str()) {
                    return Err(format!(
                        "{path} changed outside this conversation; review it before rewinding"
                    ));
                }
                file.after.clone_from(&diff.new);
            } else {
                files.insert(
                    path.clone(),
                    RewindFile {
                        path,
                        before: diff.old.clone(),
                        backup_path: diff.backup_path.clone(),
                        after: diff.new.clone(),
                        binary_before: None,
                        binary_after: None,
                        deleted: false,
                    },
                );
            }
        }
        Ok(Self {
            message_id,
            prompt,
            files: files.into_values().collect(),
        })
    }
}

/// Adapters provide file primitives; validation and retry policy live here.
pub trait RewindFiles {
    fn validate(&self, path: &str) -> impl Future<Output = Result<(), String>>;
    fn read(&self, path: &str) -> impl Future<Output = Result<Option<Vec<u8>>, String>>;
    fn copy(&self, from: &str, to: &str) -> impl Future<Output = Result<(), String>>;
    fn write(&self, path: &str, content: &str) -> impl Future<Output = Result<(), String>>;
    fn write_bytes(&self, path: &str, content: &[u8]) -> impl Future<Output = Result<(), String>>;
    fn delete(&self, path: &str) -> impl Future<Output = Result<(), String>>;
}

async fn original_bytes(
    files: &impl RewindFiles,
    file: &RewindFile,
) -> Result<Option<Vec<u8>>, String> {
    if let Some(backup) = &file.backup_path {
        let bytes = files
            .read(backup)
            .await?
            .ok_or_else(|| format!("{} checkpoint backup is missing", file.path))?;
        Ok(Some(bytes))
    } else {
        file.before_bytes()
    }
}

/// Validate every file before changing any. A durable prepared rewind can be
/// retried after an interrupted write: each file may be at either endpoint.
/// Unrelated edits are never overwritten, including edits arriving mid-restore.
pub async fn verify_files(
    files: &impl RewindFiles,
    plan: &RewindPlan,
    current: impl Fn() -> bool,
) -> Result<(), String> {
    for file in &plan.files {
        if !current() {
            return Err("Workspace changed; return to this session to resume the rewind".into());
        }
        files.validate(&file.path).await?;
        let content = files.read(&file.path).await?;
        let before = original_bytes(files, file).await?;
        if content != file.after_bytes()? && content != before {
            return Err(format!(
                "{} changed after the run; revert those edits before rewinding",
                file.path
            ));
        }
    }
    Ok(())
}

pub async fn restore_files(
    files: &impl RewindFiles,
    plan: &RewindPlan,
    current: impl Fn() -> bool,
) -> Result<(), String> {
    verify_files(files, plan, &current).await?;
    for file in &plan.files {
        if !current() {
            return Err("Workspace changed; return to this session to resume the rewind".into());
        }
        files.validate(&file.path).await?;
        let content = files.read(&file.path).await?;
        let before = original_bytes(files, file).await?;
        if content == before {
            continue;
        }
        if content != file.after_bytes()? {
            return Err(format!(
                "{} changed during rewind; no further files were restored",
                file.path
            ));
        }
        if !current() {
            return Err("Workspace changed; return to this session to resume the rewind".into());
        }
        if let Some(backup) = &file.backup_path {
            files.copy(backup, &file.path).await?;
        } else {
            match before {
                Some(content) => files.write_bytes(&file.path, &content).await?,
                None => files.delete(&file.path).await?,
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChatMessage, FileDiff, ToolStep};
    fn prompt(id: i64) -> ConversationEntry {
        ConversationEntry::Message(ChatMessage {
            id,
            session_id: 1,
            role: Role::User,
            content: format!("prompt {id}"),
            created_at: 0,
            tool_calls: None,
            tool_call_id: None,
            usage: None,
        })
    }
    fn edit(anchor: i64, old: Option<&str>, new: &str) -> ConversationEntry {
        ConversationEntry::ToolStep(ToolStep {
            tool_call_id: format!("call{anchor}"),
            name: "write_file".into(),
            summary: "edit".into(),
            ok: Some(true),
            result_summary: None,
            diff: Some(FileDiff {
                path: "file.txt".into(),
                old: old.map(str::to_string),
                new: new.into(),
                old_unavailable: false,
                backup_path: None,
            }),
            anchor_message_id: anchor,
            checkpoint: None,
        })
    }
    #[test]
    fn checkpoints_coalesce_writes_across_turns_and_detect_intervening_edits() {
        let mut entries = vec![
            prompt(1),
            edit(1, Some("original"), "one"),
            prompt(2),
            edit(2, Some("one"), "two"),
        ];
        let first = RewindPlan::from_conversation(&entries, 1).unwrap();
        assert_eq!(first.files[0].before.as_deref(), Some("original"));
        assert_eq!(first.files[0].after, "two");
        assert_eq!(
            RewindPlan::from_conversation(&entries, 2).unwrap().files[0]
                .before
                .as_deref(),
            Some("one")
        );
        entries[3] = edit(2, Some("manual change"), "two");
        assert!(
            RewindPlan::from_conversation(&entries, 1)
                .unwrap_err()
                .contains("outside")
        );
        assert!(RewindPlan::from_conversation(&entries, 99).is_err());
    }
    #[test]
    fn unsafe_and_incomplete_steps_cannot_be_rewound() {
        for (name, ok) in [
            ("run_command", Some(false)),
            ("git_commit", Some(true)),
            ("write_file", None),
        ] {
            let ConversationEntry::ToolStep(mut step) = edit(1, None, "new") else {
                unreachable!()
            };
            step.name = name.into();
            step.ok = ok;
            assert!(
                RewindPlan::from_conversation(&[prompt(1), ConversationEntry::ToolStep(step)], 1)
                    .is_err()
            );
        }
        let ConversationEntry::ToolStep(mut step) = edit(1, None, "new") else {
            unreachable!()
        };
        step.diff.as_mut().unwrap().path = "../escape".into();
        assert!(
            RewindPlan::from_conversation(&[prompt(1), ConversationEntry::ToolStep(step)], 1)
                .is_err()
        );
    }
}
