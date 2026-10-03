//! Per-run environment and workspace instructions shared by every agent host.
use std::collections::BTreeSet;
use std::fmt::Write;

use crate::BridgeClient;
use openwebide_core::{RunEnvironment, ToolDefinition, Vfs, VfsError, normalize_vfs_path};

const MAX_BYTES: usize = 16 * 1024;
const MAX_FILE_BYTES: usize = 8 * 1024;
const MAX_FILES: usize = 32;
const MAX_DEPTH: usize = 4;
const LIMIT_NOTE: &str = "\n[Project instructions truncated: run context limit reached.]\n";

pub struct RunContext {
    environment: RunEnvironment,
    seen: BTreeSet<(String, String)>,
    imports_attempted: usize,
    directories: BTreeSet<String>,
    remaining: usize,
    limited: bool,
}

/// Summarize the environment and the exact tool schemas offered to this run.
pub fn environment_context(environment: &RunEnvironment, tools: &[ToolDefinition]) -> String {
    let mut text = format!(
        "Environment\nCurrent date: {}\n",
        openwebide_core::format_utc_timestamp(environment.timestamp)
    );
    let mode = match environment.mode {
        Some(openwebide_core::WorkspaceMode::Local) => "local",
        Some(openwebide_core::WorkspaceMode::Remote) => "remote",
        None => "no project",
    };
    let _ = writeln!(
        text,
        "Project: {:?}\nProject root: {:?}\nWorkspace mode: {mode}",
        environment.project_name.as_deref().unwrap_or("No project"),
        environment
            .project_root
            .as_deref()
            .unwrap_or("No filesystem access")
    );
    if !tools.is_empty() {
        text.push_str("Filesystem tool paths are relative to the project root; commands run from that root.\n");
    }
    text.push_str("\nAvailable tools (use only these tools):\n");
    if tools.is_empty() {
        text.push_str("None; this is a chat-only run.\n");
    }
    for tool in tools {
        let description = tool.description.lines().next().unwrap_or("");
        let description = description.chars().take(240).collect::<String>();
        let _ = writeln!(text, "- {}: {description}", tool.name);
    }
    if text.len() > 4096 {
        text.truncate(boundary(&text, 4096));
        text.push_str("\n[Environment and tool summary truncated.]\n");
    }
    text
}

pub fn chat_context(environment: &RunEnvironment) -> String {
    let mut text = environment_context(environment, &[]);
    text.push_str("\nExecution bridge: commands unavailable in chat-only runs.\nCommand host OS and shell: not applicable.\nGit repository and branch: not inspected (no filesystem tools).\n");
    text
}

impl RunContext {
    pub fn new(environment: RunEnvironment) -> Self {
        Self {
            environment,
            seen: BTreeSet::new(),
            imports_attempted: 0,
            directories: BTreeSet::new(),
            remaining: MAX_BYTES,
            limited: false,
        }
    }

    pub async fn startup<V: Vfs, B: BridgeClient>(
        &mut self,
        vfs: &V,
        bridge: &B,
        tools: &[ToolDefinition],
    ) -> String {
        let mut text = environment_context(&self.environment, tools);
        if tools.iter().any(|tool| tool.name == "run_command") {
            let (host, status) = bridge.context_status().await;
            match host {
                Ok(host) => {
                    let _ = writeln!(
                        text,
                        "\nExecution bridge: connected\nCommand host OS: {:?}\nCommand shell: {:?}",
                        host.os, host.shell
                    );
                }
                Err(_) => {
                    let connection = if status.is_ok() {
                        "connected"
                    } else {
                        "connection not verified"
                    };
                    let _ = writeln!(
                        text,
                        "\nExecution bridge: {connection}; host metadata unavailable. Do not assume the browser's OS or shell."
                    );
                }
            }
            match status {
                Ok(status) => {
                    let _ = writeln!(text, "Git repository: yes\nGit branch: {:?}", status.branch);
                }
                Err(_) => self.git_head(vfs, &mut text).await,
            }
        } else {
            text.push_str("\nExecution bridge: not connected; commands are unavailable.\n");
            self.git_head(vfs, &mut text).await;
        }
        text.push_str("\nProject instructions apply within the stated directory. More specific directory instructions take precedence for files in that directory. Imported files share the importing file's scope. Before shell commands read or edit files in subdirectories, use read_file there to load the directory instructions.\n");
        text.push_str(&self.directory(vfs, "").await);
        text
    }

