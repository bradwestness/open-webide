//! Reconcile lexical fallback and parsed editing regions across bounded batches.
use super::{RegionKind, Structure, ordering::MetadataOrder};
use std::{
    collections::{HashMap, hash_map},
    ops::Range,
    vec::IntoIter,
};

type Protected = (Range<usize>, bool, RegionKind);
type OwnerHoles = hash_map::IntoIter<(usize, usize), Vec<Range<usize>>>;

pub(super) struct ContextReconciliation {
    parsed: Vec<Protected>,
    holes: IntoIter<(Range<usize>, Range<usize>)>,
    by_owner: HashMap<(usize, usize), Vec<Range<usize>>>,
    unclaimed: Option<OwnerHoles>,
    coverage: Vec<Range<usize>>,
    recognized: Vec<Range<usize>>,
    baseline: IntoIter<Protected>,
    active: Option<ParsedRegion>,
    merged: Vec<Protected>,
    order: MetadataOrder,
    index: usize,
    phase: Phase,
}

struct ParsedRegion {
    range: Range<usize>,
    closed: bool,
    kind: RegionKind,
    start: usize,
    holes: Vec<Range<usize>>,
    order: MetadataOrder,
    index: usize,
    ordered: bool,
}

enum Phase {
    Coverage,
    OrderCoverage,
    Recognize,
    Filter,
    Holes,
    Parsed,
    ReleaseHoles,
    OrderBaseline,
    Merge,
    Complete,
}

impl ContextReconciliation {
    pub fn new(parsed: Vec<Protected>, holes: Vec<(Range<usize>, Range<usize>)>) -> Self {
        Self {
            parsed,
            holes: holes.into_iter(),
            by_owner: HashMap::new(),
            unclaimed: None,
            coverage: Vec::new(),
            recognized: Vec::new(),
            baseline: Vec::new().into_iter(),
            active: None,
            merged: Vec::new(),
            order: MetadataOrder::new(),
            index: 0,
            phase: Phase::Coverage,
        }
    }

    fn next(&mut self, phase: Phase) {
        self.phase = phase;
        self.index = 0;
        self.order = MetadataOrder::new();
    }

