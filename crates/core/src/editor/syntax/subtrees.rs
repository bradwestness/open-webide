//! Retained tree ownership and identity checks shared by syntax consumers.
use super::{MAX_FOLD_NODES, Node, SyntaxStatus, Tree};
use std::{collections::HashMap, sync::Arc};

pub(super) struct Part<T> {
    kind: u16,
    bytes: usize,
    pub visits: usize,
    pub value: Arc<T>,
}

impl<T> Clone for Part<T> {
    fn clone(&self) -> Self {
        Self {
            kind: self.kind,
            bytes: self.bytes,
            visits: self.visits,
            value: self.value.clone(),
        }
    }
}

impl<T> Part<T> {
    pub(super) fn new(node: Node<'_>, visits: usize, value: T) -> Self {
        Self {
            kind: node.kind_id(),
            bytes: node.byte_range().len(),
            visits,
            value: Arc::new(value),
        }
    }
}

pub(super) struct ParsedSubtrees<T> {
    // Keep old node allocations alive: a freed node ID could identify a new node.
    tree: Option<Tree>,
    parts: HashMap<usize, Part<T>>,
}

impl<T> Default for ParsedSubtrees<T> {
    fn default() -> Self {
        Self {
            tree: None,
            parts: HashMap::new(),
        }
    }
}

impl<T> ParsedSubtrees<T> {
    pub(super) fn candidate(&self, root: Node<'_>, node: Node<'_>) -> Option<&Part<T>> {
        if self.tree.as_ref()?.root_node().kind_id() != root.kind_id() {
            return None;
        }
        self.parts
            .get(&node.id())
            .filter(|part| part.kind == node.kind_id() && part.bytes == node.byte_range().len())
    }

    pub(super) fn replace(&mut self, tree: &Tree, parts: HashMap<usize, Part<T>>) {
        self.tree = Some(tree.clone());
        self.parts = parts;
    }
}

pub(super) fn charge_visits(visited: &mut usize, visits: usize) -> Result<(), SyntaxStatus> {
    *visited = visited.saturating_add(visits);
    if *visited > MAX_FOLD_NODES {
        Err(SyntaxStatus::TooLarge)
    } else {
        Ok(())
    }
}
