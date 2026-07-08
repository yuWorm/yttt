#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitDirection {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SplitNode {
    Pane {
        id: String,
    },
    Split {
        direction: SplitDirection,
        ratio: f32,
        first: Box<SplitNode>,
        second: Box<SplitNode>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct SplitTree {
    root: SplitNode,
    focused_pane_id: Option<String>,
}

impl SplitTree {
    pub fn single(id: impl Into<String>) -> Self {
        let id = id.into();
        Self {
            root: SplitNode::Pane { id: id.clone() },
            focused_pane_id: Some(id),
        }
    }

    pub fn split_focused(
        &mut self,
        direction: SplitDirection,
        new_pane_id: impl Into<String>,
    ) -> Result<(), SplitTreeError> {
        let Some(focused_pane_id) = self.focused_pane_id.clone() else {
            return Err(SplitTreeError::NoFocusedPane);
        };
        let new_pane_id = new_pane_id.into();

        if !self
            .root
            .split_pane(&focused_pane_id, direction, &new_pane_id)
        {
            return Err(SplitTreeError::PaneNotFound(focused_pane_id));
        }

        self.focused_pane_id = Some(new_pane_id);
        Ok(())
    }

    pub fn close_focused(&mut self) -> Result<(), SplitTreeError> {
        let Some(focused_pane_id) = self.focused_pane_id.clone() else {
            return Err(SplitTreeError::NoFocusedPane);
        };

        let pane_count = self.pane_ids().len();
        if pane_count <= 1 {
            return Err(SplitTreeError::CannotCloseLastPane);
        }

        if !self.root.remove_pane(&focused_pane_id) {
            return Err(SplitTreeError::PaneNotFound(focused_pane_id));
        }

        self.focused_pane_id = self.root.first_pane_id().map(ToOwned::to_owned);
        Ok(())
    }

    pub fn focus_direction(&mut self, direction: FocusDirection) -> Result<(), SplitTreeError> {
        let Some(focused_pane_id) = self.focused_pane_id.clone() else {
            return Err(SplitTreeError::NoFocusedPane);
        };

        let next_pane_id = self
            .root
            .adjacent_pane(&focused_pane_id, direction)
            .ok_or(SplitTreeError::NoAdjacentPane)?;
        self.focused_pane_id = Some(next_pane_id.to_string());
        Ok(())
    }

    pub fn resize_focused(
        &mut self,
        direction: ResizeDirection,
        delta: f32,
    ) -> Result<(), SplitTreeError> {
        let Some(focused_pane_id) = self.focused_pane_id.clone() else {
            return Err(SplitTreeError::NoFocusedPane);
        };

        if self.root.resize_pane(&focused_pane_id, direction, delta) {
            Ok(())
        } else {
            Err(SplitTreeError::NoResizableSplit)
        }
    }

    pub fn focused_pane_id(&self) -> Option<&str> {
        self.focused_pane_id.as_deref()
    }

    pub fn root_ratio(&self) -> Option<f32> {
        match &self.root {
            SplitNode::Pane { .. } => None,
            SplitNode::Split { ratio, .. } => Some(*ratio),
        }
    }

    pub fn pane_ids(&self) -> Vec<&str> {
        let mut ids = Vec::new();
        self.root.collect_pane_ids(&mut ids);
        ids
    }
}

impl SplitNode {
    fn split_pane(
        &mut self,
        target_id: &str,
        direction: SplitDirection,
        new_pane_id: &str,
    ) -> bool {
        match self {
            Self::Pane { id } if id == target_id => {
                let existing = std::mem::replace(self, Self::Pane { id: String::new() });
                *self = Self::Split {
                    direction,
                    ratio: 0.5,
                    first: Box::new(existing),
                    second: Box::new(Self::Pane {
                        id: new_pane_id.to_string(),
                    }),
                };
                true
            }
            Self::Pane { .. } => false,
            Self::Split { first, second, .. } => {
                first.split_pane(target_id, direction, new_pane_id)
                    || second.split_pane(target_id, direction, new_pane_id)
            }
        }
    }

    fn remove_pane(&mut self, target_id: &str) -> bool {
        match self {
            Self::Pane { .. } => false,
            Self::Split { first, second, .. } => {
                if first.is_pane(target_id) {
                    *self = (**second).clone();
                    true
                } else if second.is_pane(target_id) {
                    *self = (**first).clone();
                    true
                } else {
                    first.remove_pane(target_id) || second.remove_pane(target_id)
                }
            }
        }
    }

    fn is_pane(&self, target_id: &str) -> bool {
        matches!(self, Self::Pane { id } if id == target_id)
    }

    fn contains_pane(&self, target_id: &str) -> bool {
        match self {
            Self::Pane { id } => id == target_id,
            Self::Split { first, second, .. } => {
                first.contains_pane(target_id) || second.contains_pane(target_id)
            }
        }
    }

    fn adjacent_pane(&self, target_id: &str, focus_direction: FocusDirection) -> Option<&str> {
        match self {
            Self::Pane { .. } => None,
            Self::Split {
                direction,
                first,
                second,
                ..
            } => {
                if first.contains_pane(target_id) {
                    first.adjacent_pane(target_id, focus_direction).or_else(|| {
                        match (*direction, focus_direction) {
                            (SplitDirection::Horizontal, FocusDirection::Right)
                            | (SplitDirection::Vertical, FocusDirection::Down) => {
                                second.first_pane_id()
                            }
                            _ => None,
                        }
                    })
                } else if second.contains_pane(target_id) {
                    second
                        .adjacent_pane(target_id, focus_direction)
                        .or_else(|| match (*direction, focus_direction) {
                            (SplitDirection::Horizontal, FocusDirection::Left)
                            | (SplitDirection::Vertical, FocusDirection::Up) => {
                                first.last_pane_id()
                            }
                            _ => None,
                        })
                } else {
                    None
                }
            }
        }
    }

    fn resize_pane(
        &mut self,
        target_id: &str,
        resize_direction: ResizeDirection,
        delta: f32,
    ) -> bool {
        match self {
            Self::Pane { .. } => false,
            Self::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                if first.resize_pane(target_id, resize_direction, delta)
                    || second.resize_pane(target_id, resize_direction, delta)
                {
                    return true;
                }

                let target_in_first = first.contains_pane(target_id);
                let target_in_second = second.contains_pane(target_id);
                if !target_in_first && !target_in_second {
                    return false;
                }

                let adjustment = match (*direction, resize_direction) {
                    (SplitDirection::Horizontal, ResizeDirection::Right)
                    | (SplitDirection::Vertical, ResizeDirection::Down) => delta,
                    (SplitDirection::Horizontal, ResizeDirection::Left)
                    | (SplitDirection::Vertical, ResizeDirection::Up) => -delta,
                    _ => return false,
                };
                *ratio = (*ratio + adjustment).clamp(0.1, 0.9);
                true
            }
        }
    }

    fn first_pane_id(&self) -> Option<&str> {
        match self {
            Self::Pane { id } => Some(id),
            Self::Split { first, .. } => first.first_pane_id(),
        }
    }

    fn last_pane_id(&self) -> Option<&str> {
        match self {
            Self::Pane { id } => Some(id),
            Self::Split { second, .. } => second.last_pane_id(),
        }
    }

    fn collect_pane_ids<'a>(&'a self, ids: &mut Vec<&'a str>) {
        match self {
            Self::Pane { id } => ids.push(id),
            Self::Split { first, second, .. } => {
                first.collect_pane_ids(ids);
                second.collect_pane_ids(ids);
            }
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SplitTreeError {
    #[error("no focused pane")]
    NoFocusedPane,
    #[error("pane not found: {0}")]
    PaneNotFound(String),
    #[error("cannot close the last pane")]
    CannotCloseLastPane,
    #[error("no adjacent pane in that direction")]
    NoAdjacentPane,
    #[error("no resizable split in that direction")]
    NoResizableSplit,
}