    async fn git_head<V: Vfs>(&self, vfs: &V, text: &mut String) {
        if let Ok(head) = vfs.read(".git/HEAD").await {
            let branch = head
                .trim()
                .strip_prefix("ref: refs/heads/")
                .unwrap_or("detached HEAD");
            let _ = writeln!(text, "Git repository: yes\nGit branch: {branch:?}");
        } else {
            text.push_str("Git repository: not detected (status unavailable).\n");
        }
    }

    pub async fn for_path<V: Vfs>(&mut self, vfs: &V, path: &str) -> Option<String> {
        let path = vfs.canonicalize(path).await.ok()?;
        let parent = path.rsplit_once('/').map_or("", |(dir, _)| dir);
        let mut text = String::new();
        let mut directory = String::new();
        for part in parent.split('/').filter(|part| !part.is_empty()) {
            if !directory.is_empty() {
                directory.push('/');
            }
            directory.push_str(part);
            text.push_str(&self.directory(vfs, &directory).await);
        }
        (!text.is_empty()).then_some(text)
    }

    async fn directory<V: Vfs>(&mut self, vfs: &V, directory: &str) -> String {
        if !self.directories.insert(directory.to_string()) {
            return String::new();
        }
        let mut text = String::new();
        let prefix = if directory.is_empty() {
            String::new()
        } else {
            format!("{directory}/")
        };
        // Stack preserves import order without recursive async futures.
        let mut pending = vec![
            (format!("{prefix}CLAUDE.md"), 0, false),
            (format!("{prefix}AGENTS.md"), 0, false),
        ];
        while let Some((path, depth, imported)) = pending.pop() {
            if depth > MAX_DEPTH
                || self.seen.len() >= MAX_FILES
                || (imported && self.imports_attempted >= MAX_FILES)
            {
                self.append(
                    &mut text,
                    "\n[Instruction import skipped: depth or file count limit.]\n",
                );
                continue;
            }
            if imported {
                self.imports_attempted += 1;
            }
            let canonical = match vfs.canonicalize(&path).await {
                Ok(path) => path,
                Err(VfsError::NotFound(_)) if !imported => continue,
                Err(_) => {
                    self.append(
                        &mut text,
                        &format!("\n[Instruction file unavailable: {path:?}]\n"),
                    );
                    continue;
                }
            };
            if self.seen.contains(&(canonical.clone(), String::new()))
                || self
                    .seen
                    .contains(&(canonical.clone(), directory.to_string()))
            {
                continue;
            }
            let content = match vfs.read(&canonical).await {
                Ok(content) => content,
                Err(VfsError::NotFound(_)) if !imported => continue,
                Err(_) => {
                    self.append(
                        &mut text,
                        &format!("\n[Instruction file unavailable: {path:?}]\n"),
                    );
                    continue;
                }
            };
            self.seen.insert((canonical.clone(), directory.to_string()));
            let imports = imports(&content);
            // An import-only wrapper adds no duplicate instructions.
            if !(imports.len() == 1 && content.trim() == format!("@{}", imports[0])) {
                let scope = if directory.is_empty() { "/" } else { directory };
                let inline =
                    format!("\nInstructions from {canonical:?} (scope {scope:?}):\n{content}\n");
                if content.len() <= MAX_FILE_BYTES
                    && inline.len() <= self.remaining.saturating_sub(LIMIT_NOTE.len())
                {
                    self.append(&mut text, &inline);
                } else {
                    // Never give the model an incomplete instruction file. Pointers have a
                    // separate bounded file count so exhausted inline space cannot hide them.
                    let _ = writeln!(
                        text,
                        "\nRequired instructions deferred: {canonical:?} (scope {scope:?}, {} bytes). Before working in this scope, use read_file to read the complete file; follow its offset/limit continuation hints until finished. Follow project-relative @imports too. The instructions were not included inline because they exceed the startup context budget.",
                        content.len()
                    );
                }
            }
            let parent = canonical.rsplit_once('/').map_or("", |(dir, _)| dir);
            for import in imports.into_iter().rev() {
                if import.starts_with(['/', '~', '\\']) || import.contains(':') {
                    self.append(
                        &mut text,
                        &format!("[Import outside project skipped: {import:?}]\n"),
                    );
                    continue;
                }
                match normalize_vfs_path(&format!("{parent}/{import}")) {
                    Ok(path) => pending.push((path, depth + 1, true)),
                    Err(_) => self.append(
                        &mut text,
                        &format!("[Import outside project skipped: {import:?}]\n"),
                    ),
                }
            }
        }
        text
    }

