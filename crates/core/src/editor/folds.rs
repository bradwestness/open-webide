//! Logical line ranges shared by parser, indentation and explicit-region providers.

/// The header remains visible; subsequent logical lines through `end_line` fold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FoldRange {
    pub start_line: usize,
    pub end_line: usize,
}

/// Keep one control per header and nested/disjoint ranges only.
pub fn normalize_folds(mut ranges: Vec<FoldRange>, line_count: usize) -> Vec<FoldRange> {
    ranges.retain(|range| range.start_line < range.end_line && range.end_line < line_count);
    ranges.sort_unstable_by_key(|range| (range.start_line, std::cmp::Reverse(range.end_line)));
    ranges.dedup_by_key(|range| range.start_line);
    let mut ancestors: Vec<FoldRange> = Vec::new();
    ranges.retain(|range| {
        while ancestors
            .last()
            .is_some_and(|parent| parent.end_line < range.start_line)
        {
            ancestors.pop();
        }
        if ancestors
            .last()
            .is_some_and(|parent| range.end_line > parent.end_line)
        {
            return false;
        }
        ancestors.push(*range);
        true
    });
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fold_providers_merge_without_crossing_or_duplicate_header_controls() {
        let folds = normalize_folds(
            vec![
                FoldRange {
                    start_line: 0,
                    end_line: 10,
                },
                FoldRange {
                    start_line: 0,
                    end_line: 5,
                },
                FoldRange {
                    start_line: 2,
                    end_line: 4,
                },
                FoldRange {
                    start_line: 3,
                    end_line: 7,
                },
                FoldRange {
                    start_line: 8,
                    end_line: 9,
                },
                FoldRange {
                    start_line: 11,
                    end_line: 12,
                },
                FoldRange {
                    start_line: 12,
                    end_line: 12,
                },
                FoldRange {
                    start_line: 13,
                    end_line: 20,
                },
            ],
            14,
        );
        assert_eq!(
            folds,
            vec![
                FoldRange {
                    start_line: 0,
                    end_line: 10
                },
                FoldRange {
                    start_line: 2,
                    end_line: 4
                },
                FoldRange {
                    start_line: 8,
                    end_line: 9
                },
                FoldRange {
                    start_line: 11,
                    end_line: 12
                },
            ]
        );
    }
}
