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
    source: String,
    folds: Vec<FoldRange>,
    structure: Option<StructureData>,
    // Per-line UTF-8 end offsets, avoiding another copy of every token's text.
    highlights: Option<Vec<Vec<(usize, TokenKind)>>>,
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
        if self.record_count() > MAX_ANALYSIS_RECORDS {
            return None;
        }
        let structure = self.structure.as_ref().map(|value| value.transfer_data());
        let highlights = self.highlights.as_ref().map(|lines| {
            lines
                .iter()
                .map(|line| {
                    let mut end = 0;
                    line.iter()
                        .map(|token| {
                            end += token.text.len();
                            (end, token.kind)
                        })
                        .collect()
                })
                .collect()
        });
        let data = SyntaxAnalysisData {
            source: self.source.to_string(),
            folds: self.folds.clone(),
            structure,
            highlights,
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
                lines
                    .iter()
                    .fold(lines.len(), |count, line| count.saturating_add(line.len()))
            }))
    }

    /// Reject wrong sources, invalid coordinates, escaping ranges and excessive data.
    /// This validates a worker result without parsing the document on the UI thread.
    pub fn validate(self, expected_source: &str) -> Option<Arc<SyntaxAnalysis>> {
        if self.source != expected_source
            || self.source.len() > MAX_STRUCTURE_BYTES
            || self.record_count() > MAX_ANALYSIS_RECORDS
        {
            return None;
        }
        let line_count = self.source.split('\n').count();
        if normalize_folds(self.folds.clone(), line_count) != self.folds {
            return None;
        }
        let source: Arc<str> = Arc::from(self.source);
        let structure = match self.structure {
            Some(data) => Some(Arc::new(data.validate(source.clone())?)),
            None => None,
        };
        let highlights = if let Some(lines) = self.highlights {
            if lines.len() != line_count {
                return None;
            }
            let mut tokens = Vec::with_capacity(lines.len());
            for (line, spans) in source.split('\n').zip(lines) {
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
                tokens.push(row);
            }
            Some(Arc::new(tokens))
        } else {
            None
        };
        Some(Arc::new(SyntaxAnalysis {
            source,
            folds: self.folds,
            structure,
            highlights,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::language_from_path;
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
        token.highlights.as_mut().unwrap()[0][0].0 = 1;
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
