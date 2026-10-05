//! EditorConfig policy is independent of the filesystem. Workspace supplies sources
//! from the project root to the file's parent; no host filesystem API is used here.
use ec4rs::{ConfigParser, Properties, PropertiesSource};
use serde::{Deserialize, Serialize};

use super::{Document, Edit, EditError, IndentStyle, Indentation};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LineEnding {
    #[default]
    Lf,
    CrLf,
    Cr,
}
impl LineEnding {
    pub const fn text(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
            Self::Cr => "\r",
        }
    }
    pub fn detect(text: &str) -> Self {
        for (i, byte) in text.bytes().enumerate() {
            match byte {
                b'\r' => {
                    return if text.as_bytes().get(i + 1) == Some(&b'\n') {
                        Self::CrLf
                    } else {
                        Self::Cr
                    };
                }
                b'\n' => return Self::Lf,
                _ => {}
            }
        }
        Self::Lf
    }
}

/// User defaults are stored once in user-scoped database settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorPreferences {
    pub indentation: Indentation,
    pub word_wrap: bool,
    pub show_whitespace: bool,
}
impl EditorPreferences {
    pub fn normalized(mut self) -> Self {
        self.indentation.width = self.indentation.width();
        self.indentation.tab_width = self.indentation.tab_width();
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorRules {
    pub indentation: Indentation,
    /// None means preserve existing line endings, including mixed endings.
    pub line_ending: Option<LineEnding>,
    pub final_newline: Option<bool>,
    pub trim_trailing_whitespace: bool,
    pub source: Option<String>,
}

pub struct ConfigSource<'a> {
    /// Directory relative to the workspace, empty for the project root.
    pub directory: &'a str,
    pub text: &'a str,
}

/// Discover project-bounded config files through the shared workspace contract.
/// Hosts only provide list/read primitives. Failed reads use detected/default
/// settings with a warning; a stale account/project never returns usable rules.
pub async fn load_rules(
    files: &impl crate::workspace_entries::WorkspaceEntries,
    path: &str,
    text: &str,
    defaults: EditorPreferences,
    current: impl Fn() -> bool,
) -> Result<(EditorRules, Vec<String>), String> {
    const MAX_CONFIG_BYTES: u64 = 256 * 1024;
    let path = crate::vfs::workspace_path(path).map_err(|error| error.to_string())?;
    let mut directory = crate::workspace_entries::parent(&path).to_string();
    let mut sources = Vec::new();
    let mut warnings = Vec::new();
    loop {
        if !current() {
            return Err("Editor configuration interrupted by a workspace change".into());
        }
        let name = if directory.is_empty() {
            ".editorconfig".to_string()
        } else {
            format!("{directory}/.editorconfig")
        };
        let found: Result<Option<String>, String> = async {
            let entries = files.list(&directory).await?;
            if !current() {
                return Ok(None);
            }
            let Some(entry) = entries
                .iter()
                .find(|entry| entry.name == ".editorconfig" && !entry.is_dir)
            else {
                return Ok(None);
            };
            if entry.size > MAX_CONFIG_BYTES {
                return Err("file exceeds the 256 KiB editor configuration limit".into());
            }
            let bytes = files.read_bytes(&name).await?;
            if bytes.len() > usize::try_from(MAX_CONFIG_BYTES).unwrap_or(usize::MAX) {
                return Err("file exceeds the 256 KiB editor configuration limit".into());
            }
            String::from_utf8(bytes)
                .map(Some)
                .map_err(|_| "configuration is not UTF-8".into())
        }
        .await;
        if !current() {
            return Err("Editor configuration interrupted by a workspace change".into());
        }
        match found {
            Ok(Some(config)) => {
                let stop = ConfigParser::new(config.as_bytes()).is_ok_and(|parser| parser.is_root);
                sources.push((directory.clone(), config));
                if stop {
                    break;
                }
            }
            Ok(None) => {}
            Err(error) => warnings.push(format!("Could not read {name}: {error}")),
        }
        if directory.is_empty() {
            break;
        }
        directory = crate::workspace_entries::parent(&directory).to_string();
    }
    sources.reverse();
    let sources = sources
        .iter()
        .map(|(directory, text)| ConfigSource { directory, text })
        .collect::<Vec<_>>();
    let (rules, parsing_warnings) = resolve_rules(&path, text, defaults, &sources);
    warnings.extend(parsing_warnings);
    Ok((rules, warnings))
}

/// Resolve rules with defaults < file detection < EditorConfig. Sources are
/// ordered outermost first. A nested root discards all outer properties.
/// Invalid config files are ignored atomically and reported to the caller.
pub fn resolve_rules(
    path: &str,
    text: &str,
    defaults: EditorPreferences,
    sources: &[ConfigSource<'_>],
) -> (EditorRules, Vec<String>) {
    let mut properties = Properties::new();
    let mut source = None;
    let mut warnings = Vec::new();
    for config in sources {
        let relative = if config.directory.is_empty() {
            Some(path)
        } else {
            path.strip_prefix(config.directory)
                .and_then(|tail| tail.strip_prefix('/'))
        };
        let Some(relative) = relative else {
            continue;
        };
        let name = if config.directory.is_empty() {
            ".editorconfig".to_string()
        } else {
            format!("{}/.editorconfig", config.directory)
        };
        let parsed = (|| {
            let mut parser =
                ConfigParser::new(config.text.as_bytes()).map_err(|error| error.to_string())?;
            let mut next = if parser.is_root {
                Properties::new()
            } else {
                properties.clone()
            };
            parser
                .apply_to(&mut next, relative)
                .map_err(|error| error.to_string())?;
            Ok::<_, String>(next)
        })();
        match parsed {
            Ok(next) => {
                properties = next;
                source = Some(name);
            }
            Err(error) => warnings.push(format!("Could not read {name}: {error}")),
        }
    }
    for (_, value) in properties.iter_mut() {
        *value = value.to_lowercase();
    }
    properties.use_fallbacks();
    let raw = |key: &str| properties.get_raw_for_key(key).filter_unset().into_option();
    let mut indentation = detect_indentation(text, defaults.normalized().indentation);
    if let Some(style) = raw("indent_style") {
        match style {
            "tab" => indentation.style = IndentStyle::Tabs,
            "space" => indentation.style = IndentStyle::Spaces,
            _ => {}
        }
    }
    if let Some(width) = raw("tab_width")
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|width| *width > 0)
    {
        indentation.tab_width = width.clamp(1, 16);
    }
    if let Some(width) = raw("indent_size") {
        if width == "tab" {
            indentation.width = indentation.tab_width();
        } else if let Ok(width) = width.parse::<usize>()
            && width > 0
        {
            indentation.width = width.clamp(1, 16);
        }
    }
    let rules = EditorRules {
        indentation,
        line_ending: match raw("end_of_line") {
            Some("lf") => Some(LineEnding::Lf),
            Some("crlf") => Some(LineEnding::CrLf),

            _ => None,
        },
        final_newline: match raw("insert_final_newline") {
            Some("true") => Some(true),
            Some("false") => Some(false),
            _ => None,
        },
        trim_trailing_whitespace: raw("trim_trailing_whitespace") == Some("true"),
        source,
    };
    (rules, warnings)
}

/// Detect indentation without assuming that absolute nesting depth is the unit.
/// Adjacent nonempty indentation increases vote for a unit; obvious unindented
/// text doesn't force the user's defaults to one space.
fn detect_indentation(text: &str, defaults: Indentation) -> Indentation {
    let mut tabs = 0;
    let mut spaces = 0;
    let mut previous = 0;
    let mut votes = [0usize; 17];
    for line in text.lines().take(10_000) {
        if line.trim().is_empty() {
            continue;
        }
        let prefix = line
            .bytes()
            .take_while(|byte| matches!(byte, b' ' | b'\t'))
            .collect::<Vec<_>>();
        if prefix.contains(&b'\t') {
            tabs += 1;
            previous = 0;
            continue;
        }
        let count = prefix.len();
        if count > 0 {
            spaces += 1;
            let difference = count.abs_diff(previous);
            if (2..=16).contains(&difference) {
                votes[difference] += 1;
            }
        }
        previous = count;
    }
    let mut result = defaults;
    if tabs > spaces {
        result.style = IndentStyle::Tabs;
    } else if spaces > tabs {
        result.style = IndentStyle::Spaces;
        if let Some((width, _)) = votes
            .iter()
            .enumerate()
            .filter(|(_, votes)| **votes > 0)
            .max_by_key(|(width, votes)| (**votes, std::cmp::Reverse(*width)))
        {
            result.width = width;
            result.tab_width = width;
        }
    }
    result
}

impl Document {
    /// Convert only leading indentation, retaining visual columns and all other
    /// bytes. Conversion is explicit and one undo step, never a typing side effect.
    pub fn convert_indentation(
        &mut self,
        indentation: Indentation,
        old_tab_width: usize,
    ) -> Result<bool, EditError> {
        let mut edits = Vec::new();
        let mut start = 0;
        for line in self.text.split_inclusive('\n') {
            let prefix: String = line
                .chars()
                .take_while(|ch| matches!(ch, ' ' | '\t'))
                .collect();
            let width = old_tab_width.clamp(1, 16);
            let columns = prefix.chars().fold(0, |column, ch| {
                if ch == '\t' {
                    column + width - column % width
                } else {
                    column + 1
                }
            });
            let replacement = indentation.columns(columns);
            if replacement != prefix {
                edits.push(Edit::replace(start..start + prefix.len(), replacement));
            }
            start += line.len();
        }
        self.apply_mapped(edits)
    }

