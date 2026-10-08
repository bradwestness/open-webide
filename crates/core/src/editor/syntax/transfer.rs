//! Validated data crossing the worker boundary; parser allocations never cross it.
use super::{super::structure::StructurePublication, SyntaxAnalysis};
use crate::{
    editor::*,
    highlight::{Token, TokenKind},
};
use std::sync::Arc;

pub const MAX_ANALYSIS_MESSAGE_BYTES: usize = 32 * 1024 * 1024;
pub(super) const MAX_ANALYSIS_RECORDS: usize = 100_000;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyntaxAnalysisData {
    source: SyntaxSource,
    folds: Vec<FoldRange>,
    structure: Option<StructurePublication>,
    // Per-line UTF-8 end offsets, avoiding another copy of every token's text.
    highlights: Option<Vec<TokenRowData>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    base_ticket: Option<u32>,
}

/// Source coordinates are UTF-8 bytes, never browser UTF-16 positions.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum SyntaxSource {
    Full(String),
    Replace {
        start: usize,
        end: usize,
        text: String,
    },
}

impl SyntaxSource {
    pub fn publication(source: &str, previous: Option<&str>) -> Self {
        if let Some(previous) = previous {
            let change = text_change(previous, source);
            let (start, end, text) = change.map_or((0, 0, ""), |change| {
                (
                    change.range.start,
                    change.range.end,
                    &source[change.range.start..change.new_end],
                )
            });
            // Reserve envelope overhead; full replacements remain standalone.
            if text.len().saturating_add(96) < source.len() {
                return Self::Replace {
                    start,
                    end,
                    text: text.into(),
                };
            }
        }
        Self::Full(source.into())
    }

    pub(super) fn result_length(&self, previous: Option<&str>) -> Option<usize> {
        match self {
            Self::Full(source) => Some(source.len()),
            Self::Replace { start, end, text } => {
                let old = previous?;
                if start > end {
                    return None;
                }
                old.get(..*start)?
                    .len()
                    .checked_add(text.len())?
                    .checked_add(old.get(*end..)?.len())
            }
        }
    }

    /// Resolve a validated worker request; publication policy bounds size first.
    pub fn resolve(self, previous: Option<&str>) -> Option<String> {
        let length = self.result_length(previous)?;
        if length > MAX_STRUCTURE_BYTES {
            return None;
        }
        match self {
            Self::Full(source) => Some(source),
            Self::Replace { start, end, text } => {
                let old = previous?;
                let mut source = String::with_capacity(length);
                source.push_str(old.get(..start)?);
                source.push_str(&text);
                source.push_str(old.get(end..)?);
                Some(source)
            }
        }
    }

