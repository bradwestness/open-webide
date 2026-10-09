use lepticons::LucideGlyph;
use openwebide_core::git::{GitFileStatus, GitStatusAvailability};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    Added,
    Deleted,
    Modified,
    Renamed,
    Untracked,
    Conflict,
    Copied,
    TypeChanged,
    Unknown,
    Unavailable,
    Clean,
}
impl From<GitFileStatus> for Status {
    fn from(status: GitFileStatus) -> Self {
        match status {
            GitFileStatus::Added => Self::Added,
            GitFileStatus::Deleted => Self::Deleted,
            GitFileStatus::Modified => Self::Modified,
            GitFileStatus::Renamed => Self::Renamed,
            GitFileStatus::Untracked => Self::Untracked,
            GitFileStatus::Conflict => Self::Conflict,
        }
    }
}
impl Status {
    pub fn historical(raw: &str) -> Self {
        match raw {
            "A" => Self::Added,
            "D" => Self::Deleted,
            "M" => Self::Modified,
            "U" => Self::Conflict,
            "T" => Self::TypeChanged,
            value if scored(value, 'R') => Self::Renamed,
            value if scored(value, 'C') => Self::Copied,
            _ => Self::Unknown,
        }
    }
    pub fn presentation(self) -> Presentation {
        let (glyph, class, letter, text) = match self {
            Self::Added => (LucideGlyph::FilePlus, "git-badge-added", "A", "Added"),
            Self::Deleted => (LucideGlyph::FileMinus, "git-badge-deleted", "D", "Deleted"),
            Self::Modified => (LucideGlyph::FileDiff, "git-badge-modified", "M", "Modified"),
            Self::Renamed => (
                LucideGlyph::FilePenLine,
                "git-badge-renamed",
                "R",
                "Renamed",
            ),
            Self::Untracked => (
                LucideGlyph::FileQuestionMark,
                "git-badge-untracked",
                "U",
                "Untracked",
            ),
            Self::Conflict => (
                LucideGlyph::FileExclamationPoint,
                "git-badge-conflict",
                "!",
                "Conflict",
            ),
            Self::Copied => (LucideGlyph::File, "", "C", "Copied"),
            Self::TypeChanged => (
                LucideGlyph::FileDiff,
                "git-badge-modified",
                "T",
                "Type changed",
            ),
            Self::Unknown => (LucideGlyph::File, "", "?", "Unknown Git status"),
            Self::Unavailable => (LucideGlyph::File, "", "?", "Status unavailable"),
            Self::Clean => (LucideGlyph::File, "", "", "Unchanged"),
        };
        Presentation {
            glyph,
            class,
            letter,
            text,
        }
    }
}
fn scored(raw: &str, prefix: char) -> bool {
    raw.strip_prefix(prefix).is_some_and(|score| {
        score.is_empty()
            || (score.len() <= 3
                && score.bytes().all(|byte| byte.is_ascii_digit())
                && score.parse::<u8>().is_ok_and(|score| score <= 100))
    })
}
#[derive(Clone)]
pub struct Presentation {
    pub glyph: LucideGlyph,
    pub class: &'static str,
    pub letter: &'static str,
    pub text: &'static str,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FolderSummary {
    pub counts: BTreeMap<Status, usize>,
    pub unavailable: bool,
}
impl FolderSummary {
    pub fn class(&self) -> &'static str {
        if self.counts.contains_key(&Status::Conflict) {
            "git-badge-conflict"
        } else if self.counts.len() == 1 {
            self.counts.keys().next().unwrap().presentation().class
        } else if self.counts.len() > 1 {
            "git-badge-modified"
        } else {
            ""
        }
    }
    pub fn description(&self) -> String {
        if self.counts.is_empty() {
            return if self.unavailable {
                "Descendant status unavailable"
            } else {
                "Unchanged folder"
            }
            .into();
        }
        let counts = self
            .counts
            .iter()
            .map(|(status, count)| format!("{}: {count}", status.presentation().text))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "Contains {}changes: {counts}{}",
            if self.counts.len() > 1 { "mixed " } else { "" },
            if self.unavailable {
                "; descendant status unavailable"
            } else {
                ""
            }
        )
    }
}
pub fn folders<'a>(
    entries: impl IntoIterator<Item = (&'a str, Status)>,
    availability: GitStatusAvailability,
) -> BTreeMap<String, FolderSummary> {
    let mut summaries = BTreeMap::<String, FolderSummary>::new();
    let mut unique = BTreeMap::<&str, BTreeSet<Status>>::new();
    for (path, status) in entries {
        unique.entry(path).or_default().insert(status);
    }
    for (path, statuses) in unique {
        let status = if statuses.contains(&Status::Conflict) {
            Status::Conflict
        } else if statuses.len() == 1 {
            *statuses.first().unwrap()
        } else {
            Status::Unknown
        };
        for (offset, _) in path.match_indices('/') {
            let folder = summaries.entry(path[..offset].to_owned()).or_default();
            folder.unavailable = availability != GitStatusAvailability::Complete;
            *folder.counts.entry(status).or_default() += 1;
        }
    }
    summaries
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn historical_statuses_preserve_git_meanings() {
        for (raw, status) in [
            ("A", Status::Added),
            ("D", Status::Deleted),
            ("M", Status::Modified),
            ("R100", Status::Renamed),
            ("R075", Status::Renamed),
            ("U", Status::Conflict),
            ("C100", Status::Copied),
            ("T", Status::TypeChanged),
            ("", Status::Unknown),
            ("R101", Status::Unknown),
            ("X", Status::Unknown),
        ] {
            assert_eq!(Status::historical(raw), status);
        }
        for status in [
            GitFileStatus::Added,
            GitFileStatus::Deleted,
            GitFileStatus::Modified,
            GitFileStatus::Renamed,
            GitFileStatus::Untracked,
            GitFileStatus::Conflict,
        ] {
            let presentation = Status::from(status).presentation();
            assert_eq!(presentation.text, status.description());
            assert_eq!(presentation.class, status.css_class());
            assert_eq!(presentation.letter, status.badge());
        }
    }
    #[test]
    fn folders_use_full_paths_deduplicate_and_preserve_all_counts() {
        let entries = [
            ("dir/deep/deleted", Status::Deleted),
            ("dir/added", Status::Added),
            ("dir/added", Status::Added),
            ("directory/other", Status::Conflict),
        ];
        let summary = folders(entries, GitStatusAvailability::Complete);
        assert_eq!(summary["dir"].counts.len(), 2);
        assert_eq!(summary["dir"].counts[&Status::Added], 1);
        assert_eq!(summary["dir/deep"].class(), "git-badge-deleted");
        assert_eq!(summary["dir"].class(), "git-badge-modified");
        assert_eq!(
            summary,
            folders(entries.into_iter().rev(), GitStatusAvailability::Complete)
        );
        assert!(folders(entries, GitStatusAvailability::Passive)["dir"].unavailable);
    }
}

