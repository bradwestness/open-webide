//! Map native coordinates onto paint that can omit unrendered source ranges.
use super::{EditError, MAX_EDITOR_BYTES, MAX_EDITOR_LINES};
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaintPosition {
    pub fragment: usize,
    pub offset: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaintSelection {
    pub fragment: usize,
    pub range: Range<usize>,
}

/// Coordinates are native UTF-16 units relative to one logical row. Adapters
/// provide the rendered intervals; gaps never become implied source text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaintCoverage {
    length: usize,
    fragments: Vec<Range<usize>>,
}
impl PaintCoverage {
    pub fn new(length: usize, fragments: Vec<Range<usize>>) -> Result<Self, EditError> {
        if length > MAX_EDITOR_BYTES || fragments.len() > MAX_EDITOR_LINES {
            return Err(EditError::InvalidSelection);
        }
        let mut end = 0;
        for fragment in &fragments {
            if fragment.start < end
                || fragment.start > fragment.end
                || fragment.end > length
                || fragment.is_empty() && fragment.end != length
            {
                return Err(EditError::InvalidSelection);
            }
            end = fragment.end;
        }
        Ok(Self { length, fragments })
    }
    /// At a shared boundary, prefer the following fragment, matching native
    /// caret affinity. An unpainted gap cannot borrow the preceding fragment.
    pub fn caret(&self, offset: usize) -> Option<PaintPosition> {
        if offset > self.length {
            return None;
        }
        let fragment = self
            .fragments
            .partition_point(|range| range.start <= offset)
            .checked_sub(1)?;
        let range = &self.fragments[fragment];
        (offset < range.end || offset == self.length && offset == range.end).then_some(
            PaintPosition {
                fragment,
                offset: offset - range.start,
            },
        )
    }
    /// Map a DOM fragment endpoint back to its native source coordinate. A hit
    /// at the fragment's trailing edge retains that exact endpoint, even when
    /// the following source is not painted yet.
    pub fn source_offset(&self, fragment: usize, offset: usize) -> Option<usize> {
        let range = self.fragments.get(fragment)?;
        (offset <= range.len()).then(|| range.start + offset)
    }
    /// Clip selections independently against each rendered interval. This
    /// avoids clamping endpoints across gaps or joining unrelated DOM ranges.
    pub fn selection(&self, range: Range<usize>) -> Result<Vec<PaintSelection>, EditError> {
        if range.start > range.end || range.end > self.length {
            return Err(EditError::InvalidSelection);
        }
        if range.is_empty() {
            return Ok(self
                .caret(range.start)
                .map(|at| PaintSelection {
                    fragment: at.fragment,
                    range: at.offset..at.offset,
                })
                .into_iter()
                .collect());
        }
        let first = self
            .fragments
            .partition_point(|fragment| fragment.end <= range.start);
        Ok(self
            .fragments
            .iter()
            .enumerate()
            .skip(first)
            .take_while(|(_, fragment)| fragment.start < range.end)
            .filter_map(|(index, fragment)| {
                let start = range.start.max(fragment.start);
                let end = range.end.min(fragment.end);
                (start < end).then_some(PaintSelection {
                    fragment: index,
                    range: start - fragment.start..end - fragment.start,
                })
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gaps_boundaries_and_eof_preserve_native_affinity_without_clamping() {
        let coverage = PaintCoverage::new(30, vec![4..8, 8..12, 20..24, 28..30]).unwrap();
        assert_eq!(
            coverage.caret(8),
            Some(PaintPosition {
                fragment: 1,
                offset: 0
            })
        );
        assert_eq!(
            coverage.caret(7),
            Some(PaintPosition {
                fragment: 0,
                offset: 3
            })
        );
        assert_eq!(
            coverage.caret(30),
            Some(PaintPosition {
                fragment: 3,
                offset: 2
            })
        );
        for at in [0, 3, 12, 16, 24, 27, 31, usize::MAX] {
            assert_eq!(coverage.caret(at), None);
        }
        assert_eq!(
            coverage.selection(0..30).unwrap(),
            vec![
                PaintSelection {
                    fragment: 0,
                    range: 0..4
                },
                PaintSelection {
                    fragment: 1,
                    range: 0..4
                },
                PaintSelection {
                    fragment: 2,
                    range: 0..4
                },
                PaintSelection {
                    fragment: 3,
                    range: 0..2
                },
            ]
        );
        assert_eq!(
            coverage.selection(6..22).unwrap(),
            vec![
                PaintSelection {
                    fragment: 0,
                    range: 2..4
                },
                PaintSelection {
                    fragment: 1,
                    range: 0..4
                },
                PaintSelection {
                    fragment: 2,
                    range: 0..2
                },
            ]
        );
        assert_eq!(coverage.source_offset(0, 4), Some(8));
        assert_eq!(coverage.source_offset(1, 4), Some(12));
        assert_eq!(coverage.source_offset(2, 3), Some(23));
        assert_eq!(coverage.source_offset(3, 2), Some(30));
        assert_eq!(coverage.source_offset(0, 5), None);
        assert_eq!(coverage.source_offset(4, 0), None);
        assert!(coverage.selection(12..20).unwrap().is_empty());
        assert!(coverage.selection(12..12).unwrap().is_empty());
        assert!(coverage.selection(0..31).is_err());
        let empty = PaintCoverage::new(0, std::iter::once(0..0).collect()).unwrap();
        assert_eq!(
            empty.caret(0),
            Some(PaintPosition {
                fragment: 0,
                offset: 0
            })
        );
        assert_eq!(PaintCoverage::new(0, vec![]).unwrap().caret(0), None);
        for fragments in [
            vec![8..12, 4..8],
            vec![4..10, 8..12],
            std::iter::once(0..31).collect(),
            std::iter::once(3..3).collect(),
        ] {
            assert!(PaintCoverage::new(30, fragments).is_err());
        }
    }
}
