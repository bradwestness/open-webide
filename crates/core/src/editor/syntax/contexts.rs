//! Source-relative editing contexts from retained parser subtrees.
use super::super::SyntaxContextScope;
use super::subtrees::{ParsedSubtrees, Part, charge_visits};
use super::{
    MAX_FOLD_NODES, Node, RegionKind, SyntaxContextKind, SyntaxProvider, SyntaxStatus, Tree,
    visit_node,
};
use std::{collections::HashMap, ops::Range};

#[derive(Default)]
pub(super) struct SyntaxContexts {
    pub protected: Vec<(Range<usize>, bool, RegionKind)>,
    pub holes: Vec<(Range<usize>, Range<usize>)>,
    pub selections: Vec<Range<usize>>,
}

impl SyntaxContexts {
    fn mapped(&self, mut map: impl FnMut(&Range<usize>) -> Option<Range<usize>>) -> Option<Self> {
        Some(Self {
            protected: self
                .protected
                .iter()
                .map(|(range, closed, kind)| Some((map(range)?, *closed, *kind)))
                .collect::<Option<_>>()?,
            holes: self
                .holes
                .iter()
                .map(|(owner, hole)| Some((map(owner)?, map(hole)?)))
                .collect::<Option<_>>()?,
            selections: self
                .selections
                .iter()
                .map(&mut map)
                .collect::<Option<_>>()?,
        })
    }
    fn append(&mut self, mut other: Self) {
        self.protected.append(&mut other.protected);
        self.holes.append(&mut other.holes);
        self.selections.append(&mut other.selections);
    }
}

#[derive(Default)]
pub(super) struct ParsedContexts {
    subtrees: ParsedSubtrees<SyntaxContexts>,
    #[cfg(test)]
    pub(super) reused_nodes: usize,
}

impl ParsedContexts {
    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(super) fn collect(
        &mut self,
        tree: &Tree,
        provider: SyntaxProvider,
        text: &str,
        visited: &mut usize,
        contexts: &mut SyntaxContexts,
    ) -> Result<(), SyntaxStatus> {
        #[cfg(test)]
        {
            self.reused_nodes = 0;
        }
        let root = tree.root_node();
        if provider.context_scope == SyntaxContextScope::Document {
            self.clear();
            contexts.append(extract(root, provider, text, visited)?.0);
            return Ok(());
        }
        charge_visits(visited, 1)?;
        let mut ancestry = 0;
        let mut local = true;
        extract_node(
            root,
            root,
            provider,
            text,
            &mut ancestry,
            &mut local,
            contexts,
        )?;
        charge_visits(visited, ancestry)?;
        let mut next = HashMap::new();
        let mut cursor = root.walk();
        for node in root.children(&mut cursor) {
            let retained = self.subtrees.candidate(root, node).and_then(|part| {
                let metadata = part.value.mapped(|range| {
                    let start = range.start.checked_add(node.start_byte())?;
                    let end = range.end.checked_add(node.start_byte())?;
                    (start <= end
                        && end <= node.end_byte()
                        && text.is_char_boundary(start)
                        && text.is_char_boundary(end))
                    .then_some(start..end)
                })?;
                Some((part.clone(), metadata))
            });
            let metadata = if let Some((part, metadata)) = retained {
                charge_visits(visited, part.visits)?;
                #[cfg(test)]
                {
                    self.reused_nodes += part.visits;
                }
                next.insert(node.id(), part);
                metadata
            } else {
                let before = *visited;
                let (metadata, local) = extract(node, provider, text, visited)?;
                if local
                    && let Some(relative) = metadata.mapped(|range| {
                        if range.end > node.end_byte() {
                            return None;
                        }
                        Some(
                            range.start.checked_sub(node.start_byte())?
                                ..range.end.checked_sub(node.start_byte())?,
                        )
                    })
                {
                    next.insert(node.id(), Part::new(node, *visited - before, relative));
                }
                metadata
            };
            contexts.append(metadata);
        }
        self.subtrees.replace(tree, next);
        Ok(())
    }
}

fn extract(
    node: Node<'_>,
    provider: SyntaxProvider,
    text: &str,
    visited: &mut usize,
) -> Result<(SyntaxContexts, bool), SyntaxStatus> {
    let mut ancestry = 0;
    let mut local = true;
    let mut contexts = SyntaxContexts::default();
    visit_node(node, visited, |child| {
        extract_node(
            child,
            node,
            provider,
            text,
            &mut ancestry,
            &mut local,
            &mut contexts,
        )
    })?;
    charge_visits(visited, ancestry)?;
    Ok((contexts, local))
}

fn extract_node(
    node: Node<'_>,
    boundary: Node<'_>,
    provider: SyntaxProvider,
    text: &str,
    ancestry: &mut usize,
    local: &mut bool,
    contexts: &mut SyntaxContexts,
) -> Result<(), SyntaxStatus> {
    let range = node.start_byte()..node.end_byte();
    if range.start > range.end
        || !text.is_char_boundary(range.start)
        || !text.is_char_boundary(range.end)
    {
        return Err(SyntaxStatus::Cancelled);
    }
    if range.is_empty() {
        return Ok(());
    }
    if node.is_named() && !node.is_error() && !node.is_missing() {
        contexts.selections.push(range.clone());
    }
    let Some(class) = provider.context.and_then(|classify| classify(node)) else {
        return Ok(());
    };
    if class == SyntaxContextKind::Interpolation {
        let mut parent = node.parent();
        let mut inside = node.id() != boundary.id();
        while let Some(owner) = parent {
            *ancestry += 1;
            if *ancestry > MAX_FOLD_NODES {
                return Err(SyntaxStatus::TooLarge);
            }
            if matches!(
                provider.context.and_then(|classify| classify(owner)),
                Some(
                    SyntaxContextKind::String
                        | SyntaxContextKind::Template
                        | SyntaxContextKind::Regex
                )
            ) {
                *local &= inside;
                contexts
                    .holes
                    .push((owner.start_byte()..owner.end_byte(), range.clone()));
            }
            if owner.id() == boundary.id() {
                inside = false;
            }
            parent = owner.parent();
        }
        return Ok(());
    }
    let value = &text[range.clone()];
    let (kind, closed) = match class {
        SyntaxContextKind::String => (RegionKind::String, !node.has_error()),
        SyntaxContextKind::Template => (RegionKind::Template, !node.has_error()),
        SyntaxContextKind::Text => (RegionKind::Text, true),
        SyntaxContextKind::Regex => (RegionKind::Regex, !node.has_error()),
        SyntaxContextKind::Comment if value.starts_with("/*") => {
            (RegionKind::BlockComment, value.ends_with("*/"))
        }
        SyntaxContextKind::Comment if value.starts_with("<!--") => {
            (RegionKind::BlockComment, value.ends_with("-->"))
        }
        SyntaxContextKind::Comment => (RegionKind::LineComment, false),
        SyntaxContextKind::Interpolation => unreachable!(),
    };
    contexts.protected.push((range, closed, kind));
    Ok(())
}