    fn append(&mut self, text: &mut String, value: &str) {
        if self.limited {
            return;
        }
        let available = self.remaining.saturating_sub(LIMIT_NOTE.len());
        let end = boundary(value, available);
        text.push_str(&value[..end]);
        self.remaining = self.remaining.saturating_sub(end);
        if end < value.len() {
            text.push_str(LIMIT_NOTE);
            self.limited = true;
        }
    }
}

fn boundary(text: &str, max: usize) -> usize {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    end
}

/// Markdown imports: ignore fenced/inline code and email addresses; support escaped spaces.
fn imports(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    let mut code_ticks = 0;
    for line in text.lines() {
        if line.starts_with("    ") || line.starts_with('\t') {
            continue;
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            let marker = trimmed.chars().next().unwrap_or('`');
            let length = trimmed.chars().take_while(|c| *c == marker).count();
            if let Some((open, size)) = fence {
                if marker == open && length >= size && trimmed[length..].trim().is_empty() {
                    fence = None;
                }
            } else {
                fence = Some((marker, length));
            }
            continue;
        }
        if fence.is_some() {
            continue;
        }
        let mut chars = line.chars().peekable();
        let mut previous = ' ';
        while let Some(c) = chars.next() {
            if c == '`' {
                let mut length = 1;
                while chars.peek() == Some(&'`') {
                    chars.next();
                    length += 1;
                }
                if code_ticks == 0 {
                    code_ticks = length;
                } else if code_ticks == length {
                    code_ticks = 0;
                }
                previous = c;
                continue;
            }
            if c == '@' && code_ticks == 0 && (previous.is_whitespace() || previous == '(') {
                let mut path = String::new();
                while let Some(&next) = chars.peek() {
                    if next.is_whitespace() || matches!(next, '`' | '"' | '\'' | ')' | ',') {
                        break;
                    }
                    chars.next();
                    if next == '\\' && chars.peek().is_some_and(|c| c.is_whitespace()) {
                        path.push(chars.next().unwrap_or(' '));
                    } else {
                        path.push(next);
                    }
                }
                if !path.is_empty() {
                    result.push(path);
                }
            }
            previous = c;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use openwebide_core::MemoryVfs;
    #[test]
    fn long_claude_instructions_keep_imports_and_defer_the_whole_file() {
        futures::executor::block_on(async {
            let vfs = MemoryVfs::new();
            vfs.write(
                "CLAUDE.md",
                &format!("{}\n@rules.md", "rule\n".repeat(2000)),
            )
            .await
            .unwrap();
            vfs.write("rules.md", "Imported rule beyond the inline limit")
                .await
                .unwrap();
            let mut context = RunContext::new(RunEnvironment::default());
            let text = context.startup(&vfs, &crate::NoopBridgeClient, &[]).await;
            assert!(text.contains("Required instructions deferred: \"CLAUDE.md\""));
            assert!(text.contains("read_file") && text.contains("offset/limit"));
            assert!(text.contains("Imported rule beyond the inline limit"));
            assert!(!text.contains("rule\nrule"));
        });
    }

    #[test]
    fn import_syntax() {
        assert_eq!(
            imports(
                "@AGENTS.md\n`@no.md` user@example.com @docs/a\\ b.md\n```\n@no.md\n```\n@\"no.md\""
            ),
            vec!["AGENTS.md", "docs/a b.md"]
        );
    }
    #[test]
    fn instructions_are_scoped_deduplicated_confined_and_bounded() {
        futures::executor::block_on(async {
            let vfs = MemoryVfs::new();
            vfs.write("AGENTS.md", "root @docs/shared.md")
                .await
                .unwrap();
            vfs.write("CLAUDE.md", "@AGENTS.md").await.unwrap();
            vfs.write("docs/shared.md", "shared @../AGENTS.md @../../escape.md")
                .await
                .unwrap();
            vfs.write("src/AGENTS.md", "nested").await.unwrap();
            let mut context = RunContext::new(RunEnvironment::default());
            let startup = context.startup(&vfs, &crate::NoopBridgeClient, &[]).await;
            assert_eq!(startup.matches("root @docs").count(), 1);
            assert!(startup.contains("outside project skipped"));
            assert!(!startup.contains("nested"));
            let nested = context.for_path(&vfs, "src/lib.rs").await.unwrap();
            assert!(nested.contains("nested") && nested.contains("scope \"src\""));
            assert!(context.for_path(&vfs, "src/other.rs").await.is_none());
            vfs.write("huge/AGENTS.md", &"é".repeat(10000))
                .await
                .unwrap();
            let huge = context.for_path(&vfs, "huge/file").await.unwrap();
            assert!(huge.contains("Required instructions deferred: \"huge/AGENTS.md\""));
            assert!(huge.contains("read_file") && !huge.contains("éé"));
        });
    }
    #[test]
    fn import_depth_total_budget_and_sibling_scopes_are_enforced() {
        futures::executor::block_on(async {
            let vfs = MemoryVfs::new();
            vfs.write("AGENTS.md", "@one.md").await.unwrap();
            for (from, to) in [
                ("one", "two"),
                ("two", "three"),
                ("three", "four"),
                ("four", "five"),
            ] {
                vfs.write(&format!("{from}.md"), &format!("{from} @{to}.md"))
                    .await
                    .unwrap();
            }
            vfs.write("five.md", "must not load").await.unwrap();
            vfs.write("shared.md", "Shared scoped rule").await.unwrap();
            for dir in ["a", "b"] {
                vfs.write(&format!("{dir}/CLAUDE.md"), "@../shared.md")
                    .await
                    .unwrap();
            }
            let mut context = RunContext::new(RunEnvironment::default());
            let root = context.startup(&vfs, &crate::NoopBridgeClient, &[]).await;
            assert!(root.contains("four") && !root.contains("must not load"));
            assert!(root.contains("depth or file count limit"));
            for dir in ["a", "b"] {
                let text = context
                    .for_path(&vfs, &format!("{dir}/file"))
                    .await
                    .unwrap();
                assert!(text.contains("Shared scoped rule"));
                assert!(text.contains(&format!("scope {dir:?}")));
            }
            let mut context = RunContext::new(RunEnvironment::default());
            let mut total = String::new();
            for dir in ["large1", "large2", "large3"] {
                vfs.write(&format!("{dir}/AGENTS.md"), &"é".repeat(4000))
                    .await
                    .unwrap();
                if let Some(text) = context.for_path(&vfs, &format!("{dir}/file")).await {
                    total.push_str(&text);
                }
            }
            assert!(total.len() <= MAX_BYTES + 1024);
            assert!(total.contains("Required instructions deferred: \"large3/AGENTS.md\""));
        });
    }
}
