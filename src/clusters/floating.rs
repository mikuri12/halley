use std::collections::HashMap;

use halley_core::field::NodeId;
use smithay::utils::{Logical, Point, Rectangle, Size};

#[derive(Clone, Debug)]
struct MemberFloat {
    output: String,
    rect: Rectangle<i32, Logical>,
    active: bool,
}

/// Owns cluster-member floating state independently from rule-admitted Field
/// windows. A member keeps its registry position while active here, so tiling
/// can exclude it without losing the slot it should return to.
#[derive(Default)]
pub(super) struct ClusterFloatingState {
    members: HashMap<NodeId, MemberFloat>,
}

impl ClusterFloatingState {
    pub(super) fn is_floating(&self, member: NodeId) -> bool {
        self.members.get(&member).is_some_and(|state| state.active)
    }

    pub(super) fn rect(&self, member: NodeId) -> Option<Rectangle<i32, Logical>> {
        self.members
            .get(&member)
            .filter(|state| state.active)
            .map(|state| state.rect)
    }

    pub(super) fn output(&self, member: NodeId) -> Option<&str> {
        self.members
            .get(&member)
            .filter(|state| state.active)
            .map(|state| state.output.as_str())
    }

    pub(super) fn rect_on(&self, member: NodeId, output: &str) -> Option<Rectangle<i32, Logical>> {
        self.members
            .get(&member)
            .filter(|state| state.active && state.output == output)
            .map(|state| state.rect)
    }

    pub(super) fn placements_on(&self, output: &str) -> Vec<(NodeId, Rectangle<i32, Logical>)> {
        self.members
            .iter()
            .filter_map(|(member, state)| {
                (state.active && state.output == output).then_some((*member, state.rect))
            })
            .collect()
    }

    pub(super) fn float(
        &mut self,
        member: NodeId,
        output: &str,
        fallback: Rectangle<i32, Logical>,
        work_area: Rectangle<i32, Logical>,
    ) -> Rectangle<i32, Logical> {
        let state = self.members.entry(member).or_insert(MemberFloat {
            output: output.to_string(),
            rect: fallback,
            active: false,
        });
        if state.output == output {
            state.rect = clamp_to_work_area(state.rect, work_area);
        }
        state.active = true;
        state.rect
    }

    pub(super) fn tile(&mut self, member: NodeId) -> Option<Rectangle<i32, Logical>> {
        let state = self.members.get_mut(&member)?;
        if !state.active {
            return None;
        }
        state.active = false;
        Some(state.rect)
    }

    pub(super) fn update(
        &mut self,
        member: NodeId,
        output: &str,
        rect: Rectangle<i32, Logical>,
        work_area: Rectangle<i32, Logical>,
    ) -> bool {
        let Some(state) = self.members.get_mut(&member).filter(|state| state.active) else {
            return false;
        };
        let rect = clamp_to_work_area(rect, work_area);
        if state.output == output && state.rect == rect {
            return false;
        }
        state.output = output.to_string();
        state.rect = rect;
        true
    }

    pub(super) fn remove(&mut self, member: NodeId) {
        self.members.remove(&member);
    }
}

fn clamp_to_work_area(
    rect: Rectangle<i32, Logical>,
    work_area: Rectangle<i32, Logical>,
) -> Rectangle<i32, Logical> {
    let size = Size::from((
        rect.size.w.max(1).min(work_area.size.w.max(1)),
        rect.size.h.max(1).min(work_area.size.h.max(1)),
    ));
    let max_x = work_area.loc.x + work_area.size.w - size.w;
    let max_y = work_area.loc.y + work_area.size.h - size.h;
    Rectangle::new(
        Point::from((
            rect.loc.x.clamp(work_area.loc.x, max_x),
            rect.loc.y.clamp(work_area.loc.y, max_y),
        )),
        size,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembered_geometry_survives_retiling() {
        let member = NodeId::new(7);
        let work_area = Rectangle::new((0, 0).into(), (1_000, 700).into());
        let mut state = ClusterFloatingState::default();
        let first = Rectangle::new((120, 80).into(), (500, 400).into());

        assert_eq!(state.float(member, "DP-1", first, work_area), first);
        let moved = Rectangle::new((320, 220).into(), (420, 310).into());
        assert!(state.update(member, "DP-1", moved, work_area));
        assert_eq!(state.tile(member), Some(moved));

        let unrelated_tile = Rectangle::new((0, 0).into(), (1_000, 700).into());
        assert_eq!(
            state.float(member, "DP-1", unrelated_tile, work_area),
            moved
        );
    }

    #[test]
    fn floating_geometry_stays_reachable() {
        let member = NodeId::new(8);
        let work_area = Rectangle::new((10, 20).into(), (800, 600).into());
        let mut state = ClusterFloatingState::default();
        let oversized = Rectangle::new((-500, 900).into(), (1_200, 900).into());

        assert_eq!(
            state.float(member, "DP-1", oversized, work_area),
            Rectangle::new((10, 20).into(), (800, 600).into())
        );
    }

    #[test]
    fn output_and_geometry_survive_retiling_for_the_cluster_lifetime() {
        let member = NodeId::new(9);
        let work_area = Rectangle::new((0, 0).into(), (1_000, 700).into());
        let mut state = ClusterFloatingState::default();
        let initial = Rectangle::new((100, 80).into(), (500, 400).into());
        let external = Rectangle::new((40, 60).into(), (500, 400).into());

        state.float(member, "DP-1", initial, work_area);
        assert!(state.update(member, "DP-2", external, work_area));
        assert_eq!(state.output(member), Some("DP-2"));
        assert_eq!(state.tile(member), Some(external));

        state.float(member, "DP-1", initial, work_area);
        assert_eq!(state.output(member), Some("DP-2"));
        assert_eq!(state.rect_on(member, "DP-2"), Some(external));
    }
}
