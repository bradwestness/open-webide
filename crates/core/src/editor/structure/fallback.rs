//! Retained lexical contexts and bounded collection into document coordinates.
use super::{RegionKind, Structure};
use crate::highlight::Language;
use std::{ops::Range, sync::Arc};

/// Relative contexts retain no source copy or unused bracket table.
#[derive(Default)]
pub(in crate::editor) struct FallbackContexts {
    pub(in crate::editor) opaque_starts: Vec<usize>,
    pub(in crate::editor) protected: Vec<(Range<usize>, bool, RegionKind)>,
}
impl FallbackContexts {
    pub(in crate::editor) fn scan(text: &str, language: Language) -> Self {
        Self::from_lexical(Structure::scan(text, language).unwrap_or_default())
    }
    pub(in crate::editor) fn from_lexical(lexical: super::LexicalStructure) -> Self {
        Self {
            opaque_starts: lexical.opaque_starts,
            protected: lexical.protected,
        }
    }
}

/// Injection selection guarantees ordered, disjoint bodies and monotonic ends.
pub(in crate::editor) fn embedded_scope_after(
    scopes: &[(Range<usize>, Language)],
    position: usize,
) -> Option<&Range<usize>> {
    let index = scopes.partition_point(|(body, _)| body.end <= position);
    scopes.get(index).map(|(body, _)| body)
}
pub(in crate::editor) fn overlaps(left: &Range<usize>, right: &Range<usize>) -> bool {
    left.start < right.end && right.start < left.end
}

pub(super) struct FallbackCollection {
    outer: Arc<FallbackContexts>,
    embedded: Vec<(usize, Arc<FallbackContexts>)>,
    body: usize,
    index: usize,
    phase: Phase,
}
enum Phase {
    OuterProtected,
    OuterOpaque,
    EmbeddedProtected,
    EmbeddedOpaque,
    Complete,
}
impl FallbackCollection {
    pub fn new(
        outer: Arc<FallbackContexts>,
        embedded: Vec<(usize, Arc<FallbackContexts>)>,
    ) -> Self {
        Self {
            outer,
            embedded,
            body: 0,
            index: 0,
            phase: Phase::OuterProtected,
        }
    }
    fn next(&mut self, phase: Phase) {
        self.phase = phase;
        self.index = 0;
    }

