//! Project tooling suggestions use Workspace in either execution mode.
#[cfg(target_arch = "wasm32")]
use crate::workspace::Workspace;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectTools {
    pub language: &'static str,
    pub test: &'static str,
    pub filter_flag: &'static str,
    pub linters: &'static str,
}
impl ProjectTools {
    pub fn test_command(&self, filter: &str) -> String {
        if filter.is_empty() {
            self.test.into()
        } else {
            format!(
                "{}{} {}",
                self.test,
                self.filter_flag,
                crate::text::shell_quote(filter)
            )
        }
    }
}
#[cfg(target_arch = "wasm32")]
pub async fn detect(workspace: &Workspace) -> Result<Option<ProjectTools>, String> {
    let entries = workspace
        .list("")
        .await
        .map_err(|error| error.to_string())?;
    from_entries(workspace, &entries).await
}

#[cfg(target_arch = "wasm32")]
pub async fn from_entries(
    workspace: &Workspace,
    entries: &[openwebide_core::FileEntry],
) -> Result<Option<ProjectTools>, String> {
    let names = entries
        .iter()
        .filter(|entry| !entry.is_dir)
        .map(|entry| entry.name.as_str())
        .collect::<Vec<_>>();
    let has = |name| names.contains(&name);
    let tools = if has("Cargo.toml") {
        ProjectTools {
            language: "Rust",
            test: "cargo test",
            filter_flag: " --",
            linters: "cargo clippy",
        }
    } else if has("package.json") {
        let package = workspace
            .read("package.json")
            .await
            .map_err(|error| error.to_string())?;
        let package: serde_json::Value = serde_json::from_str(&package)
            .map_err(|error| format!("Invalid package.json: {error}"))?;
        if package
            .pointer("/scripts/test")
            .and_then(serde_json::Value::as_str)
            .is_none()
        {
            return Ok(None);
        }
        ProjectTools {
            language: "JavaScript / TypeScript",
            test: if has("pnpm-lock.yaml") {
                "pnpm test"
            } else if has("yarn.lock") {
                "yarn test"
            } else if has("bun.lock") || has("bun.lockb") {
                "bun test"
            } else {
                "npm test"
            },
            filter_flag: " --",
            linters: "Biome or ESLint",
        }
    } else if has("go.mod") {
        ProjectTools {
            language: "Go",
            test: "go test ./...",
            filter_flag: " -run",
            linters: "go vet ./...",
        }
    } else if has("pyproject.toml")
        || has("pytest.ini")
        || has("setup.py")
        || has("requirements.txt")
    {
        ProjectTools {
            language: "Python",
            test: "python -m pytest",
            filter_flag: " -k",
            linters: "Ruff",
        }
    } else if names.iter().any(|name| {
        std::path::Path::new(name)
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                ["slnx", "sln", "csproj"]
                    .iter()
                    .any(|expected| extension.eq_ignore_ascii_case(expected))
            })
    }) {
        ProjectTools {
            language: ".NET",
            test: "dotnet test",
            filter_flag: " --filter",
            linters: "dotnet format --verify-no-changes",
        }
    } else {
        return Ok(None);
    };
    Ok(Some(tools))
}
