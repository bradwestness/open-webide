//! Ordering and target selection shared by project and file tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabAction {
    CloseOthers,
    CloseLeft,
    CloseRight,
    MoveLeft,
    MoveRight,
}

impl TabAction {
    pub fn targets<T: Clone + PartialEq>(self, tabs: &[T], selected: &T) -> Vec<T> {
        let Some(index) = tabs.iter().position(|tab| tab == selected) else {
            return Vec::new();
        };
        match self {
            Self::CloseOthers => tabs
                .iter()
                .filter(|tab| *tab != selected)
                .cloned()
                .collect(),
            Self::CloseLeft => tabs[..index].to_vec(),
            Self::CloseRight => tabs[index + 1..].to_vec(),
            Self::MoveLeft | Self::MoveRight => Vec::new(),
        }
    }

    pub fn reorder<T: PartialEq>(self, tabs: &mut [T], selected: &T) {
        let Some(index) = tabs.iter().position(|tab| tab == selected) else {
            return;
        };
        let neighbor = match self {
            Self::MoveLeft => index.checked_sub(1),
            Self::MoveRight => index.checked_add(1).filter(|next| *next < tabs.len()),
            _ => None,
        };
        if let Some(neighbor) = neighbor {
            tabs.swap(index, neighbor);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tab_actions_preserve_the_anchor_and_respect_edges() {
        let mut tabs = vec![1, 2, 3];
        assert_eq!(TabAction::CloseOthers.targets(&tabs, &2), [1, 3]);
        assert_eq!(TabAction::CloseLeft.targets(&tabs, &2), [1]);
        assert_eq!(TabAction::CloseRight.targets(&tabs, &2), [3]);
        assert!(TabAction::CloseOthers.targets(&tabs, &4).is_empty());
        TabAction::MoveLeft.reorder(&mut tabs, &1);
        assert_eq!(tabs, [1, 2, 3]);
        TabAction::MoveRight.reorder(&mut tabs, &2);
        assert_eq!(tabs, [1, 3, 2]);
        TabAction::MoveLeft.reorder(&mut tabs, &2);
        assert_eq!(tabs, [1, 2, 3]);
    }
}