    /// Each unit visits, orders or emits at most one metadata record. Vector and
    /// hash-table capacity growth remains allocated by the host runtime.
    pub fn advance(&mut self, structure: &mut Structure, budget: usize) -> bool {
        for _ in 0..budget {
            match self.phase {
                Phase::Coverage => {
                    if let Some((range, _, _)) = self.parsed.get(self.index) {
                        self.coverage.push(range.clone());
                        self.index += 1;
                    } else {
                        self.next(Phase::OrderCoverage);
                    }
                }
                Phase::OrderCoverage => {
                    if self
                        .order
                        .step(&mut self.coverage, |range| (range.start, range.end))
                    {
                        self.next(Phase::Recognize);
                    }
                }
                Phase::Recognize => {
                    if let Some(range) = self.coverage.get(self.index) {
                        if let Some(previous) = self.recognized.last_mut()
                            && range.start < previous.end
                        {
                            previous.end = previous.end.max(range.end);
                        } else {
                            self.recognized.push(range.clone());
                        }
                        self.index += 1;
                    } else {
                        self.baseline = std::mem::take(&mut structure.protected).into_iter();
                        self.next(Phase::Filter);
                    }
                }
                Phase::Filter => {
                    if let Some(region) = self.baseline.next() {
                        let range = &region.0;
                        let end = self
                            .recognized
                            .partition_point(|parsed| parsed.start <= range.start);
                        if !end
                            .checked_sub(1)
                            .and_then(|index| self.recognized.get(index))
                            .is_some_and(|parsed| {
                                range.end <= parsed.end || parsed.start == range.start
                            })
                        {
                            structure.protected.push(region);
                        }
                    } else {
                        self.next(Phase::Holes);
                    }
                }
                Phase::Holes => {
                    if let Some((owner, hole)) = self.holes.next() {
                        self.by_owner
                            .entry((owner.start, owner.end))
                            .or_default()
                            .push(hole);
                    } else {
                        self.next(Phase::Parsed);
                    }
                }
                Phase::Parsed => {
                    if let Some(active) = &mut self.active {
                        if !active.ordered {
                            active.ordered =
                                active.order.step(&mut active.holes, |hole| (hole.start, 0));
                        } else if let Some(hole) = active.holes.get(active.index) {
                            if active.start < hole.start {
                                structure.protected.push((
                                    active.start..hole.start,
                                    false,
                                    active.kind,
                                ));
                            }
                            active.start = active.start.max(hole.end);
                            structure.opaque_starts.push(active.start);
                            active.index += 1;
                        } else {
                            if active.start < active.range.end {
                                structure.protected.push((
                                    active.start..active.range.end,
                                    active.closed,
                                    active.kind,
                                ));
                            }
                            self.active = None;
                        }
                    } else if let Some((range, closed, kind)) = self.parsed.get(self.index) {
                        if *kind == RegionKind::Text {
                            structure.opaque_starts.push(range.start);
                        }
                        self.active = Some(ParsedRegion {
                            range: range.clone(),
                            closed: *closed,
                            kind: *kind,
                            start: range.start,
                            holes: self
                                .by_owner
                                .remove(&(range.start, range.end))
                                .unwrap_or_default(),
                            order: MetadataOrder::new(),
                            index: 0,
                            ordered: false,
                        });
                        self.index += 1;
                    } else {
                        self.unclaimed = Some(std::mem::take(&mut self.by_owner).into_iter());
                        self.next(Phase::ReleaseHoles);
                    }
                }
                Phase::ReleaseHoles => {
                    if self.unclaimed.as_mut().unwrap().next().is_none() {
                        self.unclaimed = None;
                        self.next(Phase::OrderBaseline);
                    }
                }
                Phase::OrderBaseline => {
                    if self.order.step(&mut structure.protected, |(range, _, _)| {
                        (range.start, range.end)
                    }) {
                        self.baseline = std::mem::take(&mut structure.protected).into_iter();
                        self.next(Phase::Merge);
                    }
                }
                Phase::Merge => {
                    if let Some((range, closed, kind)) = self.baseline.next() {
                        if let Some((previous, previous_closed, _)) = self.merged.last_mut()
                            && range.start < previous.end
                        {
                            if range.end > previous.end {
                                previous.end = range.end;
                                *previous_closed = closed;
                            }
                        } else {
                            self.merged.push((range, closed, kind));
                        }
                    } else {
                        structure.protected = std::mem::take(&mut self.merged);
                        self.next(Phase::Complete);
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
    use crate::highlight::Language;
    use std::sync::Arc;

    // Independent oracle retains the previous synchronous reconciliation.
    fn reference(
        mut baseline: Vec<Protected>,
        parsed: Vec<Protected>,
        holes: Vec<(Range<usize>, Range<usize>)>,
        mut opaque: Vec<usize>,
    ) -> (Vec<Protected>, Vec<usize>) {
        let mut coverage: Vec<_> = parsed.iter().map(|(range, _, _)| range.clone()).collect();
        coverage.sort_by_key(|range| (range.start, range.end));
        let mut recognized: Vec<Range<usize>> = Vec::new();
        for range in coverage {
            if let Some(previous) = recognized.last_mut()
                && range.start < previous.end
            {
                previous.end = previous.end.max(range.end);
            } else {
                recognized.push(range);
            }
        }
        baseline.retain(|(range, _, _)| {
            let end = recognized.partition_point(|parsed| parsed.start <= range.start);
            !end.checked_sub(1)
                .and_then(|index| recognized.get(index))
                .is_some_and(|parsed| range.end <= parsed.end || parsed.start == range.start)
        });
        let mut by_owner: HashMap<(usize, usize), Vec<Range<usize>>> = HashMap::new();
        for (owner, hole) in holes {
            by_owner
                .entry((owner.start, owner.end))
                .or_default()
                .push(hole);
        }
        for (range, closed, kind) in parsed {
            if kind == RegionKind::Text {
                opaque.push(range.start);
            }
            let mut start = range.start;
            let mut owned = by_owner
                .remove(&(range.start, range.end))
                .unwrap_or_default();
            owned.sort_by_key(|hole| hole.start);
            for hole in owned {
                if start < hole.start {
                    baseline.push((start..hole.start, false, kind));
                }
                start = start.max(hole.end);
                opaque.push(start);
            }
            if start < range.end {
                baseline.push((start..range.end, closed, kind));
            }
        }
        baseline.sort_by_key(|(range, _, _)| (range.start, range.end));
        let mut protected: Vec<Protected> = Vec::new();
        for (range, closed, kind) in baseline {
            if let Some((previous, previous_closed, _)) = protected.last_mut()
                && range.start < previous.end
            {
                if range.end > previous.end {
                    previous.end = range.end;
                    *previous_closed = closed;
                }
            } else {
                protected.push((range, closed, kind));
            }
        }
        (protected, opaque)
    }

    #[test]
    fn bounded_reconciliation_matches_lexical_and_parsed_region_oracle() {
        for crlf in [false, true] {
            let source =
                Arc::new(format!("{}{{文}}", "x".repeat(9000)) + if crlf { "\r\n" } else { "\n" });
            let mut parsed = Vec::new();
            let mut baseline = Vec::new();
            let mut holes = Vec::new();
            for index in (0..1000).rev() {
                let start = index * 8;
                let owner = start..start + 7;
                let kind = if index % 3 == 0 {
                    RegionKind::Text
                } else {
                    RegionKind::Template
                };
                parsed.push((owner.clone(), index % 2 == 0, kind));
                // Reverse, overlapping and equal-key holes exercise stable ordering.
                holes.extend([
                    (owner.clone(), start + 4..start + 6),
                    (owner.clone(), start + 2..start + 4),
                    (owner.clone(), start + 2..start + 3),
                ]);
                baseline.push((start..start + 8, index % 2 != 0, RegionKind::String));
                baseline.push((start + 1..start + 2, true, RegionKind::LineComment));
            }
            // An interpolation owner absent from parsed protection is discarded.
            holes.push((8500..8600, 8520..8540));
            let opaque = vec![8990];
            let (protected, expected_opaque) = reference(
                baseline.clone(),
                parsed.clone(),
                holes.clone(),
                opaque.clone(),
            );
            let expected = Structure::parsed(
                source.clone(),
                Language::Rust,
                protected,
                vec![],
                expected_opaque,
                std::iter::once(0..9005).collect(),
            )
            .unwrap();
            for budget in [1, 7, 256] {
                let mut work = Structure::prepare_parsed(
                    source.clone(),
                    Language::Rust,
                    baseline.clone(),
                    vec![],
                    opaque.clone(),
                    std::iter::once(0..9005).collect(),
                );
                work.reconcile(parsed.clone(), holes.clone());
                assert_eq!(work.advance(0), Some(false));
                assert!(work.reconciliation.is_some());
                let mut batches = 0;
                while !work.advance(budget).unwrap() {
                    batches += 1;
                }
                assert!(batches > 30);
                let actual = work.finish().unwrap();
                assert!(Arc::ptr_eq(&actual.source, &source));
                assert_eq!(actual.protected, expected.protected);
                assert_eq!(actual.opaque_starts, expected.opaque_starts);
                assert_eq!(actual.selection_ranges, expected.selection_ranges);
                assert_eq!(actual.brackets, expected.brackets);
            }
        }
    }

    #[test]
    fn unfinished_reconciliation_releases_source_and_cannot_publish() {
        let source = Arc::new("x".repeat(10000));
        let weak = Arc::downgrade(&source);
        let mut work =
            Structure::prepare_parsed(source, Language::Rust, vec![], vec![], vec![], vec![]);
        work.reconcile(vec![(0..8000, true, RegionKind::String)], vec![]);
        assert_eq!(work.advance(1), Some(false));
        assert!(work.finish().is_none());
        assert!(weak.upgrade().is_none());
    }
}
