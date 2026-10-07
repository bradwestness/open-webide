//! Reuse parser fold descriptors for unchanged top-level subtrees.
use super::{
    FoldRange, MAX_FOLD_NODES, Node, Point, SyntaxProvider, SyntaxStatus, Tree, visit_node,
};
use std::{collections::HashMap, sync::Arc};

type Span = (Point, Point);

struct Chunk {
    kind: u16,
    bytes: usize,
    nodes: usize,
    spans: Vec<Span>,
}

#[derive(Default)]
pub(super) struct ParsedFolds {
    // Retaining the old tree prevents its node IDs being recycled for unrelated nodes.
    tree: Option<Tree>,
    chunks: HashMap<usize, Arc<Chunk>>,
    #[cfg(test)]
    pub(super) reused_nodes: usize,
}

impl ParsedFolds {
    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(super) fn collect(
        &mut self,
        tree: &Tree,
        provider: SyntaxProvider,
        lines: &[&str],
        ranges: &mut Vec<FoldRange>,
        visited: &mut usize,
    ) -> Result<(), SyntaxStatus> {
        #[cfg(test)]
        {
            self.reused_nodes = 0;
        }
        let root = tree.root_node();
        charge(visited, 1)?;
        if let Some(span) = fold(root, provider) {
            publish(span, lines, ranges);
        }
        let same_root = self
            .tree
            .as_ref()
            .is_some_and(|old| old.root_node().kind_id() == root.kind_id());
        let mut next = HashMap::new();
        let mut cursor = root.walk();
        for node in root.children(&mut cursor) {
            let reused = same_root
                .then(|| self.chunks.get(&node.id()))
                .flatten()
                .filter(|chunk| {
                    chunk.kind == node.kind_id() && chunk.bytes == node.byte_range().len()
                })
                .and_then(|chunk| {
                    let spans = chunk
                        .spans
                        .iter()
                        .map(|&(start, end)| {
                            Some((
                                absolute(start, node.start_position())?,
                                absolute(end, node.start_position())?,
                            ))
                        })
                        .collect::<Option<Vec<_>>>()?;
                    spans
                        .iter()
                        .all(|&(start, end)| valid(start, lines) && valid(end, lines))
                        .then(|| (chunk.clone(), spans))
                });
            let spans = if let Some((chunk, spans)) = reused {
                charge(visited, chunk.nodes)?;
                #[cfg(test)]
                {
                    self.reused_nodes += chunk.nodes;
                }
                next.insert(node.id(), chunk);
                spans
            } else {
                let before = *visited;
                let mut spans = Vec::new();
                let mut local_headers = true;
                visit_node(node, visited, |child| {
                    if let Some(span) = fold(child, provider) {
                        if child.id() == node.id()
                            && provider.parent_headers.contains(&child.kind())
                        {
                            local_headers = false;
                        }
                        spans.push(span);
                    }
                    Ok(())
                })?;
                // A parent-owned header outside this subtree cannot be rebased locally.
                if local_headers
                    && let Some(relative) = spans
                        .iter()
                        .map(|&(start, end)| {
                            Some((
                                relative(start, node.start_position())?,
                                relative(end, node.start_position())?,
                            ))
                        })
                        .collect::<Option<Vec<_>>>()
                {
                    next.insert(
                        node.id(),
                        Arc::new(Chunk {
                            kind: node.kind_id(),
                            bytes: node.byte_range().len(),
                            nodes: *visited - before,
                            spans: relative,
                        }),
                    );
                }
                spans
            };
            for span in spans {
                // Closing-line siblings may have changed even when this subtree did not.
                publish(span, lines, ranges);
            }
        }
        self.tree = Some(tree.clone());
        self.chunks = next;
        Ok(())
    }
}

fn charge(visited: &mut usize, nodes: usize) -> Result<(), SyntaxStatus> {
    *visited = visited.saturating_add(nodes);
    if *visited > MAX_FOLD_NODES {
        Err(SyntaxStatus::TooLarge)
    } else {
        Ok(())
    }
}

fn fold(node: Node<'_>, provider: SyntaxProvider) -> Option<Span> {
    if !provider.fold_nodes.contains(&node.kind()) || node.is_missing() {
        return None;
    }
    let start = if provider.parent_headers.contains(&node.kind()) {
        node.parent()
            .map_or(node.start_position(), |parent| parent.start_position())
    } else {
        node.start_position()
    };
    Some((start, node.end_position()))
}

fn publish((start, end): Span, lines: &[&str], ranges: &mut Vec<FoldRange>) {
    let trailing = lines
        .get(end.row)
        .and_then(|line| line.get(end.column..))
        .is_some_and(|rest| !rest.trim().is_empty());
    ranges.push(FoldRange {
        start_line: start.row,
        end_line: end
            .row
            .saturating_sub(usize::from(end.column == 0 || trailing)),
    });
}

fn relative(point: Point, base: Point) -> Option<Point> {
    let row = point.row.checked_sub(base.row)?;
    Some(Point {
        row,
        column: if row == 0 {
            point.column.checked_sub(base.column)?
        } else {
            point.column
        },
    })
}

fn absolute(point: Point, base: Point) -> Option<Point> {
    Some(Point {
        row: point.row.checked_add(base.row)?,
        column: if point.row == 0 {
            point.column.checked_add(base.column)?
        } else {
            point.column
        },
    })
}

fn valid(point: Point, lines: &[&str]) -> bool {
    lines
        .get(point.row)
        .is_some_and(|line| line.is_char_boundary(point.column))
}
