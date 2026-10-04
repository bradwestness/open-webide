//! Run file review and hunk selection policy, independent of filesystem hosts.
use crate::{
    EditDecision, FileDiff, RewindFile, RewindPlan,
    diff::{Ending, Line, LineOp, line_lcs},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunChange {
    pub session_id: i64,
    pub message_id: i64,
    pub revision: i64,
    pub source_step: i64,
    pub file: RewindFile,
    pub decisions: Vec<EditDecision>,
    pub conflicted: bool,
    /// Whether this review still describes the project's current file version.
    pub available: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewRequest {
    pub session_id: i64,
    pub message_id: i64,
    pub path: String,
    pub revision: i64,
    pub decision: EditDecision,
    pub hunk: Option<usize>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewPlan {
    pub request: ReviewRequest,
    pub original: RunChange,
    pub reviewed: RunChange,
    pub restore: RewindPlan,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewHunk {
    pub index: usize,
    pub old_line: usize,
    pub new_line: usize,
    pub old: String,
    pub new: String,
    pub decision: EditDecision,
}
#[derive(Clone, Debug)]
enum Part {
    Equal(String),
    Change(ReviewHunk),
}
fn line_text(line: Line<'_>) -> String {
    let ending = match line.ending {
        Ending::Lf => "\n",
        Ending::CrLf => "\r\n",
        Ending::None => "",
    };
    format!("{}{ending}", line.text)
}
fn parts(old: &str, new: &str) -> Vec<Part> {
    let mut result = Vec::new();
    let (mut old_line, mut new_line, mut index) = (1, 1, 0);
    let mut ops = line_lcs(old, new).into_iter().peekable();
    while let Some(op) = ops.next() {
        if let LineOp::Equal(line) = op {
            result.push(Part::Equal(line_text(line)));
            old_line += 1;
            new_line += 1;
            continue;
        }
        let mut hunk = ReviewHunk {
            index,
            old_line,
            new_line,
            old: String::new(),
            new: String::new(),
            decision: EditDecision::Pending,
        };
        let mut next = Some(op);
        while let Some(op) = next {
            match op {
                LineOp::Delete(line) => {
                    hunk.old.push_str(&line_text(line));
                    old_line += 1;
                }
                LineOp::Insert(line) => {
                    hunk.new.push_str(&line_text(line));
                    new_line += 1;
                }
                LineOp::Equal(_) => unreachable!(),
            }
            next = if ops.peek().is_some_and(|op| !matches!(op, LineOp::Equal(_))) {
                ops.next()
            } else {
                None
            };
        }
        result.push(Part::Change(hunk));
        index += 1;
    }
    result
}
impl RunChange {
    pub fn new(session_id: i64, message_id: i64, file: RewindFile) -> Result<Self, String> {
        let mut record = Self {
            session_id,
            message_id,
            revision: 1,
            source_step: 0,
            file,
            decisions: vec![],
            conflicted: false,
            available: true,
        };
        let count = if record.file.before_bytes()? == record.file.after_bytes()? {
            0
        } else {
            record.hunks().len().max(1)
        };
        record.decisions = vec![EditDecision::Pending; count];
        Ok(record)
    }
    pub fn pending(&self) -> usize {
        self.decisions
            .iter()
            .filter(|&&d| d == EditDecision::Pending)
            .count()
    }
    pub fn hunks(&self) -> Vec<ReviewHunk> {
        if self.file.binary_before.is_some()
            || self.file.binary_after.is_some()
            || self.file.backup_path.is_some()
        {
            return vec![];
        }
        if self.file.before.is_none() || self.file.deleted {
            return vec![ReviewHunk {
                index: 0,
                old_line: 1,
                new_line: 1,
                old: self.file.before.clone().unwrap_or_default(),
                new: self.file.after.clone(),
                decision: self
                    .decisions
                    .first()
                    .copied()
                    .unwrap_or(EditDecision::Pending),
            }];
        }
        let old = self.file.before.as_deref().unwrap_or_default();
        let mut hunks: Vec<_> = parts(old, &self.file.after)
            .into_iter()
            .filter_map(|part| match part {
                Part::Equal(_) => None,
                Part::Change(mut h) => {
                    h.decision = self
                        .decisions
                        .get(h.index)
                        .copied()
                        .unwrap_or(EditDecision::Pending);
                    Some(h)
                }
            })
            .collect();
        if hunks.is_empty() && self.file.before.is_none() != self.file.deleted {
            hunks.push(ReviewHunk {
                index: 0,
                old_line: 1,
                new_line: 1,
                old: old.into(),
                new: self.file.after.clone(),
                decision: self
                    .decisions
                    .first()
                    .copied()
                    .unwrap_or(EditDecision::Pending),
            });
        }
        hunks
    }
    fn contents(&self, baseline: bool) -> Result<Option<Vec<u8>>, String> {
        let before = self.file.before_bytes()?;
        let after = self.file.after_bytes()?;
        // File creation/deletion and binary changes are one indivisible change.
        if before.is_none()
            || after.is_none()
            || self.file.binary_before.is_some()
            || self.file.binary_after.is_some()
        {
            let decision = self
                .decisions
                .first()
                .copied()
                .unwrap_or(EditDecision::Accepted);
            return Ok(
                if if baseline {
                    decision == EditDecision::Accepted
                } else {
                    decision != EditDecision::Rejected
                } {
                    after
                } else {
                    before
                },
            );
        }
        let mut content = String::new();
        for part in parts(
            self.file.before.as_deref().unwrap_or_default(),
            &self.file.after,
        ) {
            match part {
                Part::Equal(text) => content.push_str(&text),
                Part::Change(hunk) => {
                    let decision = self
                        .decisions
                        .get(hunk.index)
                        .copied()
                        .unwrap_or(EditDecision::Pending);
                    let new = if baseline {
                        decision == EditDecision::Accepted
                    } else {
                        decision != EditDecision::Rejected
                    };
                    content.push_str(if new { &hunk.new } else { &hunk.old });
                }
            }
        }
        Ok(Some(content.into_bytes()))
    }
    pub fn current_bytes(&self) -> Result<Option<Vec<u8>>, String> {
        self.contents(false)
    }
    pub fn pending_file(&self) -> Result<RewindFile, String> {
        Ok(RewindFile::from_bytes(
            self.file.path.clone(),
            self.contents(true)?,
            self.contents(false)?,
        ))
    }
    pub fn coalesce(&mut self, file: RewindFile) -> Result<(), String> {
        self.conflicted |= self.current_bytes()? != file.before_bytes()?;
        if self
            .decisions
            .iter()
            .any(|&decision| decision != EditDecision::Pending)
        {
            let baseline = self.pending_file()?;
            self.file.before = baseline.before;
            self.file.binary_before = baseline.binary_before;
        }
        self.file.after = file.after;
        self.file.binary_after = file.binary_after;
        self.file.deleted = file.deleted;
        self.revision += 1;
        self.decisions.clear();
        let count = if self.file.before_bytes()? == self.file.after_bytes()? {
            0
        } else {
            self.hunks().len().max(1)
        };
        self.decisions = vec![EditDecision::Pending; count];
        Ok(())
    }
    pub fn prepare(&self, request: ReviewRequest) -> Result<ReviewPlan, String> {
        if request.session_id != self.session_id
            || request.message_id != self.message_id
            || request.path != self.file.path
            || request.revision != self.revision
        {
            return Err("Review revision changed; refresh the changes panel".into());
        }
        if !self.available || self.conflicted {
            return Err("This file changed again; review its current changes instead".into());
        }
        if request.decision == EditDecision::Pending {
            return Err("Choose Accept or Reject".into());
        }
        let mut reviewed = self.clone();
        if let Some(index) = request.hunk {
            let decision = reviewed
                .decisions
                .get_mut(index)
                .ok_or("Hunk no longer exists")?;
            if *decision != EditDecision::Pending {
                return Err("Hunk has already been reviewed".into());
            }
            *decision = request.decision;
        } else {
            if reviewed.pending() == 0 {
                return Err("File has already been reviewed".into());
            }
            for decision in &mut reviewed.decisions {
                if *decision == EditDecision::Pending {
                    *decision = request.decision;
                }
            }
        }
        reviewed.revision += 1;
        let file = RewindFile::from_bytes(
            self.file.path.clone(),
            reviewed.current_bytes()?,
            self.current_bytes()?,
        );
        Ok(ReviewPlan {
            restore: RewindPlan {
                skipped: Default::default(),
                message_id: self.message_id,
                prompt: String::new(),
                files: vec![file],
            },
            request,
            original: self.clone(),
            reviewed,
        })
    }
}
impl RewindFile {
    pub fn from_bytes(path: String, before: Option<Vec<u8>>, after: Option<Vec<u8>>) -> Self {
        use base64::{Engine, engine::general_purpose::STANDARD};
        fn fields(bytes: Option<Vec<u8>>) -> (Option<String>, Option<String>) {
            match bytes {
                None => (None, None),
                Some(bytes) => match String::from_utf8(bytes) {
                    Ok(text) => (Some(text), None),
                    Err(error) => (None, Some(STANDARD.encode(error.into_bytes()))),
                },
            }
        }
        let deleted = after.is_none();
        let (before, binary_before) = fields(before);
        let (after, binary_after) = fields(after);
        Self {
            path,
            before,
            after: after.unwrap_or_default(),
            binary_before,
            binary_after,
            deleted,
            backup_path: None,
        }
    }
    pub fn preview(&self) -> FileDiff {
        FileDiff {
            path: self.path.clone(),
            old: self.before.clone(),
            new: self.after.clone(),
            old_unavailable: self.binary_before.is_some(),
            backup_path: self.backup_path.clone(),
        }
    }
}

/// Pending line markers are in the new file's coordinates; deletions attach to
/// the following line (or the final line), so removed lines remain discoverable.
pub fn pending_lines(diff: &FileDiff) -> Vec<(usize, bool)> {
    let mut lines = Vec::new();
    for part in parts(diff.old.as_deref().unwrap_or_default(), &diff.new) {
        if let Part::Change(hunk) = part {
            let count = crate::diff::split_lines(&hunk.new).len();
            if count == 0 {
                lines.push((hunk.new_line, true));
            } else {
                lines.extend((hunk.new_line..hunk.new_line + count).map(|line| (line, false)));
            }
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_hunks_keep_line_endings_and_accept_reject_choices() {
        let file = RewindFile::from_bytes(
            "file.rs".into(),
            Some(b"one\r\nkeep\nthree".to_vec()),
            Some(b"ONE\r\nkeep\nTHREE\n".to_vec()),
        );
        let record = RunChange::new(1, 2, file).unwrap();
        assert_eq!(record.hunks().len(), 2);
        let request = |revision, hunk, decision| ReviewRequest {
            session_id: 1,
            message_id: 2,
            path: "file.rs".into(),
            revision,
            hunk,
            decision,
        };
        let accepted = record
            .prepare(request(1, Some(0), EditDecision::Accepted))
            .unwrap()
            .reviewed;
        assert_eq!(
            accepted.current_bytes().unwrap().unwrap(),
            b"ONE\r\nkeep\nTHREE\n"
        );
        assert_eq!(
            accepted.pending_file().unwrap().before.unwrap(),
            "ONE\r\nkeep\nthree"
        );
        let rejected = accepted
            .prepare(request(2, Some(1), EditDecision::Rejected))
            .unwrap()
            .reviewed;
        assert_eq!(
            rejected.current_bytes().unwrap().unwrap(),
            b"ONE\r\nkeep\nthree"
        );
        assert_eq!(rejected.pending(), 0);
        assert!(
            accepted
                .prepare(request(1, Some(1), EditDecision::Rejected))
                .is_err()
        );
        assert!(
            rejected
                .prepare(request(3, Some(1), EditDecision::Accepted))
                .is_err()
        );
    }
    #[test]
    fn binary_creation_and_deletion_are_reversible_without_text_conversion() {
        for (before, after) in [
            (Some(vec![0, 255]), Some(vec![0, 254])),
            (None, Some(vec![])),
            (Some(b"gone".to_vec()), None),
        ] {
            let record = RunChange::new(
                1,
                2,
                RewindFile::from_bytes("file".into(), before.clone(), after),
            )
            .unwrap();
            assert_eq!(record.pending(), 1);
            let reviewed = record
                .prepare(ReviewRequest {
                    session_id: 1,
                    message_id: 2,
                    path: "file".into(),
                    revision: 1,
                    decision: EditDecision::Rejected,
                    hunk: None,
                })
                .unwrap()
                .reviewed;
            assert_eq!(reviewed.current_bytes().unwrap(), before);
        }
    }
    #[test]
    fn further_edits_keep_accepted_contents_as_the_review_baseline() {
        let mut record = RunChange::new(
            1,
            2,
            RewindFile::from_bytes(
                "file".into(),
                Some(b"a\nkeep\nb".to_vec()),
                Some(b"A\nkeep\nB".to_vec()),
            ),
        )
        .unwrap()
        .prepare(ReviewRequest {
            session_id: 1,
            message_id: 2,
            path: "file".into(),
            revision: 1,
            decision: EditDecision::Accepted,
            hunk: Some(0),
        })
        .unwrap()
        .reviewed;
        record
            .coalesce(RewindFile::from_bytes(
                "file".into(),
                Some(b"A\nkeep\nB".to_vec()),
                Some(b"A\nkeep\nC".to_vec()),
            ))
            .unwrap();
        assert_eq!(record.pending(), 1);
        assert_eq!(
            record.pending_file().unwrap().before.as_deref(),
            Some("A\nkeep\nb")
        );
        assert!(!record.conflicted);
        let created = RunChange::new(
            1,
            2,
            RewindFile::from_bytes("new".into(), None, Some(b"a\nb\nc".to_vec())),
        )
        .unwrap();
        assert_eq!(created.pending(), 1);
        assert_eq!(created.hunks().len(), 1);
    }
}