    fn validate(
        self,
        expected: Arc<String>,
        previous: Option<&SyntaxAnalysis>,
    ) -> Option<Arc<String>> {
        if expected.len() > MAX_STRUCTURE_BYTES {
            return None;
        }
        match self {
            Self::Full(source) => (source == expected.as_str()).then_some(expected),
            Self::Replace { start, end, text } => {
                let old = previous?.source();
                if start > end {
                    return None;
                }
                let prefix = old.get(..start)?;
                let suffix = old.get(end..)?;
                let new_end = start.checked_add(text.len())?;
                let length = new_end.checked_add(suffix.len())?;
                if length != expected.len()
                    || expected.get(..start)? != prefix
                    || expected.get(start..new_end)? != text
                    || expected.get(new_end..)? != suffix
                {
                    return None;
                }
                Some(expected)
            }
        }
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum TokenRowData {
    Spans(Vec<(usize, TokenKind)>),
    Previous { reuse: usize, count: usize },
}
impl TokenRowData {
    fn row_count(&self) -> usize {
        match self {
            Self::Spans(_) => 1,
            Self::Previous { count, .. } => *count,
        }
    }
    fn records(&self) -> usize {
        match self {
            Self::Spans(spans) => spans.len(),
            Self::Previous { count, .. } => *count,
        }
    }
}

impl SyntaxAnalysis {
    pub(super) fn record_count(&self) -> usize {
        self.folds
            .len()
            .saturating_add(
                self.structure
                    .as_ref()
                    .map_or(0, |value| value.record_count()),
            )
            .saturating_add(self.highlights.as_ref().map_or(0, |lines| {
                lines
                    .iter()
                    .fold(lines.len(), |count, line| count.saturating_add(line.len()))
            }))
    }
    pub fn transfer_data(&self) -> Option<SyntaxAnalysisData> {
        self.transfer_data_reusing(None)
    }
    pub(super) fn transfer_data_reusing(
        &self,
        previous: Option<(u32, &SyntaxAnalysis)>,
    ) -> Option<SyntaxAnalysisData> {
        if self.record_count() > MAX_ANALYSIS_RECORDS {
            return None;
        }
        let structure = self.structure.as_ref().map(|value| {
            StructurePublication::publication(
                value,
                previous.and_then(|(_, analysis)| analysis.structure.as_deref()),
            )
        });
        let mut reused = structure
            .as_ref()
            .is_some_and(StructurePublication::needs_base);
        let highlights = self.highlights.as_ref().map(|lines| {
            let mut data = Vec::new();
            for (index, line) in lines.iter().enumerate() {
                if let Some(old) = previous.and_then(|(_, analysis)| analysis.highlights.as_ref()) {
                    let shifted = if lines.len() >= old.len() {
                        index.checked_sub(lines.len() - old.len())
                    } else {
                        index.checked_add(old.len() - lines.len())
                    };
                    if let Some(reuse) = [Some(index), shifted]
                        .into_iter()
                        .flatten()
                        .find(|&candidate| old.get(candidate).is_some_and(|row| row == line))
                    {
                        reused = true;
                        match data.last_mut() {
                            Some(TokenRowData::Previous {
                                reuse: start,
                                count,
                            }) if start.checked_add(*count) == Some(reuse) => {
                                *count += 1;
                            }
                            _ => data.push(TokenRowData::Previous { reuse, count: 1 }),
                        }
                        continue;
                    }
                }
                let mut end = 0;
                data.push(TokenRowData::Spans(
                    line.iter()
                        .map(|token| {
                            end += token.text.len();
                            (end, token.kind)
                        })
                        .collect(),
                ));
            }
            data
        });
        let source = SyntaxSource::publication(
            &self.source,
            previous.map(|(_, analysis)| analysis.source()),
        );
        reused |= matches!(source, SyntaxSource::Replace { .. });
        let data = SyntaxAnalysisData {
            source,
            folds: self.folds.clone(),
            structure,
            highlights,
            base_ticket: reused.then(|| previous.expect("reused previous publication").0),
        };
        (data.record_count() <= MAX_ANALYSIS_RECORDS).then_some(data)
    }
}

impl SyntaxAnalysisData {
    fn record_count(&self) -> usize {
        self.folds
            .len()
            .saturating_add(
                self.structure
                    .as_ref()
                    .map_or(0, StructurePublication::record_count),
            )
            .saturating_add(self.highlights.as_ref().map_or(0, |lines| {
                lines.iter().fold(lines.len(), |count, line| {
                    count.saturating_add(line.records())
                })
            }))
    }

    /// Reject wrong sources, invalid coordinates, escaping ranges and excessive data.
    /// This validates a worker result without parsing the document on the UI thread.
    pub fn validate(self, expected_source: &str) -> Option<Arc<SyntaxAnalysis>> {
        self.validate_reusing(expected_source, None)
    }
    pub(super) fn validate_reusing(
        self,
        expected_source: &str,
        previous: Option<(u32, &SyntaxAnalysis)>,
    ) -> Option<Arc<SyntaxAnalysis>> {
        if expected_source.len() > MAX_STRUCTURE_BYTES {
            return None;
        }
        self.validate_shared(Arc::new(expected_source.to_owned()), previous)
    }
    pub(super) fn validate_shared(
        self,
        expected_source: Arc<String>,
        previous: Option<(u32, &SyntaxAnalysis)>,
    ) -> Option<Arc<SyntaxAnalysis>> {
        if self
            .base_ticket
            .is_some_and(|ticket| previous.is_none_or(|(base, _)| base != ticket))
        {
            return None;
        }
        if self.record_count() > MAX_ANALYSIS_RECORDS {
            return None;
        }
        if matches!(self.source, SyntaxSource::Replace { .. }) && self.base_ticket.is_none() {
            return None;
        }
        let source = self
            .source
            .validate(expected_source, previous.map(|(_, analysis)| analysis))?;
        let line_count = source.split('\n').count();
        if normalize_folds(self.folds.clone(), line_count) != self.folds {
            return None;
        }
        let structure = match self.structure {
            Some(data) => {
                if data.needs_base() && self.base_ticket.is_none() {
                    return None;
                }
                let budget = MAX_ANALYSIS_RECORDS.checked_sub(self.folds.len())?;
                Some(Arc::new(data.validate(
                    source.clone(),
                    previous.and_then(|(_, old)| old.structure.as_deref()),
                    budget,
                )?))
            }
            None => None,
        };
        let highlights = if let Some(lines) = self.highlights {
            let declared_rows = lines
                .iter()
                .try_fold(0_usize, |count, line| count.checked_add(line.row_count()))?;
            if declared_rows != line_count || declared_rows > MAX_ANALYSIS_RECORDS {
                return None;
            }
            let mut records = self
                .folds
                .len()
                .saturating_add(structure.as_ref().map_or(0, |value| value.record_count()));
            let mut raw_lines = source.split('\n');
            let mut tokens = Vec::with_capacity(line_count);
            for data in lines {
                match data {
                    TokenRowData::Previous { reuse, count } => {
                        self.base_ticket?;
                        if count == 0 {
                            return None;
                        }
                        let end = reuse.checked_add(count)?;
                        let rows = previous?.1.highlights.as_ref()?.get(reuse..end)?;
                        for row in rows {
                            records = records.saturating_add(row.len()).saturating_add(1);
                            if records > MAX_ANALYSIS_RECORDS {
                                return None;
                            }
                            let line = raw_lines.next()?;
                            let mut start = 0_usize;
                            for token in row.iter() {
                                let end = start.checked_add(token.text.len())?;
                                if line.get(start..end) != Some(token.text.as_str()) {
                                    return None;
                                }
                                start = end;
                            }
                            if start != line.len() {
                                return None;
                            }
                            tokens.push(row.clone());
                        }
                    }
                    TokenRowData::Spans(spans) => {
                        records = records.saturating_add(spans.len()).saturating_add(1);
                        if records > MAX_ANALYSIS_RECORDS {
                            return None;
                        }
                        let line = raw_lines.next()?;
                        let mut start = 0;
                        let mut row = Vec::with_capacity(spans.len());
                        for (end, kind) in spans {
                            if end < start
                                || (end == start && !line.is_empty())
                                || !line.is_char_boundary(end)
                            {
                                return None;
                            }
                            row.push(Token {
                                kind,
                                text: line[start..end].to_string(),
                            });
                            start = end;
                        }
                        if start != line.len() {
                            return None;
                        }
                        tokens.push(Arc::from(row));
                    }
                }
            }
            if raw_lines.next().is_some() {
                return None;
            }
            Some(Arc::new(tokens))
        } else {
            None
        };
        let analysis = Arc::new(SyntaxAnalysis {
            source,
            folds: self.folds,
            structure,
            highlights,
        });
        (analysis.record_count() <= MAX_ANALYSIS_RECORDS).then_some(analysis)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::language_from_path;
    #[test]
    fn structural_patches_reconstruct_edits_and_reject_invalid_bases_ranges_and_budgets() {
        let source: String = (0..100)
            .map(|index| format!("fn f{index}() {{ let s = \"文😀\"; }}\r\n"))
            .collect();
        let mut document = SyntaxDocument::new(crate::highlight::Language::Rust).unwrap();
        let old = document.prepare(&source, 4, || true).1.unwrap();
        for revised in [
            source.clone(),
            source.replace("f50()", "g50()"),
            source.replace("f50()", "longer50()"),
            source.replacen("fn f50()", "// 文😀\r\nfn f50()", 1),
            source.replace("fn f50() { let s = \"文😀\"; }\r\n", ""),
        ] {
            let next = document.prepare(&revised, 4, || true).1.unwrap();
            let wire = next.transfer_data_reusing(Some((42, &old))).unwrap();
            assert!(wire.structure.as_ref().unwrap().needs_base());
            let restored = wire
                .clone()
                .validate_reusing(&revised, Some((42, &old)))
                .unwrap();
            assert_eq!(
                serde_json::to_value(restored.structure().unwrap().transfer_data()).unwrap(),
                serde_json::to_value(next.structure().unwrap().transfer_data()).unwrap(),
            );
            assert!(wire.clone().validate_reusing(&revised, None).is_none());
            assert!(
                wire.clone()
                    .validate_reusing(&revised, Some((43, &old)))
                    .is_none()
            );
            let mut invalid = wire.clone();
            invalid.base_ticket = None;
            invalid.source = SyntaxSource::Full(revised.clone());
            invalid.highlights = None;
            assert!(
                invalid
                    .validate_reusing(&revised, Some((42, &old)))
                    .is_none()
            );
            assert!(
                wire.structure
                    .clone()
                    .unwrap()
                    .validate(
                        Arc::new(revised.clone()),
                        old.structure().map(AsRef::as_ref),
                        1,
                    )
                    .is_none()
            );
            let mut wrong_language = serde_json::to_value(&wire).unwrap();
            wrong_language["structure"]["changes"]["language"] = serde_json::json!("Python");
            let invalid: SyntaxAnalysisData = serde_json::from_value(wrong_language).unwrap();
            assert!(
                invalid
                    .validate_reusing(&revised, Some((42, &old)))
                    .is_none()
            );
            let mut wrong_coordinate = serde_json::to_value(&wire).unwrap();
            wrong_coordinate["structure"]["changes"]["brackets"] =
                serde_json::json!({"start":0,"end":1,"items":[[usize::MAX,"{",null]]});
            let invalid: SyntaxAnalysisData = serde_json::from_value(wrong_coordinate).unwrap();
            assert!(
                invalid
                    .validate_reusing(&revised, Some((42, &old)))
                    .is_none()
            );
            for (start, end) in [(usize::MAX, usize::MAX), (1, 0), (0, usize::MAX)] {
                let mut value = serde_json::to_value(&wire).unwrap();
                value["structure"]["changes"]["brackets"] =
                    serde_json::json!({"start":start,"end":end,"items":[]});
                let invalid: SyntaxAnalysisData = serde_json::from_value(value).unwrap();
                assert!(
                    invalid
                        .validate_reusing(&revised, Some((42, &old)))
                        .is_none()
                );
            }
            let mut value = serde_json::to_value(&wire).unwrap();
            value["structure"]["changes"]["brackets"] =
                serde_json::json!({"start":0,"end":0,"items":[],"extra":true});
            assert!(serde_json::from_value::<SyntaxAnalysisData>(value).is_err());
        }
        let next = document.prepare(&source, 4, || true).1.unwrap();
        let full_bytes = serde_json::to_vec(&next.transfer_data().unwrap())
            .unwrap()
            .len();
        let patch_bytes =
            serde_json::to_vec(&next.transfer_data_reusing(Some((42, &old))).unwrap())
                .unwrap()
                .len();
        assert!(
            patch_bytes * 10 < full_bytes,
            "{patch_bytes} versus {full_bytes}"
        );
        let mut python = SyntaxDocument::new(crate::highlight::Language::Python).unwrap();
        let other = python
            .prepare("def f():\n    return 1\n", 4, || true)
            .1
            .unwrap();
        let wire = other.transfer_data_reusing(Some((42, &old))).unwrap();
        assert!(!wire.structure.as_ref().unwrap().needs_base());
        assert!(
            wire.validate_reusing(other.source(), Some((42, &old)))
                .is_some()
        );
    }

    #[test]
    fn token_row_runs_preserve_insertions_deletions_disjoint_edits_and_source_ownership() {
        let source: String = (0..1000)
            .map(|index| format!("SELECT {index}, '文😀';\r\n"))
            .collect();
        let mut document = SyntaxDocument::new(crate::highlight::Language::Sql).unwrap();
        let old = document.prepare(&source, 4, || true).1.unwrap();
        let row = "SELECT 500, '文😀';\r\n";
        for (revised, max_runs) in [
            (source.clone(), 1),
            (source.replacen(row, "SELECT changed, '🦀';\r\n", 1), 3),
            (
                source.replacen(row, &format!("SELECT inserted, '🦀';\r\n{row}"), 1),
                3,
            ),
            (source.replacen(row, "", 1), 2),
            (
                source
                    .replace("SELECT 100,", "SELECT first,")
                    .replace("SELECT 900,", "SELECT last,"),
                5,
            ),
        ] {
            let next = document.prepare(&revised, 4, || true).1.unwrap();
            let wire = next.transfer_data_reusing(Some((42, &old))).unwrap();
            assert!(wire.highlights.as_ref().unwrap().len() <= max_runs);
            let snapshot: Arc<String> = Arc::new(revised.to_owned());
            let restored = wire
                .validate_shared(snapshot.clone(), Some((42, &old)))
                .unwrap();
            assert!(Arc::ptr_eq(&snapshot, restored.source_snapshot()));
            assert_eq!(restored.highlights(), next.highlights());
        }
        let full = old.transfer_data().unwrap();
        let snapshot: Arc<String> = Arc::new(source.to_owned());
        let restored = full.validate_shared(snapshot.clone(), None).unwrap();
        assert!(Arc::ptr_eq(&snapshot, restored.source_snapshot()));
        let wire = old.transfer_data_reusing(Some((42, &old))).unwrap();
        for (reuse, count) in [
            (0, 0),
            (0, 1000),
            (0, 1002),
            (1, 1001),
            (usize::MAX, 2),
            (0, usize::MAX),
        ] {
            let mut invalid = wire.clone();
            invalid.highlights = Some(vec![TokenRowData::Previous { reuse, count }]);
            assert!(
                invalid
                    .validate_shared(snapshot.clone(), Some((42, &old)))
                    .is_none()
            );
        }
        for malformed in [
            serde_json::json!({"reuse":0}),
            serde_json::json!({"reuse":0,"count":1,"extra":true}),
        ] {
            assert!(serde_json::from_value::<TokenRowData>(malformed).is_err());
        }
    }

    #[test]
    fn reconstructed_row_runs_cannot_multiply_the_token_record_budget() {
        let row: Arc<[Token]> = Arc::from(vec![
            Token {
                kind: TokenKind::Plain,
                text: String::new()
            };
            MAX_ANALYSIS_RECORDS / 2
        ]);
        let old = SyntaxAnalysis {
            source: Arc::new(String::new()),
            folds: vec![],
            structure: None,
            highlights: Some(Arc::new(vec![row])),
        };
        let mut wire = old.transfer_data().unwrap();
        wire.source = SyntaxSource::Full("\n\n".into());
        wire.base_ticket = Some(42);
        wire.highlights = Some(vec![TokenRowData::Previous { reuse: 0, count: 3 }]);
        assert!(wire.validate_reusing("\n\n", Some((42, &old))).is_none());
    }

    #[test]
    fn source_publications_validate_unicode_edits_and_reject_invalid_bases_and_ranges() {
        let source = format!("{}文😀\r\n{}", "prefix ".repeat(30), "suffix ".repeat(30));
        let mut document = SyntaxDocument::new(crate::highlight::Language::Plain).unwrap();
        let old = document.prepare(&source, 4, || true).1.unwrap();
        for revised in [
            source.clone(),
            source.replace("文😀", "🦀 new"),
            source.replace("文😀", ""),
            source.replace("文😀", "文😀 inserted\r\n"),
            format!("{source}🦀"),
            format!("🦀{source}"),
            source
                .replace("prefix ", "prefix changed ")
                .replace("suffix ", "tail "),
        ] {
            let next = document.prepare(&revised, 4, || true).1.unwrap();
            let wire = next.transfer_data_reusing(Some((42, &old))).unwrap();
            let restored = wire
                .clone()
                .validate_reusing(&revised, Some((42, &old)))
                .unwrap();
            assert_eq!(restored.source(), revised);
            assert_eq!(restored.highlights(), next.highlights());
            if matches!(wire.source, SyntaxSource::Replace { .. }) {
                assert!(wire.clone().validate(&revised).is_none());
                assert!(
                    wire.clone()
                        .validate_reusing(&revised, Some((43, &old)))
                        .is_none()
                );
                assert!(
                    wire.validate_reusing(&(revised.clone() + "x"), Some((42, &old)))
                        .is_none()
                );
            }
        }
        let start = source.find('文').unwrap();
        for (start, end, text) in [
            (start + 1, start + 1, ""), // Inside a UTF-8 scalar.
            (start + 3, start, ""),     // Reversed range.
            (0, usize::MAX, ""),
            (0, 0, "wrong"),
        ] {
            assert!(
                SyntaxSource::Replace {
                    start,
                    end,
                    text: text.into()
                }
                .validate(Arc::new(source.clone()), Some(&old))
                .is_none()
            );
        }
        let mut wire = old.transfer_data_reusing(Some((42, &old))).unwrap();
        assert!(matches!(wire.source, SyntaxSource::Replace { .. }));
        wire.base_ticket = None;
        assert!(wire.validate_reusing(&source, Some((42, &old))).is_none());
        let malformed = serde_json::json!({"start":0,"end":0,"text":"","extra":true});
        assert!(serde_json::from_value::<SyntaxSource>(malformed).is_err());
    }

    #[test]
    fn round_trip_all_languages_and_literals_preserves_every_consumer() {
        let cases = crate::editor::syntax_contracts::LANGUAGE_CASES
            .iter()
            .map(|&(path, source, _)| (path, source))
            .chain(
                crate::editor::syntax_contracts::LITERAL_CASES
                    .iter()
                    .map(|case| (case.0, case.1)),
            )
            .chain([
                ("empty.rs", ""),
                ("plain.txt", "文😀\r\n"),
                ("empty.html", "<script></script>"),
            ]);
        for (path, source) in cases {
            let mut document = SyntaxDocument::new(language_from_path(path)).unwrap();
            let (_, original) = document.prepare(source, 4, || true);
            let original = original.unwrap();
            let json = serde_json::to_string(&original.transfer_data().unwrap()).unwrap();
            let restored: SyntaxAnalysisData = serde_json::from_str(&json).unwrap();
            let restored = restored
                .validate(source)
                .unwrap_or_else(|| panic!("{path}: {json}"));
            assert_eq!(original.folds(), restored.folds(), "{path}");
            assert_eq!(original.highlights(), restored.highlights(), "{path}");
            if let Some(context) = original.structure() {
                let next = restored.structure().unwrap();
                assert_eq!(context.protected, next.protected, "{path}");
                assert_eq!(context.brackets, next.brackets, "{path}");
                assert_eq!(context.scopes, next.scopes, "{path}");
                for index in source.char_indices().map(|(index, _)| index) {
                    assert_eq!(context.language_at(index), next.language_at(index));
                }
            }
        }
    }

    #[test]
    fn invalid_sources_utf8_ranges_folds_tokens_and_bracket_links_are_rejected() {
        let source = "😀 fn main() {\r\n call(\"文\");\r\n}\r\n";
        let mut document = SyntaxDocument::new(crate::highlight::Language::Rust).unwrap();
        let (_, prepared) = document.prepare(source, 4, || true);
        let wire = prepared.unwrap().transfer_data().unwrap();
        assert!(
            wire.clone()
                .validate(&source.replace("main", "xxxx"))
                .is_none()
        );
        let mut token = wire.clone();
        let TokenRowData::Spans(spans) = &mut token.highlights.as_mut().unwrap()[0] else {
            unreachable!()
        };
        spans[0].0 = 1;
        assert!(token.validate(source).is_none());
        let mut folds = wire.clone();
        folds.folds.push(FoldRange {
            start_line: 0,
            end_line: usize::MAX,
        });
        assert!(folds.validate(source).is_none());
        let mut value = serde_json::to_value(&wire).unwrap();
        value["structure"]["brackets"][0][2] = serde_json::json!(usize::MAX);
        assert!(
            serde_json::from_value::<SyntaxAnalysisData>(value)
                .unwrap()
                .validate(source)
                .is_none()
        );
        let mut value = serde_json::to_value(&wire).unwrap();
        value["structure"]["selections"][0]["end"] = serde_json::json!(source.len() + 1);
        assert!(
            serde_json::from_value::<SyntaxAnalysisData>(value)
                .unwrap()
                .validate(source)
                .is_none()
        );
        let mut many = wire;
        many.folds = vec![
            FoldRange {
                start_line: 0,
                end_line: 1
            };
            MAX_ANALYSIS_RECORDS + 1
        ];
        assert!(many.validate(source).is_none());
    }
}
