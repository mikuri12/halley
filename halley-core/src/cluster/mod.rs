use self::tiling::{MasterStackLayout, Rect, layout_master_stack};
use crate::decay::DecayLevel;
use crate::field::{Field, Node, NodeId, NodeState, Vec2, Visibility};
use std::collections::HashMap;

pub mod layout;
pub mod stacking;
pub mod tiling;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClusterId(u64);

impl ClusterId {
    pub fn new(raw: u64) -> Self {
        Self(raw)
    }
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClusterMode {
    Expanded,
    Collapsed,
    Active,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClusterRemoveMemberOutcome {
    Removed,
    RequiresDissolve,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClusterCreateError {
    TooFewMembers,
    DuplicateMember,
    MissingNode(NodeId),
    AlreadyClustered(NodeId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClusterAddMemberError {
    MissingCluster,
    MissingNode(NodeId),
    AlreadyClustered(NodeId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClusterReorderError {
    MissingCluster,
    InvalidMembers,
    UnknownMember(NodeId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoveNodeClusterEffect {
    RemovedMember(ClusterId),
    DissolvedCluster(ClusterId),
    RemovedCore(ClusterId),
}

/// A cluster is a group of window nodes (members). Member nodes always live
/// in `Field.nodes`, in every mode - this record only tracks grouping/mode/
/// core bookkeeping, never node payloads. When collapsed, a `NodeState::Core`
/// `Field` node represents the cluster as a handle.
#[derive(Clone, Debug)]
pub struct Cluster {
    pub id: ClusterId,
    members: Vec<NodeId>,

    /// When collapsed, which Core node represents this cluster.
    pub core: Option<NodeId>,

    pub pinned: bool,

    pub mode: ClusterMode,
}

impl Cluster {
    fn new(id: ClusterId, members: Vec<NodeId>) -> Option<Self> {
        if has_duplicates(&members) {
            return None;
        }
        Some(Self {
            id,
            members,
            core: None,
            pinned: false,
            mode: ClusterMode::Expanded,
        })
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.members.contains(&id)
    }

    pub fn members(&self) -> &[NodeId] {
        &self.members
    }

    pub fn master(&self) -> Option<NodeId> {
        self.members.first().copied()
    }

    pub fn secondaries(&self) -> &[NodeId] {
        self.members.get(1..).unwrap_or_default()
    }

    pub fn visible_members(&self, max_stack: usize) -> &[NodeId] {
        if max_stack == 0 {
            &self.members
        } else {
            let limit = max_stack + 1;
            let end = self.members.len().min(limit);
            &self.members[..end]
        }
    }

    pub fn overflow_members(&self, max_stack: usize) -> &[NodeId] {
        if max_stack == 0 {
            &[]
        } else {
            let limit = max_stack + 1;
            if self.members.len() <= limit {
                &[]
            } else {
                &self.members[limit..]
            }
        }
    }

    pub fn core_node(&self) -> Option<NodeId> {
        self.core
    }

    pub fn is_collapsed(&self) -> bool {
        matches!(self.mode, ClusterMode::Collapsed)
    }

    pub fn is_active(&self) -> bool {
        matches!(self.mode, ClusterMode::Active)
    }

    pub fn set_collapsed(&mut self, collapsed: bool) {
        self.mode = if collapsed {
            ClusterMode::Collapsed
        } else {
            ClusterMode::Expanded
        };
    }

    fn enter_active(&mut self) {
        self.mode = ClusterMode::Active;
    }

    fn exit_active(&mut self) {
        self.mode = ClusterMode::Expanded;
    }

    pub fn workspace_layout(&self, bounds: Rect, max_stack: usize) -> MasterStackLayout {
        layout_master_stack(bounds, self.visible_members(max_stack))
    }

    fn add_member(&mut self, member: NodeId) -> bool {
        if self.members.contains(&member) {
            return false;
        }
        self.members.push(member);
        true
    }

    fn add_member_front(&mut self, member: NodeId) -> bool {
        if self.members.contains(&member) {
            return false;
        }
        self.members.insert(0, member);
        true
    }

    fn remove_member(&mut self, member: NodeId) -> Option<ClusterRemoveMemberOutcome> {
        if !self.members.contains(&member) {
            return None;
        }
        self.members.retain(|&id| id != member);
        Some(ClusterRemoveMemberOutcome::Removed)
    }

    fn remove_member_for_node_removal(&mut self, member: NodeId) -> bool {
        let before = self.members.len();
        self.members.retain(|&id| id != member);
        self.members.len() != before
    }

    fn reorder_members(&mut self, ordered_members: Vec<NodeId>) -> bool {
        if ordered_members.len() != self.members.len() || has_duplicates(&ordered_members) {
            return false;
        }

        let mut current = self.members.clone();
        let mut reordered = ordered_members.clone();
        current.sort_by_key(|id| id.as_u64());
        reordered.sort_by_key(|id| id.as_u64());
        if current != reordered {
            return false;
        }

        self.members = ordered_members;
        true
    }

    fn promote_member_to_master(&mut self, member: NodeId) -> bool {
        let Some(index) = self.members.iter().position(|&id| id == member) else {
            return false;
        };
        if index == 0 {
            return true;
        }
        self.members.remove(index);
        self.members.insert(0, member);
        true
    }

    fn swap_overflow_member_with_visible(
        &mut self,
        overflow_member: NodeId,
        visible_member: NodeId,
        max_stack: usize,
    ) -> bool {
        let Some(overflow_index) = self.members.iter().position(|&id| id == overflow_member) else {
            return false;
        };
        let Some(visible_index) = self.members.iter().position(|&id| id == visible_member) else {
            return false;
        };
        if max_stack > 0 {
            let limit = max_stack + 1;
            if overflow_index < limit || visible_index >= limit {
                return false;
            }
        } else {
            // unlimited; no overflow member can exist
            return false;
        }

        self.members[overflow_index] = visible_member;
        self.members[visible_index] = overflow_member;
        true
    }

    fn reorder_overflow_member(
        &mut self,
        member: NodeId,
        target_overflow_index: usize,
        max_stack: usize,
    ) -> bool {
        let Some(member_index) = self.members.iter().position(|&id| id == member) else {
            return false;
        };
        if max_stack == 0 {
            return false;
        }
        let limit = max_stack + 1;
        if member_index < limit {
            return false;
        }

        let overflow_len = self.members.len().saturating_sub(limit);
        if overflow_len <= 1 {
            return true;
        }

        let member = self.members.remove(member_index);
        let clamped_index = target_overflow_index.min(overflow_len - 1);
        let insert_index = (limit + clamped_index).min(self.members.len());
        self.members.insert(insert_index, member);
        true
    }
}

fn has_duplicates(members: &[NodeId]) -> bool {
    let mut seen = std::collections::HashSet::new();
    for member in members {
        if !seen.insert(*member) {
            return true;
        }
    }
    false
}

fn find_duplicate_member(members: &[NodeId]) -> Option<NodeId> {
    let mut seen = std::collections::HashSet::new();
    for member in members {
        if !seen.insert(*member) {
            return Some(*member);
        }
    }
    None
}

/// Owns all cluster bookkeeping for a single space. `World` holds one
/// `ClusterRegistry` per `SpaceId`, mirroring its one-`Field`-per-`SpaceId`
/// layout (see `world.rs`) - `NodeId`/`ClusterId` are only unique within a
/// single space, so a single flat registry spanning every space would risk
/// silently conflating ids across spaces.
///
/// `Field` itself has zero cluster awareness. This registry reads/mutates
/// node data through `Field`'s ordinary public API only (`node`, `node_mut`,
/// `set_state`, `spawn_surface`, `remove`, `carry`, ...) - the same way any
/// external consumer would, never through anything Field-internal.
#[derive(Debug, Default)]
pub struct ClusterRegistry {
    next_cluster: u64,
    clusters: HashMap<ClusterId, Cluster>,
    membership: HashMap<NodeId, ClusterId>,
}

impl ClusterRegistry {
    pub fn new() -> Self {
        Self {
            next_cluster: 1,
            clusters: HashMap::new(),
            membership: HashMap::new(),
        }
    }

    pub fn cluster(&self, id: ClusterId) -> Option<&Cluster> {
        self.clusters.get(&id)
    }

    pub fn cluster_mut(&mut self, id: ClusterId) -> Option<&mut Cluster> {
        self.clusters.get_mut(&id)
    }

    pub fn cluster_ids(&self) -> Vec<ClusterId> {
        let mut ids: Vec<_> = self.clusters.keys().copied().collect();
        ids.sort_by_key(|id| id.as_u64());
        ids
    }

    pub fn clusters_iter(&self) -> impl Iterator<Item = &Cluster> {
        self.clusters.values()
    }

    /// O(1) via the membership index (the old `Field`-owned version
    /// linear-scanned every cluster's member list for this).
    pub fn cluster_id_for_member(&self, member: NodeId) -> Option<ClusterId> {
        self.membership.get(&member).copied()
    }

    pub fn cluster_id_for_core(&self, core: NodeId) -> Option<ClusterId> {
        self.clusters
            .iter()
            .find_map(|(&cid, c)| (c.core == Some(core)).then_some(cid))
    }

    pub fn is_cluster_member(&self, id: NodeId) -> bool {
        self.membership.contains_key(&id)
    }

    pub fn is_active_cluster_member(&self, id: NodeId) -> bool {
        self.cluster_id_for_member(id)
            .is_some_and(|cid| self.clusters.get(&cid).is_some_and(|c| c.is_active()))
    }

    /// Remove a cluster record wholesale (needed for cross-space transfer -
    /// see `World::transfer_cluster_by_core`). Does not touch `Field`.
    pub fn remove_cluster_record(&mut self, id: ClusterId) -> Option<Cluster> {
        if let Some(cluster) = self.clusters.get(&id) {
            for &m in cluster.members() {
                self.membership.remove(&m);
            }
        }
        self.clusters.remove(&id)
    }

    /// Insert an existing cluster record wholesale (needed for cross-space
    /// transfer). Does not touch `Field`.
    pub fn insert_cluster_record(&mut self, cluster: Cluster) {
        self.next_cluster = self.next_cluster.max(cluster.id.as_u64() + 1);
        for &m in cluster.members() {
            self.membership.insert(m, cluster.id);
        }
        self.clusters.insert(cluster.id, cluster);
    }

    pub fn create_cluster(
        &mut self,
        field: &mut Field,
        members: Vec<NodeId>,
    ) -> Result<ClusterId, ClusterCreateError> {
        if find_duplicate_member(&members).is_some() {
            return Err(ClusterCreateError::DuplicateMember);
        }

        for &member in &members {
            if field.node(member).is_none() {
                return Err(ClusterCreateError::MissingNode(member));
            }
            if self.cluster_id_for_member(member).is_some() {
                return Err(ClusterCreateError::AlreadyClustered(member));
            }
        }

        let id = ClusterId::new(self.next_cluster);
        self.next_cluster += 1;

        let mut any_member_pinned = false;
        for &member in &members {
            if field.node(member).is_some_and(|n| n.pinned) {
                any_member_pinned = true;
                let _ = field.set_pinned(member, false);
            }
        }

        let mut cluster =
            Cluster::new(id, members.clone()).ok_or(ClusterCreateError::TooFewMembers)?;
        cluster.pinned = any_member_pinned;
        self.clusters.insert(id, cluster);
        for member in members {
            self.membership.insert(member, id);
        }
        Ok(id)
    }

    pub fn add_member_to_cluster(
        &mut self,
        field: &mut Field,
        id: ClusterId,
        member: NodeId,
    ) -> Result<(), ClusterAddMemberError> {
        if field.node(member).is_none() {
            return Err(ClusterAddMemberError::MissingNode(member));
        }
        if self.cluster_id_for_member(member).is_some() {
            return Err(ClusterAddMemberError::AlreadyClustered(member));
        }
        let is_pinned = field.node(member).is_some_and(|n| n.pinned);
        if is_pinned {
            let _ = field.set_pinned(member, false);
        }

        let Some(cluster) = self.clusters.get_mut(&id) else {
            return Err(ClusterAddMemberError::MissingCluster);
        };
        if !cluster.add_member(member) {
            return Err(ClusterAddMemberError::AlreadyClustered(member));
        }
        if is_pinned {
            cluster.pinned = true;
        }
        self.membership.insert(member, id);
        Ok(())
    }

    pub fn add_member_to_cluster_front(
        &mut self,
        field: &mut Field,
        id: ClusterId,
        member: NodeId,
    ) -> Result<(), ClusterAddMemberError> {
        if field.node(member).is_none() {
            return Err(ClusterAddMemberError::MissingNode(member));
        }
        if self.cluster_id_for_member(member).is_some() {
            return Err(ClusterAddMemberError::AlreadyClustered(member));
        }
        let is_pinned = field.node(member).is_some_and(|n| n.pinned);
        if is_pinned {
            let _ = field.set_pinned(member, false);
        }

        let Some(cluster) = self.clusters.get_mut(&id) else {
            return Err(ClusterAddMemberError::MissingCluster);
        };
        if !cluster.add_member_front(member) {
            return Err(ClusterAddMemberError::AlreadyClustered(member));
        }
        if is_pinned {
            cluster.pinned = true;
        }
        self.membership.insert(member, id);
        Ok(())
    }

    pub fn remove_member_from_cluster(
        &mut self,
        id: ClusterId,
        member: NodeId,
    ) -> Option<ClusterRemoveMemberOutcome> {
        let cluster = self.clusters.get_mut(&id)?;
        let outcome = cluster.remove_member(member)?;
        if matches!(outcome, ClusterRemoveMemberOutcome::Removed) {
            self.membership.remove(&member);
        }
        Some(outcome)
    }

    pub fn reorder_cluster_members(
        &mut self,
        id: ClusterId,
        ordered_members: Vec<NodeId>,
    ) -> Result<(), ClusterReorderError> {
        let Some(cluster) = self.clusters.get_mut(&id) else {
            return Err(ClusterReorderError::MissingCluster);
        };
        for &member in &ordered_members {
            if !cluster.contains(member) {
                return Err(ClusterReorderError::UnknownMember(member));
            }
        }
        if !cluster.reorder_members(ordered_members) {
            return Err(ClusterReorderError::InvalidMembers);
        }
        Ok(())
    }

    pub fn promote_cluster_member_to_master(
        &mut self,
        id: ClusterId,
        member: NodeId,
    ) -> Result<(), ClusterReorderError> {
        let Some(cluster) = self.clusters.get_mut(&id) else {
            return Err(ClusterReorderError::MissingCluster);
        };
        if !cluster.contains(member) {
            return Err(ClusterReorderError::UnknownMember(member));
        }
        if !cluster.promote_member_to_master(member) {
            return Err(ClusterReorderError::InvalidMembers);
        }
        Ok(())
    }

    pub fn swap_cluster_overflow_member_with_visible(
        &mut self,
        id: ClusterId,
        overflow_member: NodeId,
        visible_member: NodeId,
        max_stack: usize,
    ) -> bool {
        let Some(cluster) = self.clusters.get_mut(&id) else {
            return false;
        };
        cluster.swap_overflow_member_with_visible(overflow_member, visible_member, max_stack)
    }

    pub fn reorder_cluster_overflow_member(
        &mut self,
        id: ClusterId,
        member: NodeId,
        target_overflow_index: usize,
        max_stack: usize,
    ) -> bool {
        let Some(cluster) = self.clusters.get_mut(&id) else {
            return false;
        };
        cluster.reorder_overflow_member(member, target_overflow_index, max_stack)
    }

    pub fn cycle_cluster_stacking_members(
        &mut self,
        id: ClusterId,
        direction: self::layout::ClusterCycleDirection,
    ) -> Option<NodeId> {
        let cluster = self.clusters.get_mut(&id)?;
        self::stacking::cycle_stacking_members(&mut cluster.members, direction)
    }

    pub fn dissolve_cluster(&mut self, field: &mut Field, id: ClusterId) -> bool {
        self.finish_dissolve_cluster(field, id)
    }

    /// "Active" cluster mode is purely a mode marker here - unlike the old
    /// design, members never physically relocate on activate/deactivate,
    /// so there's no node bookkeeping to do beyond the mode transition.
    pub fn activate_cluster_workspace(&mut self, id: ClusterId) -> bool {
        let Some(cluster) = self.clusters.get_mut(&id) else {
            return false;
        };
        if cluster.is_active() {
            return true;
        }
        cluster.enter_active();
        true
    }

    pub fn deactivate_cluster_workspace(&mut self, id: ClusterId) -> bool {
        let Some(cluster) = self.clusters.get_mut(&id) else {
            return false;
        };
        if !cluster.is_active() {
            return true;
        }
        cluster.exit_active();
        true
    }

    /// Drag the cluster by its core handle.
    pub fn carry_cluster_by_core(&mut self, field: &mut Field, core: NodeId, to: Vec2) -> bool {
        if self.cluster_id_for_core(core).is_none() {
            return false;
        }
        field.carry(core, to)
    }

    /// Collapse the cluster into a Core node. Empty clusters use the origin
    /// as their fallback position; callers creating an empty cluster should
    /// prefer [`Self::collapse_cluster_at`].
    pub fn collapse_cluster(&mut self, field: &mut Field, id: ClusterId) -> Option<NodeId> {
        self.collapse_cluster_at(field, id, Vec2 { x: 0.0, y: 0.0 })
    }

    /// Collapse a cluster, using `fallback_position` when it has no members
    /// from which to compute a centroid.
    pub fn collapse_cluster_at(
        &mut self,
        field: &mut Field,
        id: ClusterId,
        fallback_position: Vec2,
    ) -> Option<NodeId> {
        let (members, already_collapsed, existing_core, was_active) = {
            let c = self.clusters.get(&id)?;
            (
                c.members().to_vec(),
                c.is_collapsed(),
                c.core,
                c.is_active(),
            )
        };

        if already_collapsed {
            return existing_core;
        }
        if was_active {
            self.deactivate_cluster_workspace(id);
        }

        for &m in &members {
            field.set_state(m, NodeState::Node);
            if let Some(n) = field.node_mut(m) {
                n.visibility.set(Visibility::HIDDEN_BY_CLUSTER, true);
            }
        }

        let mut sum = Vec2 { x: 0.0, y: 0.0 };
        for &m in &members {
            let n = field.node(m)?;
            sum.x += n.pos.x;
            sum.y += n.pos.y;
        }
        let core_pos = if members.is_empty() {
            fallback_position
        } else {
            let k = members.len() as f32;
            Vec2 {
                x: sum.x / k,
                y: sum.y / k,
            }
        };

        const CORE_SIZE: Vec2 = Vec2 { x: 48.0, y: 48.0 };

        // Reuse the existing core NodeId if the cluster had one and its
        // Field node is still present; recreate it at the same id if it was
        // somehow removed without going through this registry (defensive -
        // mirrors the old code's `entry().or_insert_with()`); otherwise
        // spawn a fresh node through Field's ordinary public API.
        let core_id = match existing_core {
            Some(cid) if field.node(cid).is_some() => cid,
            Some(cid) => {
                field.insert_existing(Node {
                    id: cid,
                    state: NodeState::Core,
                    label: format!("Cluster {}", id.as_u64()),
                    pos: core_pos,
                    intrinsic_size: CORE_SIZE,
                    footprint: CORE_SIZE,
                    resize_footprint: None,
                    pinned: false,
                    anchor: false,
                    is_landmark: false,
                    visibility: Visibility::NONE,
                    last_touch_ms: 0,
                    decay: DecayLevel::Hot,
                });
                cid
            }
            None => {
                let new_id =
                    field.spawn_surface(format!("Cluster {}", id.as_u64()), core_pos, CORE_SIZE);
                field.set_state(new_id, NodeState::Core);
                new_id
            }
        };

        if let Some(n) = field.node_mut(core_id) {
            n.pos = core_pos;
            n.state = NodeState::Core;
            n.footprint = CORE_SIZE;
            n.intrinsic_size = CORE_SIZE;
            n.visibility.clear(Visibility::HIDDEN_BY_CLUSTER);
            n.visibility.clear(Visibility::DETACHED);
        }

        let c = self.clusters.get_mut(&id)?;
        let pinned = c.pinned;
        c.set_collapsed(true);
        c.core = Some(core_id);

        if let Some(n) = field.node_mut(core_id) {
            n.pinned = pinned;
        }

        Some(core_id)
    }

    /// Expand the cluster. Note: this does not remove the core node (it
    /// never did, in the old design either) - callers that need the core
    /// gone after expanding are responsible for that themselves, same as
    /// before. In practice nothing in halley-wl calls this today; it's
    /// exercised only by this crate's own tests.
    pub fn expand_cluster(&mut self, field: &mut Field, id: ClusterId) -> bool {
        if self.cluster(id).is_some_and(|c| c.is_active()) {
            return true;
        }
        let members = {
            let c = match self.clusters.get(&id) {
                Some(c) => c,
                None => return false,
            };
            if !c.is_collapsed() {
                return true;
            }
            c.members().to_vec()
        };

        for m in members {
            field.set_state(m, NodeState::Active);
            if let Some(n) = field.node_mut(m) {
                n.visibility.set(Visibility::HIDDEN_BY_CLUSTER, false);
            }
        }

        if let Some(c) = self.clusters.get_mut(&id) {
            c.set_collapsed(false);
        }
        true
    }

    pub fn remove_node_cluster_safe(
        &mut self,
        field: &mut Field,
        id: NodeId,
    ) -> Option<(Node, Option<RemoveNodeClusterEffect>)> {
        if let Some(cid) = self.cluster_id_for_member(id) {
            let removed = field.remove(id)?;
            self.membership.remove(&id);
            let cluster = self.clusters.get_mut(&cid)?;
            cluster.remove_member_for_node_removal(id);
            return Some((removed, Some(RemoveNodeClusterEffect::RemovedMember(cid))));
        }

        if let Some(cid) = self.cluster_id_for_core(id) {
            let removed = field.remove(id)?;
            let was_collapsed = self.cluster(cid).is_some_and(|c| c.is_collapsed());
            if was_collapsed {
                let _ = self.expand_cluster(field, cid);
            }
            if let Some(cluster) = self.clusters.get_mut(&cid) {
                cluster.core = None;
                cluster.set_collapsed(false);
            }
            return Some((removed, Some(RemoveNodeClusterEffect::RemovedCore(cid))));
        }

        field.remove(id).map(|node| (node, None))
    }

    fn finish_dissolve_cluster(&mut self, field: &mut Field, id: ClusterId) -> bool {
        let Some(cluster) = self.clusters.remove(&id) else {
            return false;
        };

        for &member in cluster.members() {
            self.membership.remove(&member);
            let _ = field.set_state(member, NodeState::Active);
            if let Some(node) = field.node_mut(member) {
                node.visibility.clear(Visibility::HIDDEN_BY_CLUSTER);
                node.visibility.clear(Visibility::DETACHED);
            }
        }

        if let Some(core_id) = cluster.core {
            let _ = field.remove(core_id);
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(n: u64) -> Vec<NodeId> {
        (0..n).map(NodeId::new).collect()
    }

    #[test]
    fn visible_members_respects_max_stack() {
        let members = ids(10);
        let cluster = Cluster::new(ClusterId::new(1), members.clone()).unwrap();

        // max_stack 3 means 4 visible (1 master + 3 stack)
        assert_eq!(cluster.visible_members(3).len(), 4);
        assert_eq!(cluster.overflow_members(3).len(), 6);

        // max_stack 5 means 6 visible
        assert_eq!(cluster.visible_members(5).len(), 6);
        assert_eq!(cluster.overflow_members(5).len(), 4);
    }

    #[test]
    fn zero_max_stack_means_unlimited_visible() {
        let members = ids(10);
        let cluster = Cluster::new(ClusterId::new(1), members.clone()).unwrap();

        assert_eq!(cluster.visible_members(0).len(), 10);
        assert_eq!(cluster.overflow_members(0).len(), 0);
    }

    #[test]
    fn visible_members_capped_by_total_members() {
        let members = ids(3);
        let cluster = Cluster::new(ClusterId::new(1), members.clone()).unwrap();

        assert_eq!(cluster.visible_members(5).len(), 3);
        assert_eq!(cluster.overflow_members(5).len(), 0);
    }

    // --- ClusterRegistry tests (ported/adapted from the old field.rs
    // cluster tests, now written against ClusterRegistry + Field directly
    // instead of Field owning cluster state itself) ---

    #[test]
    fn cluster_create_allows_empty_members() {
        let mut f = Field::new();
        let mut r = ClusterRegistry::new();

        let cid = r.create_cluster(&mut f, Vec::new()).unwrap();
        assert!(r.cluster(cid).unwrap().members().is_empty());
        assert_eq!(r.cluster(cid).unwrap().master(), None);
        assert!(r.cluster(cid).unwrap().secondaries().is_empty());
    }

    #[test]
    fn empty_cluster_collapses_at_explicit_position() {
        let mut f = Field::new();
        let mut r = ClusterRegistry::new();
        let cid = r.create_cluster(&mut f, Vec::new()).unwrap();
        let position = Vec2 { x: 120.0, y: -45.0 };

        let core = r.collapse_cluster_at(&mut f, cid, position).unwrap();

        assert_eq!(f.node(core).unwrap().pos, position);
        assert!(r.cluster(cid).unwrap().is_collapsed());
    }

    #[test]
    fn cluster_create_rejects_missing_nodes() {
        let mut f = Field::new();
        let mut r = ClusterRegistry::new();
        let missing = NodeId::new(999);

        assert_eq!(
            r.create_cluster(&mut f, vec![missing]),
            Err(ClusterCreateError::MissingNode(missing))
        );
    }

    #[test]
    fn cluster_create_allows_singletons() {
        let mut f = Field::new();
        let mut r = ClusterRegistry::new();
        let a = f.spawn_surface("A", Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });

        let cid = r.create_cluster(&mut f, vec![a]).unwrap();
        assert_eq!(r.cluster(cid).unwrap().members(), &[a]);
    }

    #[test]
    fn cluster_create_rejects_duplicate_members() {
        let mut f = Field::new();
        let mut r = ClusterRegistry::new();
        let a = f.spawn_surface("A", Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });
        let b = f.spawn_surface("B", Vec2 { x: 10.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });

        assert_eq!(
            r.create_cluster(&mut f, vec![a, a, b]),
            Err(ClusterCreateError::DuplicateMember)
        );
    }

    #[test]
    fn collapse_cluster_creates_core_and_shrinks_members() {
        let mut f = Field::new();
        let mut r = ClusterRegistry::new();
        let a = f.spawn_surface("A", Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 100.0, y: 50.0 });
        let b = f.spawn_surface("B", Vec2 { x: 10.0, y: 0.0 }, Vec2 { x: 100.0, y: 50.0 });

        let cid = r.create_cluster(&mut f, vec![a, b]).unwrap();
        let core = r.collapse_cluster(&mut f, cid).unwrap();

        assert_eq!(f.node(a).unwrap().state, NodeState::Node);
        assert_eq!(f.node(b).unwrap().state, NodeState::Node);
        assert_eq!(f.node(a).unwrap().footprint, Vec2 { x: 24.0, y: 24.0 });

        assert!(
            f.node(a)
                .unwrap()
                .visibility
                .has(Visibility::HIDDEN_BY_CLUSTER)
        );
        assert!(
            f.node(b)
                .unwrap()
                .visibility
                .has(Visibility::HIDDEN_BY_CLUSTER)
        );
        assert!(!f.is_visible(a));
        assert!(!f.is_visible(b));

        let cn = f.node(core).unwrap();
        assert_eq!(cn.state, NodeState::Core);
        assert_eq!(cn.footprint, Vec2 { x: 48.0, y: 48.0 });
        assert!(f.is_visible(core));

        let c = r.cluster(cid).unwrap();
        assert!(c.is_collapsed());
        assert_eq!(c.core, Some(core));
    }

    #[test]
    fn expand_cluster_restores_members_active_and_visible() {
        let mut f = Field::new();
        let mut r = ClusterRegistry::new();
        let a = f.spawn_surface("A", Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 100.0, y: 50.0 });
        let b = f.spawn_surface("B", Vec2 { x: 10.0, y: 0.0 }, Vec2 { x: 100.0, y: 50.0 });

        let cid = r.create_cluster(&mut f, vec![a, b]).unwrap();
        r.collapse_cluster(&mut f, cid).unwrap();

        assert!(r.expand_cluster(&mut f, cid));

        assert_eq!(f.node(a).unwrap().state, NodeState::Active);
        assert_eq!(f.node(b).unwrap().state, NodeState::Active);
        assert_eq!(f.node(a).unwrap().footprint, Vec2 { x: 100.0, y: 50.0 });

        assert!(
            !f.node(a)
                .unwrap()
                .visibility
                .has(Visibility::HIDDEN_BY_CLUSTER)
        );
        assert!(f.is_visible(a));

        let c = r.cluster(cid).unwrap();
        assert!(!c.is_collapsed());
    }

    #[test]
    fn collapsing_twice_returns_same_core_without_duplicating() {
        let mut f = Field::new();
        let mut r = ClusterRegistry::new();
        let a = f.spawn_surface("A", Vec2 { x: -20.0, y: 0.0 }, Vec2 { x: 100.0, y: 50.0 });
        let b = f.spawn_surface("B", Vec2 { x: 20.0, y: 0.0 }, Vec2 { x: 100.0, y: 50.0 });

        let cid = r.create_cluster(&mut f, vec![a, b]).unwrap();
        let first_core = r.collapse_cluster(&mut f, cid).unwrap();
        let second_core = r.collapse_cluster(&mut f, cid).unwrap();

        assert_eq!(first_core, second_core);
        assert!(f.node(first_core).is_some());
    }

    #[test]
    fn active_cluster_members_stay_in_field() {
        let mut f = Field::new();
        let mut r = ClusterRegistry::new();
        let a = f.spawn_surface("A", Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });
        let b = f.spawn_surface("B", Vec2 { x: 10.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });

        let cid = r.create_cluster(&mut f, vec![a, b]).unwrap();
        assert!(r.activate_cluster_workspace(cid));

        // Unlike the old ActiveWorkspace design, active members are still
        // ordinary, visible Field nodes - nothing physically relocates.
        assert!(f.nodes().contains_key(&a));
        assert!(f.nodes().contains_key(&b));
        assert!(f.is_visible(a));
        assert!(f.is_visible(b));
        assert!(r.is_active_cluster_member(a));
        assert!(r.cluster(cid).unwrap().is_active());

        assert!(r.deactivate_cluster_workspace(cid));
        assert!(!r.cluster(cid).unwrap().is_active());
    }

    #[test]
    fn carry_respects_pinned() {
        let mut f = Field::new();
        let mut r = ClusterRegistry::new();
        let a = f.spawn_surface("A", Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });
        let b = f.spawn_surface("B", Vec2 { x: 10.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });

        let cid = r.create_cluster(&mut f, vec![a, b]).unwrap();
        let core = r.collapse_cluster(&mut f, cid).unwrap();

        assert!(f.set_pinned(core, true));
        assert!(!r.carry_cluster_by_core(&mut f, core, Vec2 { x: 999.0, y: 999.0 }));
    }

    #[test]
    fn creating_a_cluster_transfers_member_pin_to_the_core() {
        let mut f = Field::new();
        let mut r = ClusterRegistry::new();
        let a = f.spawn_surface("A", Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });
        let b = f.spawn_surface("B", Vec2 { x: 10.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });
        assert!(f.set_pinned(a, true));

        let cluster = r.create_cluster(&mut f, vec![a, b]).unwrap();
        let core = r.collapse_cluster(&mut f, cluster).unwrap();

        assert!(!f.node(a).unwrap().pinned);
        assert!(!f.node(b).unwrap().pinned);
        assert!(r.cluster(cluster).unwrap().pinned);
        assert!(f.node(core).unwrap().pinned);
    }

    #[test]
    fn remove_member_allows_two_member_cluster_to_become_singleton() {
        let mut f = Field::new();
        let mut r = ClusterRegistry::new();
        let a = f.spawn_surface("A", Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });
        let b = f.spawn_surface("B", Vec2 { x: 10.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });

        let cid = r.create_cluster(&mut f, vec![a, b]).unwrap();

        assert_eq!(
            r.remove_member_from_cluster(cid, a),
            Some(ClusterRemoveMemberOutcome::Removed)
        );
        let cluster = r.cluster(cid).unwrap();
        assert_eq!(cluster.members(), &[b]);
        assert_eq!(cluster.master(), Some(b));
        assert!(!r.is_cluster_member(a));
    }

    #[test]
    fn raw_member_removal_keeps_two_member_cluster_as_singleton() {
        let mut f = Field::new();
        let mut r = ClusterRegistry::new();
        let a = f.spawn_surface("A", Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });
        let b = f.spawn_surface("B", Vec2 { x: 10.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });

        let cid = r.create_cluster(&mut f, vec![a, b]).unwrap();
        let core = r.collapse_cluster(&mut f, cid).unwrap();

        let (_, effect) = r.remove_node_cluster_safe(&mut f, a).unwrap();

        assert_eq!(effect, Some(RemoveNodeClusterEffect::RemovedMember(cid)));
        assert_eq!(r.cluster(cid).unwrap().members(), &[b]);
        assert!(f.node(core).is_some());
        assert!(f.node(a).is_none());
        assert!(f.node(b).is_some());
        assert!(!f.is_visible(b));
    }

    #[test]
    fn removing_last_member_retains_empty_cluster_and_core() {
        let mut f = Field::new();
        let mut r = ClusterRegistry::new();
        let a = f.spawn_surface("A", Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });
        let b = f.spawn_surface("B", Vec2 { x: 10.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });

        let cid = r.create_cluster(&mut f, vec![a, b]).unwrap();
        let core = r.collapse_cluster(&mut f, cid).unwrap();

        let (_, effect_a) = r.remove_node_cluster_safe(&mut f, a).unwrap();
        assert_eq!(effect_a, Some(RemoveNodeClusterEffect::RemovedMember(cid)));

        let (_, effect_b) = r.remove_node_cluster_safe(&mut f, b).unwrap();
        assert_eq!(effect_b, Some(RemoveNodeClusterEffect::RemovedMember(cid)));

        let cluster = r.cluster(cid).expect("empty cluster retained");
        assert!(cluster.members().is_empty());
        assert_eq!(cluster.master(), None);
        assert!(cluster.is_collapsed());
        assert!(f.node(core).is_some());
    }

    #[test]
    fn removing_core_expands_and_clears_core_reference() {
        let mut f = Field::new();
        let mut r = ClusterRegistry::new();
        let a = f.spawn_surface("A", Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });
        let b = f.spawn_surface("B", Vec2 { x: 10.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });

        let cid = r.create_cluster(&mut f, vec![a, b]).unwrap();
        let core = r.collapse_cluster(&mut f, cid).unwrap();

        let (_, effect) = r.remove_node_cluster_safe(&mut f, core).unwrap();
        assert_eq!(effect, Some(RemoveNodeClusterEffect::RemovedCore(cid)));

        assert!(f.node(core).is_none());
        assert!(r.cluster(cid).is_some());
        assert_eq!(r.cluster(cid).unwrap().core, None);
        assert!(!r.cluster(cid).unwrap().is_collapsed());
        assert_eq!(f.node(a).unwrap().state, NodeState::Active);
        assert_eq!(f.node(b).unwrap().state, NodeState::Active);
    }

