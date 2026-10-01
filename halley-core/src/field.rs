use crate::decay::DecayLevel;
use crate::viewport::Viewport;
use crate::visual::{NodeVisual, VisualParams, build_visuals, build_visuals_in_view};

use std::collections::HashMap;

/// A stable identity for anything that exists in the Field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(u64);

impl NodeId {
    pub fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// 2D point / vector in Field coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

/// Axis-aligned rectangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub min: Vec2,
    pub max: Vec2,
}

impl Rect {
    pub fn width(self) -> f32 {
        self.max.x - self.min.x
    }

    pub fn height(self) -> f32 {
        self.max.y - self.min.y
    }

    pub fn contains(self, p: Vec2) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y
    }

    pub fn intersects(self, other: Rect) -> bool {
        self.min.x <= other.max.x
            && self.max.x >= other.min.x
            && self.min.y <= other.max.y
            && self.max.y >= other.min.y
    }
}

/// Semantic visibility flags.
/// This is NOT rendering; it's "experience-layer existence":
/// - hidden nodes should be skipped by focus/nav/bearings/in_view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Visibility(u8);

impl Visibility {
    pub const NONE: Self = Self(0);

    /// Hidden because user/system explicitly hid it.
    pub const HIDDEN_EXPLICIT: Self = Self(1 << 0);

    /// Hidden because its cluster is collapsed.
    pub const HIDDEN_BY_CLUSTER: Self = Self(1 << 1);

    /// Node exists in storage, but is currently detached from the experience layer.
    pub const DETACHED: Self = Self(1 << 2);

    pub fn is_hidden(self) -> bool {
        (self.0 & (Self::HIDDEN_EXPLICIT.0 | Self::HIDDEN_BY_CLUSTER.0 | Self::DETACHED.0)) != 0
    }

    pub fn has(self, flag: Self) -> bool {
        (self.0 & flag.0) != 0
    }

    pub fn set(&mut self, flag: Self, on: bool) {
        if on {
            self.0 |= flag.0;
        } else {
            self.0 &= !flag.0;
        }
    }

    pub fn clear(&mut self, flag: Self) {
        self.0 &= !flag.0;
    }
}

/// Representation state.
///
/// `Core` (the collapsed-cluster handle representation) is currently dead:
/// nothing constructs it, since `Field` no longer owns cluster placeholders
/// at all (see the `Field` doc comment) — the cluster module tracks its own
/// synthetic core entry externally rather than storing it as a `Node` here.
/// Left in place rather than removed pre-emptively; revisit once step 1f/1h
/// land and it's clear whether anything still needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeState {
    Active,
    Drifting,
    Node, // dot with label
    Core,
}

/// A Node is the universal "thing" that exists in the Field.
#[derive(Clone, Debug)]
pub struct Node {
    pub id: NodeId,
    pub state: NodeState,

    pub label: String,

    /// Center position in Field coordinates.
    pub pos: Vec2,

    pub intrinsic_size: Vec2, // "real" size for Active
    pub footprint: Vec2,      // spatial occupancy right now
    pub resize_footprint: Option<Vec2>,

    /// Pinned in place (movement constraint). This was previously called `anchored`.
    pub pinned: bool,

    /// Routing marker: important node that should always be surfaced in navigation
    /// (Bearings/Lift). Does NOT bypass visibility rules.
    pub anchor: bool,

    /// Explicit "treat this node as a fixed obstacle other nodes must avoid"
    /// marker, read by the overlap-avoidance engine. This used to be inferred
    /// from `NodeState` (`NodeState::Node | Core` implied landmark) — that
    /// conflated representation state with placement-engine semantics, so
    /// it's now its own field, set independently by whichever policy (decay
    /// going Cold, a collapsed cluster core, ...) decides a node should
    /// behave this way. Nothing sets it yet; wiring it up is later work
    /// (steps 1f/1h), this step only introduces the representation.
    pub is_landmark: bool,

    /// Semantic visibility / participation flags.
    pub visibility: Visibility,

    pub last_touch_ms: u64,
    pub decay: DecayLevel,
}

impl Node {
    /// The footprint a node occupies when shrunk to its "dot with label"
    /// representation (`NodeState::Node`). Named explicitly rather than
    /// left as a magic constant inline in `set_state`, so anything that
    /// needs to know a node's collapsed size (e.g. the cluster module
    /// laying out collapsed members) can ask for it directly instead of
    /// duplicating the number.
    pub fn collapsed_footprint(&self) -> Vec2 {
        Vec2 { x: 24.0, y: 24.0 }
    }
}