    /// Apply explicit save policies as one undoable command. No policy means
    /// preserve bytes; an empty document never gains a final newline.
    pub fn prepare_save(&mut self, rules: &EditorRules) -> Result<bool, EditError> {
        let mut edits = Vec::new();
        let bytes = self.text.as_bytes();
        let mut start = 0;
        let mut position = 0;
        while position < bytes.len() {
            if !matches!(bytes[position], b'\r' | b'\n') {
                position += 1;
                continue;
            }
            let end = position
                + if bytes[position] == b'\r' && bytes.get(position + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                };
            let trim = if rules.trim_trailing_whitespace {
                self.text[start..position]
                    .trim_end_matches([' ', '\t'])
                    .len()
                    + start
            } else {
                position
            };
            let ending = rules
                .line_ending
                .map_or(&self.text[position..end], |ending| ending.text());
            if self.text[trim..end] != *ending {
                edits.push(Edit::replace(trim..end, ending));
            }
            start = end;
            position = end;
        }
        if start < self.text.len() && rules.trim_trailing_whitespace {
            let trim = self.text[start..].trim_end_matches([' ', '\t']).len() + start;
            if trim < self.text.len() {
                edits.push(Edit::replace(trim..self.text.len(), ""));
            }
        }
        let candidate = super::replace_edits(&self.text, &edits).0;
        if !candidate.is_empty() {
            match rules.final_newline {
                Some(true) if !candidate.ends_with(['\r', '\n']) => {
                    let ending = rules
                        .line_ending
                        .unwrap_or_else(|| LineEnding::detect(&self.text))
                        .text();
                    if let Some(last) = edits
                        .last_mut()
                        .filter(|edit| edit.range.end == self.text.len())
                    {
                        last.text.push_str(ending);
                    } else {
                        edits.push(Edit::replace(self.text.len()..self.text.len(), ending));
                    }
                }
                Some(false) => {
                    let cutoff = if rules.trim_trailing_whitespace {
                        self.text.trim_end_matches(['\r', '\n', ' ', '\t']).len()
                    } else {
                        self.text.trim_end_matches(['\r', '\n']).len()
                    };
                    if cutoff < self.text.len() {
                        edits.retain(|edit| edit.range.end <= cutoff && edit.range.start < cutoff);
                        edits.push(Edit::replace(cutoff..self.text.len(), ""));
                    }
                }
                _ => {}
            }
        }
        self.apply_mapped(edits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Selection;
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct ConfigFiles {
        files: BTreeMap<String, Vec<u8>>,
        listed: RefCell<Vec<String>>,
        fail: Option<String>,
        stale: Cell<bool>,
        invalidate_on_read: bool,
    }
    impl crate::workspace_entries::WorkspaceEntries for ConfigFiles {
        async fn canonicalize(&self, path: &str) -> Result<String, String> {
            Ok(path.into())
        }
        async fn list(&self, path: &str) -> Result<Vec<crate::FileEntry>, String> {
            self.listed.borrow_mut().push(path.into());
            if self.fail.as_deref() == Some(path) {
                return Err("permission denied".into());
            }
            Ok(self
                .files
                .iter()
                .filter(|(name, _)| crate::workspace_entries::parent(name) == path)
                .map(|(name, bytes)| crate::FileEntry {
                    name: name.rsplit('/').next().unwrap().into(),
                    path: name.clone(),
                    is_dir: false,
                    size: u64::try_from(bytes.len()).unwrap(),
                })
                .collect())
        }
        async fn read_bytes(&self, path: &str) -> Result<Vec<u8>, String> {
            if self.invalidate_on_read {
                self.stale.set(true);
            }
            self.files
                .get(path)
                .cloned()
                .ok_or_else(|| "not found".into())
        }
        async fn create(&self, _: &str, _: crate::vfs::VfsEntryKind) -> Result<(), String> {
            Err("read-only test workspace".into())
        }
        async fn write_bytes(&self, _: &str, _: &[u8]) -> Result<(), String> {
            Err("read-only test workspace".into())
        }
        async fn delete(&self, _: &str) -> Result<(), String> {
            Err("read-only test workspace".into())
        }
    }

    #[test]
    fn discovery_stops_at_root_and_discards_stale_reads() {
        let mut files = ConfigFiles::default();
        files
            .files
            .insert(".editorconfig".into(), b"[*]\nindent_size=8".to_vec());
        files.files.insert(
            "src/.editorconfig".into(),
            b"root=true\n[*.rs]\nindent_size=2".to_vec(),
        );
        let (rules, warnings) = futures::executor::block_on(load_rules(
            &files,
            "src/deep/a.rs",
            "",
            EditorPreferences::default(),
            || !files.stale.get(),
        ))
        .unwrap();
        assert!(warnings.is_empty());
        assert_eq!(rules.indentation.width, 2);
        assert_eq!(*files.listed.borrow(), ["src/deep", "src"]);
        files.invalidate_on_read = true;
        assert!(
            futures::executor::block_on(load_rules(
                &files,
                "src/a.rs",
                "",
                EditorPreferences::default(),
                || !files.stale.get()
            ))
            .is_err()
        );
    }

    #[test]
    fn failed_invalid_and_oversized_configs_use_detection_with_warnings() {
        for bytes in [vec![255], vec![b' '; 256 * 1024 + 1]] {
            let mut files = ConfigFiles {
                fail: Some("src".into()),
                ..Default::default()
            };
            files.files.insert(".editorconfig".into(), bytes);
            let (rules, warnings) = futures::executor::block_on(load_rules(
                &files,
                "src/a.rs",
                "x\n  y",
                EditorPreferences::default(),
                || true,
            ))
            .unwrap();
            assert_eq!(rules.indentation.width, 2);
            assert_eq!(warnings.len(), 2);
        }
    }

    #[test]
    fn save_policies_keep_carets_on_unrelated_lines_and_handle_eof_without_overlap() {
        for text in ["😀a  \r\nb\r\n\r\n", "😀a  \r\nb  ", "😀a  \r\n  \r\n"] {
            for final_newline in [None, Some(true), Some(false)] {
                let mut doc = Document::new(text);
                doc.set_selections(vec![Selection::caret(4)]).unwrap();
                let mut rules = resolve_rules("a", text, EditorPreferences::default(), &[]).0;
                rules.trim_trailing_whitespace = true;
                rules.line_ending = Some(LineEnding::Lf);
                rules.final_newline = final_newline;
                doc.prepare_save(&rules).unwrap();
                assert_eq!(doc.selections(), [Selection::caret(4)]);
                assert!(!doc.text().contains('\r'));
                assert!(doc.text().lines().all(|line| !line.ends_with([' ', '\t'])));
                if final_newline == Some(false) {
                    assert!(!doc.text().ends_with('\n'));
                }
                if final_newline == Some(true) {
                    assert!(doc.text().ends_with('\n'));
                }
                assert!(!doc.prepare_save(&rules).unwrap());
                doc.undo();
                assert_eq!(doc.text(), text);
            }
        }
    }

    #[test]
    fn config_paths_are_relative_and_precedence_defaults_are_bounded() {
        let configs = [
            ConfigSource {
                directory: "",
                text: "[src/*.rs]\nindent_size=2\n[*.txt]\nindent_size=3",
            },
            ConfigSource {
                directory: "src",
                text: "[/nested/*.rs]\nindent_size=6\n[*.rs]\ntab_width=8",
            },
        ];
        for (path, width, tab_width) in [
            ("src/a.rs", 2, 8),
            ("src/nested/a.rs", 6, 8),
            ("docs/a.txt", 3, 3),
            ("a.rs", 4, 4),
        ] {
            let (rules, warnings) = resolve_rules(path, "", EditorPreferences::default(), &configs);
            assert!(warnings.is_empty());
            assert_eq!(
                (rules.indentation.width, rules.indentation.tab_width),
                (width, tab_width)
            );
        }
        let defaults = EditorPreferences {
            indentation: Indentation {
                style: IndentStyle::Spaces,
                width: usize::MAX,
                tab_width: 0,
            },
            ..Default::default()
        };
        let rules = resolve_rules("a", "", defaults, &[]).0;
        assert_eq!(
            (rules.indentation.width, rules.indentation.tab_width),
            (16, 1)
        );
    }
    #[test]
    fn nested_rules_root_unset_globs_and_separate_tab_width() {
        let sources = [
            ConfigSource {
                directory: "",
                text: "root=true\n[*]\nindent_style=space\nindent_size=8\ninsert_final_newline=true",
            },
            ConfigSource {
                directory: "src",
                text: "root=true\n[*.{rs,ts}]\nindent_style=TAB\nindent_size=4\ntab_width=3\n[foo{1..3}.rs]\nindent_size=unset",
            },
        ];
        let (rules, warnings) =
            resolve_rules("src/foo2.rs", "", EditorPreferences::default(), &sources);
        assert!(warnings.is_empty());
        assert_eq!(rules.indentation.style, IndentStyle::Tabs);
        assert_eq!(rules.indentation.width, 4);
        assert_eq!(rules.indentation.tab_width, 3);
        assert_eq!(rules.final_newline, None);
        assert_eq!(rules.indentation.unit(), "\t ");
    }
    #[test]
    fn detection_precedence_invalid_rules_and_atomic_invalid_file() {
        let text = "fn a() {\n  x();\n    y();\n}";
        let sources = [
            ConfigSource {
                directory: "",
                text: "[*.rs]\nindent_size=3\ninsert_final_newline=false",
            },
            ConfigSource {
                directory: "src",
                text: "[*.rs]\nindent_size=6\nbroken line",
            },
        ];
        let (rules, warnings) =
            resolve_rules("src/a.rs", text, EditorPreferences::default(), &sources);
        assert_eq!(rules.indentation.width, 3);
        assert_eq!(rules.final_newline, Some(false));
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            resolve_rules("a.rs", text, EditorPreferences::default(), &[])
                .0
                .indentation
                .width,
            2
        );
        assert_eq!(
            resolve_rules("a.rs", "\tx\n\t\ty", EditorPreferences::default(), &[])
                .0
                .indentation
                .style,
            IndentStyle::Tabs
        );
    }
    #[test]
    fn conversion_and_save_policies_preserve_unicode_and_undo_as_commands() {
        let mut doc = Document::new("\t 😀  \r\n  x\t");
        doc.convert_indentation(
            Indentation {
                style: IndentStyle::Spaces,
                width: 2,
                tab_width: 2,
            },
            4,
        )
        .unwrap();
        assert_eq!(doc.text(), "     😀  \r\n  x\t");
        let mut rules = resolve_rules("a", doc.text(), EditorPreferences::default(), &[]).0;
        rules.line_ending = Some(LineEnding::Lf);
        rules.trim_trailing_whitespace = true;
        rules.final_newline = Some(true);
        doc.prepare_save(&rules).unwrap();
        assert_eq!(doc.text(), "     😀\n  x\n");
        doc.undo();
        assert_eq!(doc.text(), "     😀  \r\n  x\t");
        doc.undo();
        assert_eq!(doc.text(), "\t 😀  \r\n  x\t");
        let mut empty = Document::new("");
        empty.prepare_save(&rules).unwrap();
        assert_eq!(empty.text(), "");
    }
}
