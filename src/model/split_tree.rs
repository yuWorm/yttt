#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitDirection {
    Horizontal,
    Vertical,
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

    pub fn focused_pane_id(&self) -> Option<&str> {
        self.focused_pane_id.as_deref()
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

    fn first_pane_id(&self) -> Option<&str> {
        match self {
            Self::Pane { id } => Some(id),
            Self::Split { first, .. } => first.first_pane_id(),
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
}