/// The infinite 2D space containing all Nodes.
///
/// `Field` is deliberately cluster-blind: it knows nothing about
/// `ClusterId`/`Cluster` membership. Cluster bookkeeping lives one level up,
/// on `World` (see `world.rs`), since a cluster needs to be able to move
/// between `Field`s and a single `Field` can't own something that outlives it.
pub struct Field {
    next_node: u64,
    nodes: HashMap<NodeId, Node>,
}

impl Field {
    fn make_surface_node(id: NodeId, label: String, pos: Vec2, size: Vec2) -> Node {
        Node {
            id,
            state: NodeState::Active,
            label,
            pos,
            intrinsic_size: size,
            footprint: size,
            resize_footprint: None,
            pinned: false,
            anchor: false,
            is_landmark: false,
            visibility: Visibility::NONE,
            last_touch_ms: 0,
            decay: DecayLevel::Hot,
        }
    }

    pub fn new() -> Self {
        Self {
            next_node: 1,
            nodes: HashMap::new(),
        }
    }

    pub fn nodes(&self) -> &HashMap<NodeId, Node> {
        &self.nodes
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&id)
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(&id)
    }

    /// Spawn a basic Surface node.
    pub fn spawn_surface(&mut self, label: impl Into<String>, pos: Vec2, size: Vec2) -> NodeId {
        let id = NodeId(self.next_node);
        self.next_node += 1;

        let node = Self::make_surface_node(id, label.into(), pos, size);

        self.nodes.insert(id, node);
        id
    }

    /// Remove a node from the Field.
    pub fn remove(&mut self, id: NodeId) -> Option<Node> {
        self.nodes.remove(&id)
    }

    pub fn node_ids_all(&self) -> Vec<NodeId> {
        self.nodes.keys().copied().collect()
    }

    /// Set/unset movement pinning.
    pub fn set_pinned(&mut self, id: NodeId, on: bool) -> bool {
        let Some(n) = self.node_mut(id) else {
            return false;
        };
        n.pinned = on;
        true
    }

    /// Back-compat alias: previously `anchor()` meant "pinned in place".
    /// Prefer `set_pinned()`. (We keep this to avoid churn in other modules.)
    pub fn anchor(&mut self, id: NodeId, on: bool) -> bool {
        self.set_pinned(id, on)
    }

    /// Set/unset routing anchor marker.
    pub fn set_anchor(&mut self, id: NodeId, on: bool) -> bool {
        let Some(n) = self.node_mut(id) else {
            return false;
        };
        n.anchor = on;
        true
    }

    pub fn is_anchor(&self, id: NodeId) -> bool {
        self.node(id).is_some_and(|n| n.anchor)
    }

    /// Set/unset the landmark marker (see `Node::is_landmark`'s doc comment).
    pub fn set_landmark(&mut self, id: NodeId, on: bool) -> bool {
        let Some(n) = self.node_mut(id) else {
            return false;
        };
        n.is_landmark = on;
        true
    }

    pub fn is_landmark(&self, id: NodeId) -> bool {
        self.node(id).is_some_and(|n| n.is_landmark)
    }

    /// Return all experience-visible anchors (stable order).
    pub fn anchors(&self) -> Vec<NodeId> {
        let mut out: Vec<NodeId> = self
            .nodes
            .iter()
            .filter_map(|(&id, n)| (self.is_visible(id) && n.anchor).then_some(id))
            .collect();
        out.sort_by_key(|id| id.as_u64());
        out
    }

    /// Carry a node to a new position (respects pinning).
    pub fn carry(&mut self, id: NodeId, to: Vec2) -> bool {
        let Some(n) = self.node_mut(id) else {
            return false;
        };
        if n.pinned {
            return false;
        }
        n.pos = to;
        true
    }

    /// Axis-aligned bounds in Field space.
    pub fn bounds(&self, id: NodeId) -> Option<Rect> {
        let n = self.node(id)?;
        Some(Self::bounds_for_node(n))
    }

    fn bounds_for_node(n: &Node) -> Rect {
        let half = Vec2 {
            x: n.footprint.x * 0.5,
            y: n.footprint.y * 0.5,
        };
        Rect {
            min: Vec2 {
                x: n.pos.x - half.x,
                y: n.pos.y - half.y,
            },
            max: Vec2 {
                x: n.pos.x + half.x,
                y: n.pos.y + half.y,
            },
        }
    }

    /// Return nodes that intersect the view rect AND are experience-visible.
    pub fn in_view(&self, view: Rect) -> Vec<NodeId> {
        self.nodes
            .keys()
            .copied()
            .filter(|&id| self.is_visible(id))
            .filter(|&id| self.bounds(id).is_some_and(|b| b.intersects(view)))
            .collect()
    }

    /// Return all nodes that intersect the view rect (includes hidden nodes).
    pub fn in_view_all(&self, view: Rect) -> Vec<NodeId> {
        self.nodes
            .keys()
            .copied()
            .filter(|&id| self.bounds(id).is_some_and(|b| b.intersects(view)))
            .collect()
    }

    /// True iff the node exists and is not hidden by any visibility reason.
    pub fn is_visible(&self, id: NodeId) -> bool {
        self.node(id).is_some_and(|n| !n.visibility.is_hidden())
    }

    /// Explicit hide/show (does not touch cluster-hidden).
    pub fn set_hidden(&mut self, id: NodeId, on: bool) -> bool {
        let Some(n) = self.node_mut(id) else {
            return false;
        };
        n.visibility.set(Visibility::HIDDEN_EXPLICIT, on);
        true
    }

    /// Detach/attach.
    pub fn set_detached(&mut self, id: NodeId, on: bool) -> bool {
        let Some(n) = self.node_mut(id) else {
            return false;
        };
        n.visibility.set(Visibility::DETACHED, on);
        true
    }

    /// Record interaction with a node.
    pub fn touch(&mut self, id: NodeId, now_ms: u64) -> bool {
        let Some(n) = self.node_mut(id) else {
            return false;
        };
        n.last_touch_ms = now_ms;
        n.decay = DecayLevel::Hot;

        // Core is a handle; it doesn't switch representation via touch.
        if n.state != NodeState::Core {
            n.state = NodeState::Active;
            n.footprint = n.resize_footprint.unwrap_or(n.intrinsic_size);
        }

        true
    }

    /// Apply a decay level to a node by mapping it to representation state.
    pub fn set_decay_level(&mut self, id: NodeId, level: DecayLevel) -> bool {
        let Some(n) = self.node(id) else {
            return false;
        };

        // Core is a handle; it doesn't decay away.
        if n.state == NodeState::Core {
            return true;
        }

        let state = match level {
            DecayLevel::Hot => NodeState::Active,
            DecayLevel::Cold => NodeState::Node,
        };

        if let Some(nm) = self.node_mut(id) {
            nm.decay = level;
        }
        self.set_state(id, state)
    }

    pub fn set_state(&mut self, id: NodeId, state: NodeState) -> bool {
        const CORE: Vec2 = Vec2 { x: 48.0, y: 48.0 };

        let Some(n) = self.node_mut(id) else {
            return false;
        };

        n.state = state.clone();
        n.footprint = match state {
            NodeState::Active => n.resize_footprint.unwrap_or(n.intrinsic_size),
            NodeState::Drifting => n.footprint,
            NodeState::Node => n.collapsed_footprint(),
            NodeState::Core => CORE,
        };

        true
    }

    pub fn set_resize_footprint(&mut self, id: NodeId, size: Option<Vec2>) -> bool {
        let Some(n) = self.nodes.get_mut(&id) else {
            return false;
        };

        n.resize_footprint = size;
        if matches!(n.state, NodeState::Active) {
            n.footprint = n.resize_footprint.unwrap_or(n.intrinsic_size);
        }

        true
    }

    pub fn sync_active_footprint_to_intrinsic(&mut self, id: NodeId) -> bool {
        let Some(n) = self.nodes.get_mut(&id) else {
            return false;
        };
        n.resize_footprint = None;
        if matches!(n.state, NodeState::Active) {
            n.footprint = n.intrinsic_size;
        }
        true
    }

    /// Canonical visuals feed: for full behavior (including cluster-core
    /// badge counts), call `build_visuals()` directly with a
    /// `ClusterRegistry` - these convenience wrappers only have `&Field`, so
    /// they can't provide one and always omit the badge count.
    pub fn visuals_visible(&self) -> Vec<NodeVisual> {
        let vp = Viewport::new(Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 0.0, y: 0.0 });
        build_visuals(self, &vp, VisualParams::default(), None)
    }

    pub fn visuals_in_view(&self, view: Rect) -> Vec<NodeVisual> {
        let vp = Viewport::new(Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 0.0, y: 0.0 });
        build_visuals_in_view(self, &vp, view, VisualParams::default(), None)
    }

    pub fn insert_existing(&mut self, node: Node) {
        // keep ids stable; bump next_node if needed so future spawns don't collide
        self.next_node = self.next_node.max(node.id.as_u64() + 1);
        self.nodes.insert(node.id, node);
    }
}

