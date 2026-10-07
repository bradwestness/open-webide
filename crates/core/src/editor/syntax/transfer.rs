//! Validated data crossing the worker boundary; parser allocations never cross it.
use super::{super::structure::StructureData, SyntaxAnalysis};
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
    structure: Option<StructureData>,
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

    fn validate(self, expected: &str, previous: Option<&SyntaxAnalysis>) -> Option<Arc<str>> {
        if expected.len() > MAX_STRUCTURE_BYTES {
            return None;
        }
        match self {
            Self::Full(source) => (source == expected).then(|| Arc::from(source)),
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
                Some(Arc::from(expected))
            }
        }
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum TokenRowData {
    Spans(Vec<(usize, TokenKind)>),
    Previous { reuse: usize },
}
impl TokenRowData {
    fn records(&self) -> usize {
        match self {
            Self::Spans(spans) => spans.len(),
            Self::Previous { .. } => 1,
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
        let structure = self.structure.as_ref().map(|value| value.transfer_data());
        let mut reused = false;
        let highlights = self.highlights.as_ref().map(|lines| {
            lines
                .iter()
                .enumerate()
                .map(|(index, line)| {
                    if let Some(old) =
                        previous.and_then(|(_, analysis)| analysis.highlights.as_ref())
                    {
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
                            return TokenRowData::Previous { reuse };
                        }
                    }
                    let mut end = 0;
                    TokenRowData::Spans(
                        line.iter()
                            .map(|token| {
                                end += token.text.len();
                                (end, token.kind)
                            })
                            .collect(),
                    )
                })
                .collect()
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
                    .map_or(0, StructureData::record_count),
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
            Some(data) => Some(Arc::new(data.validate(source.clone())?)),
            None => None,
        };
        let highlights = if let Some(lines) = self.highlights {
            if lines.len() != line_count {
                return None;
            }
            let mut tokens = Vec::with_capacity(lines.len());
            for (line, data) in source.split('\n').zip(lines) {
                let spans = match data {
                    TokenRowData::Spans(spans) => spans,
                    TokenRowData::Previous { reuse } => {
                        self.base_ticket?;
                        let row = previous?.1.highlights.as_ref()?.get(reuse)?;
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
                        continue;
                    }
                };
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
                .validate(&source, Some(&old))
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