#[cfg(test)]
mod glyph_tests {
    use super::*;
    #[test]
    fn all_status_glyphs_and_duplicate_conflicts_are_deterministic() {
        for (status, glyph) in [
            (Status::Added, LucideGlyph::FilePlus),
            (Status::Deleted, LucideGlyph::FileMinus),
            (Status::Modified, LucideGlyph::FileDiff),
            (Status::Renamed, LucideGlyph::FilePenLine),
            (Status::Untracked, LucideGlyph::FileQuestionMark),
            (Status::Conflict, LucideGlyph::FileExclamationPoint),
            (Status::TypeChanged, LucideGlyph::FileDiff),
            (Status::Copied, LucideGlyph::File),
        ] {
            assert_eq!(status.presentation().glyph, glyph);
        }
        let entries = [
            ("dir/a", Status::Added),
            ("dir/a", Status::Deleted),
            ("dir/conflict", Status::Conflict),
            ("dir/unknown", Status::Unknown),
        ];
        let summary = folders(entries, GitStatusAvailability::Complete);
        assert_eq!(
            summary,
            folders(entries.into_iter().rev(), GitStatusAvailability::Complete)
        );
        assert_eq!(summary["dir"].class(), "git-badge-conflict");
        assert_eq!(summary["dir"].counts[&Status::Unknown], 2);
        assert_eq!(Status::Copied.presentation().class, "");
    }
}