    #[test]
    fn promote_and_reorder_preserve_explicit_master_contract() {
        let mut f = Field::new();
        let mut r = ClusterRegistry::new();
        let a = f.spawn_surface("A", Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });
        let b = f.spawn_surface("B", Vec2 { x: 10.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });
        let c = f.spawn_surface("C", Vec2 { x: 20.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });

        let cid = r.create_cluster(&mut f, vec![a, b, c]).unwrap();
        r.promote_cluster_member_to_master(cid, c).unwrap();
        assert_eq!(r.cluster(cid).unwrap().members(), &[c, a, b]);
        assert_eq!(r.cluster(cid).unwrap().master(), Some(c));

        r.reorder_cluster_members(cid, vec![b, c, a]).unwrap();
        assert_eq!(r.cluster(cid).unwrap().members(), &[b, c, a]);
        assert_eq!(r.cluster(cid).unwrap().master(), Some(b));
        assert_eq!(r.cluster(cid).unwrap().secondaries(), &[c, a]);
    }

    #[test]
    fn cluster_workspace_layout_only_tiles_first_four_members() {
        let mut f = Field::new();
        let mut r = ClusterRegistry::new();
        let members = (0..6)
            .map(|index| {
                f.spawn_surface(
                    format!("N{}", index),
                    Vec2 {
                        x: index as f32 * 10.0,
                        y: 0.0,
                    },
                    Vec2 { x: 10.0, y: 10.0 },
                )
            })
            .collect::<Vec<_>>();

        let cid = r.create_cluster(&mut f, members.clone()).unwrap();
        let cluster = r.cluster(cid).unwrap();
        let layout = cluster.workspace_layout(
            self::tiling::Rect {
                x: 0.0,
                y: 0.0,
                w: 1000.0,
                h: 600.0,
            },
            3,
        );

        assert_eq!(cluster.visible_members(3), &members[..4]);
        assert_eq!(cluster.overflow_members(3), &members[4..]);
        assert_eq!(layout.tiles.len(), 4);
        assert!(
            layout
                .tiles
                .iter()
                .all(|tile| members[..4].contains(&tile.id))
        );
    }
}