impl Default for Field {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Cluster-related tests (collapse/expand/create/active-workspace/etc.)
    // were removed here in step 1a, since `Field` no longer knows about
    // clusters at all. They come back, redesigned against the new
    // `World`-owned cluster registry, in step 1h.

    #[test]
    fn landmark_marker_is_independent_of_state_and_defaults_off() {
        let mut f = Field::new();
        let id = f.spawn_surface("A", Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });

        assert!(!f.is_landmark(id));

        assert!(f.set_landmark(id, true));
        assert!(f.is_landmark(id));
        // Setting the landmark marker doesn't touch representation state.
        assert_eq!(f.node(id).unwrap().state, NodeState::Active);

        assert!(f.set_landmark(id, false));
        assert!(!f.is_landmark(id));
    }

    #[test]
    fn carry_respects_pinned() {
        let mut f = Field::new();
        let id = f.spawn_surface("A", Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });

        assert!(f.carry(id, Vec2 { x: 5.0, y: 5.0 }));
        assert_eq!(f.node(id).unwrap().pos, Vec2 { x: 5.0, y: 5.0 });

        assert!(f.set_pinned(id, true));
        assert!(!f.carry(id, Vec2 { x: 9.0, y: 9.0 }));
        assert_eq!(f.node(id).unwrap().pos, Vec2 { x: 5.0, y: 5.0 });
    }

    #[test]
    fn in_view_finds_intersections() {
        let mut f = Field::new();
        let a = f.spawn_surface("A", Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });
        let _b = f.spawn_surface("B", Vec2 { x: 100.0, y: 100.0 }, Vec2 { x: 10.0, y: 10.0 });

        let view = Rect {
            min: Vec2 { x: -20.0, y: -20.0 },
            max: Vec2 { x: 20.0, y: 20.0 },
        };

        let ids = f.in_view_all(view);
        assert_eq!(ids, vec![a]);
    }

    #[test]
    fn in_view_skips_hidden_nodes() {
        let mut f = Field::new();
        let a = f.spawn_surface("A", Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });

        assert!(f.set_hidden(a, true));

        let view = Rect {
            min: Vec2 { x: -20.0, y: -20.0 },
            max: Vec2 { x: 20.0, y: 20.0 },
        };

        let ids = f.in_view(view);
        assert!(ids.is_empty());
        assert!(!f.is_visible(a));
    }

    #[test]
    fn set_state_changes_footprint() {
        let mut f = Field::new();
        let id = f.spawn_surface("A", Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 100.0, y: 50.0 });

        assert_eq!(f.node(id).unwrap().footprint, Vec2 { x: 100.0, y: 50.0 });

        assert!(f.set_state(id, NodeState::Node));
        assert_eq!(f.node(id).unwrap().footprint, Vec2 { x: 24.0, y: 24.0 });

        assert!(f.set_state(id, NodeState::Active));
        assert_eq!(f.node(id).unwrap().footprint, Vec2 { x: 100.0, y: 50.0 });
    }

    #[test]
    fn touch_sets_last_touch_and_wakes_node() {
        let mut f = Field::new();
        let id = f.spawn_surface("A", Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });

        assert!(f.set_decay_level(id, DecayLevel::Cold));
        assert_eq!(f.node(id).unwrap().state, NodeState::Node);

        assert!(f.touch(id, 1234));
        let n = f.node(id).unwrap();
        assert_eq!(n.last_touch_ms, 1234);
        assert_eq!(n.decay, DecayLevel::Hot);
        assert_eq!(n.state, NodeState::Active);
    }

    #[test]
    fn set_decay_level_maps_to_representation_state() {
        let mut f = Field::new();
        let id = f.spawn_surface("A", Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });

        assert!(f.set_decay_level(id, DecayLevel::Hot));
        assert_eq!(f.node(id).unwrap().decay, DecayLevel::Hot);
        assert_eq!(f.node(id).unwrap().state, NodeState::Active);

        assert!(f.set_decay_level(id, DecayLevel::Cold));
        assert_eq!(f.node(id).unwrap().decay, DecayLevel::Cold);
        assert_eq!(f.node(id).unwrap().state, NodeState::Node);
    }

    #[test]
    fn visuals_skip_hidden_nodes() {
        let mut f = Field::new();
        let a = f.spawn_surface("A", Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });
        let b = f.spawn_surface("B", Vec2 { x: 50.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });

        assert!(f.set_hidden(b, true));

        let vis = f.visuals_visible();
        assert_eq!(vis.len(), 1);
        assert_eq!(vis[0].id, a);
    }
}
