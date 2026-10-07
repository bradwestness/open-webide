//! Retain descendant color classifications while refreshing external parent roles.
use super::subtrees::{ParsedSubtrees, Part, charge_visits, visit_parts};
use super::{Node, SyntaxProvider, SyntaxStatus, Tree, visit_node};
use crate::{editor::SyntaxHighlightScope, highlight::TokenKind};
use std::{collections::HashMap, ops::Range};

type Spans = Vec<(Range<usize>, TokenKind)>;

#[derive(Default)]
pub(super) struct ParsedHighlights {
    subtrees: ParsedSubtrees<Spans>,
    #[cfg(test)]
    pub(super) reused_nodes: usize,
}

impl ParsedHighlights {
    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(super) fn collect(
        &mut self,
        tree: &Tree,
        provider: SyntaxProvider,
        text: &str,
        visited: &mut usize,
        spans: &mut Spans,
    ) -> Result<(), SyntaxStatus> {
        #[cfg(test)]
        {
            self.reused_nodes = 0;
        }
        if provider.highlight.is_none() {
            self.clear();
            return Ok(());
        }
        let root = tree.root_node();
        if provider.highlight_scope == SyntaxHighlightScope::Document {
            self.clear();
            return visit_node(root, visited, |node| {
                classify(node, provider, text, spans);
                Ok(())
            });
        }
        charge_visits(visited, 1)?;
        classify(root, provider, text, spans);
        let mut next = HashMap::new();
        visit_parts(root, |node, complete| {
            // Only this node's field membership can depend on an external parent.
            charge_visits(visited, 1)?;
            classify(node, provider, text, spans);
            if !complete {
                return Ok(());
            }
            let retained = self.subtrees.candidate(root, node).and_then(|part| {
                let absolute = part
                    .value
                    .iter()
                    .map(|(range, kind)| {
                        let start = node.start_byte().checked_add(range.start)?;
                        let end = node.start_byte().checked_add(range.end)?;
                        (start < end
                            && end <= node.end_byte()
                            && text.is_char_boundary(start)
                            && text.is_char_boundary(end))
                        .then_some((start..end, *kind))
                    })
                    .collect::<Option<Spans>>()?;
                Some((part.clone(), absolute))
            });
            if let Some((part, absolute)) = retained {
                charge_visits(visited, part.visits)?;
                #[cfg(test)]
                {
                    self.reused_nodes += part.visits;
                }
                spans.extend(absolute);
                next.insert(node.id(), part);
            } else {
                let before = *visited;
                let mut descendants = Vec::new();
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    visit_node(child, visited, |child| {
                        classify(child, provider, text, &mut descendants);
                        Ok(())
                    })?;
                }
                if *visited > before {
                    let relative = descendants
                        .iter()
                        .map(|(range, kind)| {
                            (
                                range.start - node.start_byte()..range.end - node.start_byte(),
                                *kind,
                            )
                        })
                        .collect();
                    next.insert(node.id(), Part::new(node, *visited - before, relative));
                }
                spans.extend(descendants);
            }
            Ok(())
        })?;
        self.subtrees.replace(tree, next);
        Ok(())
    }
}

fn classify(node: Node<'_>, provider: SyntaxProvider, text: &str, spans: &mut Spans) {
    if let Some(kind) = provider.highlight.and_then(|select| select(node)) {
        let range = node.byte_range();
        if !range.is_empty()
            && text.is_char_boundary(range.start)
            && text.is_char_boundary(range.end)
        {
            spans.push((range, kind));
        }
    }
}