    /// Copy/filter/shift one record per unit; scope lookup is logarithmic.
    pub fn advance(&mut self, structure: &mut Structure, budget: usize) -> bool {
        for _ in 0..budget {
            match self.phase {
                Phase::OuterProtected => {
                    if let Some((range, closed, kind)) = self.outer.protected.get(self.index) {
                        if !embedded_scope_after(&structure.scopes, range.start)
                            .is_some_and(|body| overlaps(range, body))
                        {
                            structure.protected.push((range.clone(), *closed, *kind));
                        }
                        self.index += 1;
                    } else {
                        self.next(Phase::OuterOpaque);
                    }
                }
                Phase::OuterOpaque => {
                    if let Some(position) = self.outer.opaque_starts.get(self.index) {
                        if !embedded_scope_after(&structure.scopes, *position)
                            .is_some_and(|body| body.contains(position))
                        {
                            structure.opaque_starts.push(*position);
                        }
                        self.index += 1;
                    } else {
                        self.next(Phase::EmbeddedProtected);
                    }
                }
                Phase::EmbeddedProtected => {
                    if let Some((offset, fallback)) = self.embedded.get(self.body) {
                        if let Some((range, closed, kind)) = fallback.protected.get(self.index) {
                            structure.protected.push((
                                (range.start + offset)..(range.end + offset),
                                *closed,
                                *kind,
                            ));
                            self.index += 1;
                        } else {
                            self.next(Phase::EmbeddedOpaque);
                        }
                    } else {
                        self.next(Phase::Complete);
                    }
                }
                Phase::EmbeddedOpaque => {
                    let (offset, fallback) = &self.embedded[self.body];
                    if let Some(position) = fallback.opaque_starts.get(self.index) {
                        structure.opaque_starts.push(position + offset);
                        self.index += 1;
                    } else {
                        self.body += 1;
                        self.next(Phase::EmbeddedProtected);
                    }
                }
                Phase::Complete => return true,
            }
        }
        matches!(self.phase, Phase::Complete)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_batches_match_full_collection_with_adjacent_embedded_bodies() {
        for ending in ["\n", "\r\n"] {
            let source = Arc::new(format!("{}文😀{ending}", "x".repeat(20_000)));
            let scopes = vec![
                (2000..4000, Language::JavaScript),
                (4000..6000, Language::Css),
                (8000..10000, Language::Python),
            ];
            let outer = Arc::new(FallbackContexts {
                protected: (0..2000)
                    .map(|index| (index * 8..index * 8 + 7, index % 2 == 0, RegionKind::String))
                    .collect(),
                opaque_starts: (0..2000).map(|index| index * 8).collect(),
            });
            let embedded: Vec<_> = scopes
                .iter()
                .map(|(body, _)| {
                    (
                        body.start,
                        Arc::new(FallbackContexts {
                            protected: (0..200)
                                .map(|index| {
                                    (
                                        index * 8..index * 8 + 7,
                                        index % 2 != 0,
                                        RegionKind::Template,
                                    )
                                })
                                .collect(),
                            opaque_starts: (0..200).map(|index| index * 8).collect(),
                        }),
                    )
                })
                .collect();
            let mut expected_protected = outer.protected.clone();
            let mut expected_opaque = outer.opaque_starts.clone();
            expected_protected
                .retain(|(range, _, _)| !scopes.iter().any(|(body, _)| overlaps(range, body)));
            expected_opaque
                .retain(|position| !scopes.iter().any(|(body, _)| body.contains(position)));
            for (offset, metadata) in &embedded {
                expected_protected.extend(metadata.protected.iter().map(
                    |(range, closed, kind)| {
                        ((range.start + offset)..(range.end + offset), *closed, *kind)
                    },
                ));
                expected_opaque.extend(
                    metadata
                        .opaque_starts
                        .iter()
                        .map(|position| position + offset),
                );
            }
            for budget in [1, 7, 256] {
                let mut work = Structure::prepare_parsed(
                    source.clone(),
                    Language::Html,
                    vec![],
                    scopes.clone(),
                    vec![],
                    vec![],
                );
                work.collect_fallbacks(outer.clone(), embedded.clone());
                assert_eq!(work.advance(0), Some(false));
                assert!(work.fallback.is_some());
                let mut batches = 0;
                while work.fallback.is_some() {
                    let before =
                        work.structure.protected.len() + work.structure.opaque_starts.len();
                    assert_eq!(work.advance(budget), Some(false));
                    let after = work.structure.protected.len() + work.structure.opaque_starts.len();
                    assert!(after - before <= budget, "one copied record per unit");
                    assert!(work.structure.brackets.is_empty());
                    batches += 1;
                }
                assert!(batches > 10);
                assert_eq!(work.structure.protected, expected_protected);
                assert_eq!(work.structure.opaque_starts, expected_opaque);
                assert!(
                    work.finish().is_none(),
                    "collection cannot publish before final validation"
                );
            }
        }
    }

    #[test]
    fn cancelling_fallback_collection_releases_source_and_retained_metadata() {
        let source = Arc::new("source".to_owned());
        let weak_source = Arc::downgrade(&source);
        let fallback = Arc::new(FallbackContexts {
            protected: vec![(0..6, true, RegionKind::String)],
            opaque_starts: vec![0],
        });
        let weak_metadata = Arc::downgrade(&fallback);
        let mut work =
            Structure::prepare_parsed(source, Language::Rust, vec![], vec![], vec![], vec![]);
        work.collect_fallbacks(fallback, vec![]);
        assert_eq!(work.advance(1), Some(false));
        assert!(work.finish().is_none());
        assert!(weak_source.upgrade().is_none());
        assert!(weak_metadata.upgrade().is_none());
    }
}
