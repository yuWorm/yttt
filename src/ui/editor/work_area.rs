use std::collections::HashSet;

use super::workspace::{DocumentId, WorkItemId};

const MIN_SPLIT_RATIO: f32 = 0.05;
const MAX_SPLIT_RATIO: f32 = 0.95;
const PLACEHOLDER_GROUP_ID: TabGroupId = TabGroupId(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TabGroupId(u64);

impl TabGroupId {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WorkAreaSplitId(u64);

impl WorkAreaSplitId {
    pub fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkAreaSplitAxis {
    Row,
    Column,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkAreaDropEdge {
    Left,
    Right,
    Top,
    Bottom,
}

impl WorkAreaDropEdge {
    fn axis(self) -> WorkAreaSplitAxis {
        match self {
            Self::Left | Self::Right => WorkAreaSplitAxis::Row,
            Self::Top | Self::Bottom => WorkAreaSplitAxis::Column,
        }
    }

    fn places_new_group_first(self) -> bool {
        matches!(self, Self::Left | Self::Top)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkAreaDropPlacement {
    TabIndex(usize),
    Center,
    Edge(WorkAreaDropEdge),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabGroup {
    id: TabGroupId,
    items: Vec<WorkItemId>,
    active_item: Option<WorkItemId>,
}

impl TabGroup {
    fn new(id: TabGroupId, items: Vec<WorkItemId>, active_item: Option<WorkItemId>) -> Self {
        Self {
            id,
            items,
            active_item,
        }
    }

    fn empty(id: TabGroupId) -> Self {
        Self::new(id, Vec::new(), None)
    }

    pub fn id(&self) -> TabGroupId {
        self.id
    }

    pub fn items(&self) -> &[WorkItemId] {
        &self.items
    }

    pub fn active_item(&self) -> Option<&WorkItemId> {
        self.active_item.as_ref()
    }

    fn select_fallback_after_removal(&mut self, removed_index: usize) {
        self.active_item = self
            .items
            .get(removed_index)
            .cloned()
            .or_else(|| self.items.last().cloned());
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum WorkAreaNode {
    Group(TabGroup),
    Split {
        id: WorkAreaSplitId,
        axis: WorkAreaSplitAxis,
        ratio: f32,
        first: Box<WorkAreaNode>,
        second: Box<WorkAreaNode>,
    },
}

/// A validated, entity-free representation of the editor work-area tree.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub(crate) struct WorkAreaSnapshot {
    root: WorkAreaSnapshotNode,
    active_group_id: u64,
    next_group_id: u64,
    next_split_id: u64,
    order_customized: bool,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum WorkAreaSnapshotNode {
    Group {
        id: u64,
        items: Vec<WorkItemId>,
        active_item: Option<WorkItemId>,
    },
    Split {
        id: u64,
        axis: WorkAreaSplitAxisSnapshot,
        ratio: f32,
        first: Box<WorkAreaSnapshotNode>,
        second: Box<WorkAreaSnapshotNode>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum WorkAreaSplitAxisSnapshot {
    Row,
    Column,
}

#[derive(Clone, Copy, Debug)]
struct WorkAreaRect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl WorkAreaRect {
    const ROOT: Self = Self {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    };

    fn split(self, axis: WorkAreaSplitAxis, ratio: f32) -> (Self, Self) {
        match axis {
            WorkAreaSplitAxis::Row => {
                let first_width = self.width * ratio;
                (
                    Self {
                        width: first_width,
                        ..self
                    },
                    Self {
                        x: self.x + first_width,
                        width: self.width - first_width,
                        ..self
                    },
                )
            }
            WorkAreaSplitAxis::Column => {
                let first_height = self.height * ratio;
                (
                    Self {
                        height: first_height,
                        ..self
                    },
                    Self {
                        y: self.y + first_height,
                        height: self.height - first_height,
                        ..self
                    },
                )
            }
        }
    }

    fn directional_score(self, candidate: Self, edge: WorkAreaDropEdge) -> Option<(f32, f32)> {
        let center_x = self.x + self.width / 2.0;
        let center_y = self.y + self.height / 2.0;
        let candidate_center_x = candidate.x + candidate.width / 2.0;
        let candidate_center_y = candidate.y + candidate.height / 2.0;
        match edge {
            WorkAreaDropEdge::Left if candidate_center_x < center_x => Some((
                (self.x - (candidate.x + candidate.width)).max(0.0),
                (candidate_center_y - center_y).abs(),
            )),
            WorkAreaDropEdge::Right if candidate_center_x > center_x => Some((
                (candidate.x - (self.x + self.width)).max(0.0),
                (candidate_center_y - center_y).abs(),
            )),
            WorkAreaDropEdge::Top if candidate_center_y < center_y => Some((
                (self.y - (candidate.y + candidate.height)).max(0.0),
                (candidate_center_x - center_x).abs(),
            )),
            WorkAreaDropEdge::Bottom if candidate_center_y > center_y => Some((
                (candidate.y - (self.y + self.height)).max(0.0),
                (candidate_center_x - center_x).abs(),
            )),
            _ => None,
        }
    }
}

impl WorkAreaNode {
    fn placeholder() -> Self {
        Self::Group(TabGroup::empty(PLACEHOLDER_GROUP_ID))
    }

    fn find_group(&self, target: TabGroupId) -> Option<&TabGroup> {
        match self {
            Self::Group(group) if group.id == target => Some(group),
            Self::Group(_) => None,
            Self::Split { first, second, .. } => first
                .find_group(target)
                .or_else(|| second.find_group(target)),
        }
    }

    fn find_group_mut(&mut self, target: TabGroupId) -> Option<&mut TabGroup> {
        match self {
            Self::Group(group) if group.id == target => Some(group),
            Self::Group(_) => None,
            Self::Split { first, second, .. } => {
                if let Some(group) = first.find_group_mut(target) {
                    Some(group)
                } else {
                    second.find_group_mut(target)
                }
            }
        }
    }

    fn group_rect(&self, target: TabGroupId, rect: WorkAreaRect) -> Option<WorkAreaRect> {
        match self {
            Self::Group(group) => (group.id == target).then_some(rect),
            Self::Split {
                axis,
                ratio,
                first,
                second,
                ..
            } => {
                let (first_rect, second_rect) = rect.split(*axis, *ratio);
                first
                    .group_rect(target, first_rect)
                    .or_else(|| second.group_rect(target, second_rect))
            }
        }
    }

    fn visit_group_rects(
        &self,
        rect: WorkAreaRect,
        visitor: &mut impl FnMut(TabGroupId, WorkAreaRect),
    ) {
        match self {
            Self::Group(group) => visitor(group.id, rect),
            Self::Split {
                axis,
                ratio,
                first,
                second,
                ..
            } => {
                let (first_rect, second_rect) = rect.split(*axis, *ratio);
                first.visit_group_rects(first_rect, visitor);
                second.visit_group_rects(second_rect, visitor);
            }
        }
    }

    fn adjacent_group_id(&self, target: TabGroupId, edge: WorkAreaDropEdge) -> Option<TabGroupId> {
        let target_rect = self.group_rect(target, WorkAreaRect::ROOT)?;
        let mut best = None::<(TabGroupId, f32, f32)>;
        self.visit_group_rects(WorkAreaRect::ROOT, &mut |group_id, rect| {
            if group_id == target {
                return;
            }
            let Some((distance, alignment)) = target_rect.directional_score(rect, edge) else {
                return;
            };
            let replace = best.is_none_or(|(_, best_distance, best_alignment)| {
                distance.total_cmp(&best_distance).is_lt()
                    || (distance.total_cmp(&best_distance).is_eq()
                        && alignment.total_cmp(&best_alignment).is_lt())
            });
            if replace {
                best = Some((group_id, distance, alignment));
            }
        });
        best.map(|(group_id, _, _)| group_id)
    }

    fn group_containing(&self, item: &WorkItemId) -> Option<TabGroupId> {
        match self {
            Self::Group(group) if group.items.contains(item) => Some(group.id),
            Self::Group(_) => None,
            Self::Split { first, second, .. } => first
                .group_containing(item)
                .or_else(|| second.group_containing(item)),
        }
    }

    fn first_group_id(&self) -> TabGroupId {
        match self {
            Self::Group(group) => group.id,
            Self::Split { first, .. } => first.first_group_id(),
        }
    }

    fn collect_items(&self, items: &mut Vec<WorkItemId>) {
        match self {
            Self::Group(group) => items.extend(group.items.iter().cloned()),
            Self::Split { first, second, .. } => {
                first.collect_items(items);
                second.collect_items(items);
            }
        }
    }

    fn retain_available(
        &mut self,
        available: &HashSet<WorkItemId>,
        seen: &mut HashSet<WorkItemId>,
    ) {
        match self {
            Self::Group(group) => {
                let active_index = group
                    .active_item
                    .as_ref()
                    .and_then(|active| group.items.iter().position(|item| item == active))
                    .unwrap_or_default();
                group
                    .items
                    .retain(|item| available.contains(item) && seen.insert(item.clone()));
                if group
                    .active_item
                    .as_ref()
                    .is_none_or(|active| !group.items.contains(active))
                {
                    group.select_fallback_after_removal(active_index);
                }
            }
            Self::Split { first, second, .. } => {
                first.retain_available(available, seen);
                second.retain_available(available, seen);
            }
        }
    }

    fn prune_empty(self) -> Option<Self> {
        match self {
            Self::Group(group) if group.items.is_empty() => None,
            Self::Group(group) => Some(Self::Group(group)),
            Self::Split {
                id,
                axis,
                ratio,
                first,
                second,
            } => match (first.prune_empty(), second.prune_empty()) {
                (Some(first), Some(second)) => Some(Self::Split {
                    id,
                    axis,
                    ratio,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (Some(remaining), None) | (None, Some(remaining)) => Some(remaining),
                (None, None) => None,
            },
        }
    }

    fn split_group(
        &mut self,
        target: TabGroupId,
        new_group: TabGroup,
        split_id: WorkAreaSplitId,
        edge: WorkAreaDropEdge,
    ) -> bool {
        if matches!(self, Self::Group(group) if group.id == target) {
            let existing = std::mem::replace(self, Self::placeholder());
            let new_group = Self::Group(new_group);
            let (first, second) = if edge.places_new_group_first() {
                (new_group, existing)
            } else {
                (existing, new_group)
            };
            *self = Self::Split {
                id: split_id,
                axis: edge.axis(),
                ratio: 0.5,
                first: Box::new(first),
                second: Box::new(second),
            };
            return true;
        }

        match self {
            Self::Group(_) => false,
            Self::Split { first, second, .. } => {
                if first.find_group(target).is_some() {
                    first.split_group(target, new_group, split_id, edge)
                } else {
                    second.split_group(target, new_group, split_id, edge)
                }
            }
        }
    }

    fn resize_split(&mut self, target: WorkAreaSplitId, delta: f32) -> Option<f32> {
        match self {
            Self::Group(_) => None,
            Self::Split {
                id,
                ratio,
                first,
                second,
                ..
            } => {
                if *id == target {
                    *ratio = (*ratio + delta).clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO);
                    Some(*ratio)
                } else {
                    first
                        .resize_split(target, delta)
                        .or_else(|| second.resize_split(target, delta))
                }
            }
        }
    }

    fn replace_item(&mut self, old: &WorkItemId, new: &WorkItemId) -> bool {
        match self {
            Self::Group(group) => {
                let mut replaced = false;
                for item in &mut group.items {
                    if item == old {
                        *item = new.clone();
                        replaced = true;
                    }
                }
                if group.active_item.as_ref() == Some(old) {
                    group.active_item = Some(new.clone());
                }
                replaced
            }
            Self::Split { first, second, .. } => {
                first.replace_item(old, new) || second.replace_item(old, new)
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct WorkAreaState {
    root: WorkAreaNode,
    active_group_id: TabGroupId,
    next_group_id: u64,
    next_split_id: u64,
    order_customized: bool,
}

impl WorkAreaState {
    pub(crate) fn new(initial_item: Option<WorkItemId>) -> Self {
        let group_id = TabGroupId(1);
        let items = initial_item.iter().cloned().collect();
        Self {
            root: WorkAreaNode::Group(TabGroup::new(group_id, items, initial_item)),
            active_group_id: group_id,
            next_group_id: 2,
            next_split_id: 1,
            order_customized: false,
        }
    }

    pub(crate) fn snapshot(&self) -> WorkAreaSnapshot {
        WorkAreaSnapshot {
            root: work_area_snapshot_node(&self.root),
            active_group_id: self.active_group_id.raw(),
            next_group_id: self.next_group_id,
            next_split_id: self.next_split_id,
            order_customized: self.order_customized,
        }
    }
    pub(crate) fn items(&self) -> Vec<WorkItemId> {
        let mut items = Vec::new();
        self.root.collect_items(&mut items);
        items
    }

    pub(crate) fn restore(snapshot: WorkAreaSnapshot) -> Result<Self, String> {
        let mut group_ids = HashSet::new();
        let mut split_ids = HashSet::new();
        let mut item_ids = HashSet::new();
        let mut largest_group_id = 0;
        let mut largest_split_id = 0;
        let root = work_area_node_from_snapshot(
            snapshot.root,
            &mut group_ids,
            &mut split_ids,
            &mut item_ids,
            &mut largest_group_id,
            &mut largest_split_id,
        )?;
        let active_group_id = TabGroupId(snapshot.active_group_id);
        if !group_ids.contains(&active_group_id) {
            return Err("active work-area group does not exist".to_string());
        }
        Ok(Self {
            root,
            active_group_id,
            next_group_id: snapshot
                .next_group_id
                .max(largest_group_id.saturating_add(1)),
            next_split_id: snapshot
                .next_split_id
                .max(largest_split_id.saturating_add(1)),
            order_customized: snapshot.order_customized,
        })
    }

    pub(crate) fn root(&self) -> &WorkAreaNode {
        &self.root
    }

    pub(crate) fn active_group_id(&self) -> TabGroupId {
        self.active_group_id
    }

    pub(crate) fn active_item(&self) -> Option<&WorkItemId> {
        self.root
            .find_group(self.active_group_id)
            .and_then(TabGroup::active_item)
    }

    pub(crate) fn active_group_items(&self) -> &[WorkItemId] {
        self.root
            .find_group(self.active_group_id)
            .map(TabGroup::items)
            .unwrap_or_default()
    }

    pub(crate) fn adjacent_group_id(&self, edge: WorkAreaDropEdge) -> Option<TabGroupId> {
        self.root.adjacent_group_id(self.active_group_id, edge)
    }

    pub(crate) fn group_items_containing(&self, item: &WorkItemId) -> Option<&[WorkItemId]> {
        let group_id = self.root.group_containing(item)?;
        self.root.find_group(group_id).map(TabGroup::items)
    }

    pub(crate) fn group_id_containing(&self, item: &WorkItemId) -> Option<TabGroupId> {
        self.root.group_containing(item)
    }

    pub(crate) fn ordered_items(
        &self,
        terminal_ids: &[String],
        file_ids: &[DocumentId],
    ) -> Vec<WorkItemId> {
        let available = available_items(terminal_ids, file_ids);
        if !self.order_customized {
            return available;
        }

        let available_set = available.iter().cloned().collect::<HashSet<_>>();
        let mut ordered = Vec::with_capacity(available.len());
        self.root.collect_items(&mut ordered);
        ordered.retain(|item| available_set.contains(item));
        for item in available {
            if !ordered.contains(&item) {
                ordered.push(item);
            }
        }
        ordered
    }

    pub(crate) fn reconcile(&mut self, terminal_ids: &[String], file_ids: &[DocumentId]) {
        let available = available_items(terminal_ids, file_ids);
        if !self.order_customized {
            let active = self
                .active_item()
                .filter(|active| available.contains(active))
                .cloned()
                .or_else(|| available.first().cloned());
            let group_id = self.active_group_id;
            self.root = WorkAreaNode::Group(TabGroup::new(group_id, available, active));
            return;
        }

        let available_set = available.iter().cloned().collect::<HashSet<_>>();
        let mut seen = HashSet::new();
        self.root.retain_available(&available_set, &mut seen);

        let missing = available
            .into_iter()
            .filter(|item| !seen.contains(item))
            .collect::<Vec<_>>();
        if let Some(group) = self.root.find_group_mut(self.active_group_id) {
            group.items.extend(missing);
            if group.active_item.is_none() {
                group.active_item = group.items.first().cloned();
            }
        }
        self.prune_empty_groups();
    }

    pub(crate) fn activate_group(&mut self, group_id: TabGroupId) -> Option<WorkItemId> {
        let active = self.root.find_group(group_id)?.active_item.clone();
        self.active_group_id = group_id;
        active
    }

    pub(crate) fn activate_item(&mut self, item: &WorkItemId) -> bool {
        let Some(group_id) = self.root.group_containing(item) else {
            return false;
        };
        let Some(group) = self.root.find_group_mut(group_id) else {
            return false;
        };
        group.active_item = Some(item.clone());
        self.active_group_id = group_id;
        true
    }

    pub(crate) fn append_item_to_active_group(&mut self, item: WorkItemId) {
        if self.root.group_containing(&item).is_some() {
            self.activate_item(&item);
            return;
        }
        if let Some(group) = self.root.find_group_mut(self.active_group_id) {
            group.items.push(item.clone());
            group.active_item = Some(item);
        }
    }

    pub(crate) fn move_item(
        &mut self,
        item: &WorkItemId,
        source_group: TabGroupId,
        target_group: TabGroupId,
        placement: WorkAreaDropPlacement,
    ) -> bool {
        let source_contains_item = self
            .root
            .find_group(source_group)
            .is_some_and(|group| group.items.contains(item));
        if !source_contains_item || self.root.find_group(target_group).is_none() {
            return false;
        }

        let changed = match placement {
            WorkAreaDropPlacement::TabIndex(to_index) => {
                self.move_item_to_tab_index(item, source_group, target_group, to_index)
            }
            WorkAreaDropPlacement::Center => {
                if source_group == target_group {
                    let was_active = self.active_item() == Some(item);
                    self.activate_item(item);
                    !was_active
                } else {
                    let target_index = self
                        .root
                        .find_group(target_group)
                        .map(|group| group.items.len())
                        .unwrap_or_default();
                    self.move_item_to_tab_index(item, source_group, target_group, target_index)
                }
            }
            WorkAreaDropPlacement::Edge(edge) => {
                self.split_group_with_item(item, source_group, target_group, edge)
            }
        };
        if changed {
            self.order_customized = true;
        }
        changed
    }

    pub(crate) fn remove_item(&mut self, item: &WorkItemId) -> bool {
        let Some(group_id) = self.root.group_containing(item) else {
            return false;
        };
        let Some(group) = self.root.find_group_mut(group_id) else {
            return false;
        };
        let Some(index) = group.items.iter().position(|candidate| candidate == item) else {
            return false;
        };
        let was_active = group.active_item.as_ref() == Some(item);
        group.items.remove(index);
        if was_active {
            group.select_fallback_after_removal(index);
        }
        self.prune_empty_groups();
        true
    }

    pub(crate) fn replace_item(&mut self, old: &WorkItemId, new: &WorkItemId) -> bool {
        self.root.replace_item(old, new)
    }

    pub(crate) fn resize_split(&mut self, split_id: WorkAreaSplitId, delta: f32) -> Option<f32> {
        self.root.resize_split(split_id, delta)
    }

    fn move_item_to_tab_index(
        &mut self,
        item: &WorkItemId,
        source_group: TabGroupId,
        target_group: TabGroupId,
        to_index: usize,
    ) -> bool {
        if source_group == target_group {
            let Some(group) = self.root.find_group_mut(source_group) else {
                return false;
            };
            let Some(from_index) = group.items.iter().position(|candidate| candidate == item)
            else {
                return false;
            };
            if from_index == to_index {
                return false;
            }
            let moved = group.items.remove(from_index);
            let destination = to_index.min(group.items.len());
            group.items.insert(destination, moved);
            return from_index != destination;
        }

        let Some((moved, source_was_active)) = self.take_item(source_group, item) else {
            return false;
        };
        let Some(target) = self.root.find_group_mut(target_group) else {
            return false;
        };
        let destination = to_index.min(target.items.len());
        target.items.insert(destination, moved.clone());
        target.active_item = Some(moved);
        self.active_group_id = target_group;
        if source_was_active {
            // The source fallback was selected by take_item before the target became active.
        }
        self.prune_empty_groups();
        true
    }

    fn split_group_with_item(
        &mut self,
        item: &WorkItemId,
        source_group: TabGroupId,
        target_group: TabGroupId,
        edge: WorkAreaDropEdge,
    ) -> bool {
        if source_group == target_group
            && self
                .root
                .find_group(source_group)
                .is_some_and(|group| group.items.len() == 1)
        {
            return false;
        }

        let Some((moved, _)) = self.take_item(source_group, item) else {
            return false;
        };
        if source_group != target_group {
            self.prune_empty_groups();
        }

        let new_group_id = self.allocate_group_id();
        let split_id = self.allocate_split_id();
        let new_group = TabGroup::new(new_group_id, vec![moved.clone()], Some(moved));
        let split = self
            .root
            .split_group(target_group, new_group, split_id, edge);
        debug_assert!(split, "validated target group disappeared during split");
        if !split {
            return false;
        }
        self.active_group_id = new_group_id;
        true
    }

    fn take_item(
        &mut self,
        source_group: TabGroupId,
        item: &WorkItemId,
    ) -> Option<(WorkItemId, bool)> {
        let source = self.root.find_group_mut(source_group)?;
        let index = source
            .items
            .iter()
            .position(|candidate| candidate == item)?;
        let was_active = source.active_item.as_ref() == Some(item);
        let moved = source.items.remove(index);
        if was_active {
            source.select_fallback_after_removal(index);
        }
        Some((moved, was_active))
    }

    fn prune_empty_groups(&mut self) {
        let root = std::mem::replace(&mut self.root, WorkAreaNode::placeholder());
        if let Some(root) = root.prune_empty() {
            self.root = root;
            if self.root.find_group(self.active_group_id).is_none() {
                self.active_group_id = self.root.first_group_id();
            }
            return;
        }

        let group_id = self.allocate_group_id();
        self.root = WorkAreaNode::Group(TabGroup::empty(group_id));
        self.active_group_id = group_id;
    }

    fn allocate_group_id(&mut self) -> TabGroupId {
        let id = TabGroupId(self.next_group_id);
        self.next_group_id = self.next_group_id.wrapping_add(1);
        id
    }

    fn allocate_split_id(&mut self) -> WorkAreaSplitId {
        let id = WorkAreaSplitId(self.next_split_id);
        self.next_split_id = self.next_split_id.wrapping_add(1);
        id
    }
}

fn work_area_snapshot_node(node: &WorkAreaNode) -> WorkAreaSnapshotNode {
    match node {
        WorkAreaNode::Group(group) => WorkAreaSnapshotNode::Group {
            id: group.id.raw(),
            items: group.items.clone(),
            active_item: group.active_item.clone(),
        },
        WorkAreaNode::Split {
            id,
            axis,
            ratio,
            first,
            second,
        } => WorkAreaSnapshotNode::Split {
            id: id.raw(),
            axis: match axis {
                WorkAreaSplitAxis::Row => WorkAreaSplitAxisSnapshot::Row,
                WorkAreaSplitAxis::Column => WorkAreaSplitAxisSnapshot::Column,
            },
            ratio: *ratio,
            first: Box::new(work_area_snapshot_node(first)),
            second: Box::new(work_area_snapshot_node(second)),
        },
    }
}

fn work_area_node_from_snapshot(
    snapshot: WorkAreaSnapshotNode,
    group_ids: &mut HashSet<TabGroupId>,
    split_ids: &mut HashSet<WorkAreaSplitId>,
    item_ids: &mut HashSet<WorkItemId>,
    largest_group_id: &mut u64,
    largest_split_id: &mut u64,
) -> Result<WorkAreaNode, String> {
    match snapshot {
        WorkAreaSnapshotNode::Group {
            id,
            items,
            active_item,
        } => {
            let group_id = TabGroupId(id);
            if !group_ids.insert(group_id) {
                return Err(format!("duplicate work-area group id: {id}"));
            }
            *largest_group_id = (*largest_group_id).max(id);
            if items.iter().any(|item| !item_ids.insert(item.clone())) {
                return Err("a work item appears in multiple work-area groups".to_string());
            }
            if active_item
                .as_ref()
                .is_some_and(|item| !items.contains(item))
            {
                return Err("work-area group active item is not in the group".to_string());
            }
            Ok(WorkAreaNode::Group(TabGroup::new(
                group_id,
                items,
                active_item,
            )))
        }
        WorkAreaSnapshotNode::Split {
            id,
            axis,
            ratio,
            first,
            second,
        } => {
            if !ratio.is_finite() || !(MIN_SPLIT_RATIO..=MAX_SPLIT_RATIO).contains(&ratio) {
                return Err(format!("work-area split ratio is invalid: {ratio}"));
            }
            let split_id = WorkAreaSplitId(id);
            if !split_ids.insert(split_id) {
                return Err(format!("duplicate work-area split id: {id}"));
            }
            *largest_split_id = (*largest_split_id).max(id);
            Ok(WorkAreaNode::Split {
                id: split_id,
                axis: match axis {
                    WorkAreaSplitAxisSnapshot::Row => WorkAreaSplitAxis::Row,
                    WorkAreaSplitAxisSnapshot::Column => WorkAreaSplitAxis::Column,
                },
                ratio,
                first: Box::new(work_area_node_from_snapshot(
                    *first,
                    group_ids,
                    split_ids,
                    item_ids,
                    largest_group_id,
                    largest_split_id,
                )?),
                second: Box::new(work_area_node_from_snapshot(
                    *second,
                    group_ids,
                    split_ids,
                    item_ids,
                    largest_group_id,
                    largest_split_id,
                )?),
            })
        }
    }
}

pub(super) fn available_items(terminal_ids: &[String], file_ids: &[DocumentId]) -> Vec<WorkItemId> {
    terminal_ids
        .iter()
        .cloned()
        .map(WorkItemId::Terminal)
        .chain(file_ids.iter().cloned().map(WorkItemId::File))
        .collect()
}
