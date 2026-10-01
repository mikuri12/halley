use std::time::Duration;

use halley_core::cluster::ClusterId;
use halley_core::field::{Field, NodeId, NodeState};

use super::{ClusterSystem, JoinCandidate, JoinContact, JoinReadiness};

impl ClusterSystem {
    /// Tracks an ordinary Field window docked against the currently bloomed
    /// collapsed-cluster core.
    pub(crate) fn update_join_candidate(
        &mut self,
        field: &Field,
        output: &str,
        member: NodeId,
        contact: JoinContact,
        now: Duration,
    ) -> bool {
        let eligible_surface = field
            .node(member)
            .is_some_and(|node| matches!(node.state, NodeState::Active | NodeState::Drifting));
        if !eligible_surface
            || self.registry.is_cluster_member(member)
            || self.active_on(output).is_some()
        {
            return self.cancel_join_candidate();
        }
        let Some(cluster_id) = self.bloom.join_target_on_output(output) else {
            return self.cancel_join_candidate();
        };
        let Some(metadata) = self.metadata(cluster_id) else {
            return self.cancel_join_candidate();
        };
        let gap = contact.gap.max(0.0);
        let core_radius = contact.core_radius.max(0.0);
        let dx = contact.center.x - metadata.core_position.x;
        let dy = contact.center.y - metadata.core_position.y;
        let horizontal_extent = if dx >= 0.0 {
            contact.member_left
        } else {
            contact.member_right
        };
        let vertical_extent = if dy >= 0.0 {
            contact.member_top
        } else {
            contact.member_bottom
        };
        let touching_gap = dx.abs() <= horizontal_extent.max(0.0) + core_radius + gap
            && dy.abs() <= vertical_extent.max(0.0) + core_radius + gap;
        if !touching_gap {
            return self.cancel_join_candidate();
        }
        if self.join_candidate.as_ref().is_some_and(|candidate| {
            candidate.member == member
                && candidate.cluster_id == cluster_id
                && candidate.output == output
        }) {
            return false;
        }
        self.join_candidate = Some(JoinCandidate {
            member,
            cluster_id,
            output: output.to_string(),
            started_at: now,
            ready: false,
        });
        true
    }

    pub fn tick_join_candidate_ready(&mut self, now: Duration) -> bool {
        let Some(candidate) = self.join_candidate.as_mut() else {
            return false;
        };
        if candidate.ready {
            return false;
        }
        if now.saturating_sub(candidate.started_at)
            < Duration::from_millis(self.config.join_dwell_ms)
        {
            return false;
        }
        candidate.ready = true;
        true
    }

    pub fn cancel_join_candidate(&mut self) -> bool {
        self.join_candidate.take().is_some()
    }

    /// Completes an armed join on button release. Releasing before readiness
    /// consumes the candidate without absorbing the window.
    pub fn commit_join_candidate(
        &mut self,
        field: &mut Field,
        member: NodeId,
    ) -> Option<ClusterId> {
        let candidate = self.join_candidate.take()?;
        if candidate.member != member
            || !candidate.ready
            || self.active_on(&candidate.output).is_some()
            || self.bloom.join_target_on_output(&candidate.output) != Some(candidate.cluster_id)
        {
            return None;
        }
        self.registry
            .add_member_to_cluster(field, candidate.cluster_id, member)
            .ok()?;
        Some(candidate.cluster_id)
    }

    pub(crate) fn join_readiness_on_output(&self, output: &str) -> Option<JoinReadiness> {
        let candidate = self.join_candidate.as_ref()?;
        if !candidate.ready
            || candidate.output != output
            || self.bloom.join_target_on_output(output) != Some(candidate.cluster_id)
        {
            return None;
        }
        Some(JoinReadiness {
            member: candidate.member,
            cluster_id: candidate.cluster_id,
        })
    }

    pub(crate) fn join_ready_for(&self, member: NodeId, output: &str) -> bool {
        self.join_readiness_on_output(output)
            .is_some_and(|readiness| readiness.member == member)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clusters::bloom::HOLD_DURATION;
    use halley_core::field::Vec2;

    fn surface(field: &mut Field, label: &str, x: f32) -> NodeId {
        field.spawn_surface(label, Vec2 { x, y: 100.0 }, Vec2 { x: 320.0, y: 200.0 })
    }

    fn clustered_system() -> (ClusterSystem, Field, ClusterId, NodeId) {
        let mut field = Field::new();
        let first = surface(&mut field, "first", 80.0);
        let second = surface(&mut field, "second", 120.0);
        let joining = surface(&mut field, "joining", 500.0);
        let config = halley_config::Clusters {
            join_dwell_ms: 500,
            ..halley_config::Clusters::default()
        };
        let mut system = ClusterSystem::new(config, halley_config::ClusterAnimation::default());
        assert!(system.begin_creation("DP-1".into()));
        assert!(system.toggle_creation_member(first, "DP-1"));
        assert!(system.toggle_creation_member(second, "DP-1"));
        assert!(system.begin_naming());
        let cluster = system.finish_creation(&mut field).unwrap();
        (system, field, cluster, joining)
    }

    fn open_bloom(system: &mut ClusterSystem, cluster: ClusterId) {
        assert!(system.set_hovered_core(Some(cluster), Duration::ZERO));
        assert!(system.bloom_wakeup(HOLD_DURATION));
    }

    fn update_at(
        system: &mut ClusterSystem,
        field: &Field,
        member: NodeId,
        center: Vec2,
        now: Duration,
    ) -> bool {
        system.update_join_candidate(
            field,
            "DP-1",
            member,
            JoinContact {
                center,
                member_left: 160.0,
                member_right: 160.0,
                member_top: 100.0,
                member_bottom: 100.0,
                core_radius: 34.0,
                gap: 20.0,
            },
            now,
        )
    }

    #[test]
    fn only_an_open_non_closing_bloom_accepts_a_join_candidate() {
        let (mut system, field, cluster, joining) = clustered_system();
        let core = system.metadata(cluster).unwrap().core_position;
        assert!(!update_at(
            &mut system,
            &field,
            joining,
            core,
            Duration::ZERO
        ));

        open_bloom(&mut system, cluster);
        assert!(update_at(&mut system, &field, joining, core, HOLD_DURATION));
        assert!(system.close_bloom("DP-1", HOLD_DURATION));
        assert!(system.join_candidate.is_none());
        assert!(!update_at(
            &mut system,
            &field,
            joining,
            core,
            HOLD_DURATION
        ));
    }

    #[test]
    fn bounds_plus_landmark_gap_control_candidate_contact() {
        let (mut system, field, cluster, joining) = clustered_system();
        open_bloom(&mut system, cluster);
        let core = system.metadata(cluster).unwrap().core_position;
        let contact = Vec2 {
            x: core.x + 160.0 + 34.0 + 20.0,
            y: core.y,
        };
        assert!(update_at(
            &mut system,
            &field,
            joining,
            contact,
            HOLD_DURATION
        ));
        assert!(update_at(
            &mut system,
            &field,
            joining,
            Vec2 {
                x: contact.x + 0.1,
                y: contact.y,
            },
            HOLD_DURATION
        ));
        assert!(system.join_candidate.is_none());
    }

    #[test]
    fn dwell_readiness_arms_the_affordance_and_release_join() {
        let (mut system, mut field, cluster, joining) = clustered_system();
        open_bloom(&mut system, cluster);
        let core = system.metadata(cluster).unwrap().core_position;
        assert!(update_at(
            &mut system,
            &field,
            joining,
            core,
            Duration::from_millis(2_000)
        ));
        assert!(!system.tick_join_candidate_ready(Duration::from_millis(2_499)));
        assert!(system.join_readiness_on_output("DP-1").is_none());
        assert!(system.tick_join_candidate_ready(Duration::from_millis(2_500)));
        assert_eq!(
            system.join_readiness_on_output("DP-1"),
            Some(JoinReadiness {
                member: joining,
                cluster_id: cluster,
            })
        );
        assert!(system.join_ready_for(joining, "DP-1"));
        assert_eq!(
            system.commit_join_candidate(&mut field, joining),
            Some(cluster)
        );
        assert!(system.registry().is_cluster_member(joining));
    }

    #[test]
    fn early_release_consumes_candidate_without_joining() {
        let (mut system, mut field, cluster, joining) = clustered_system();
        open_bloom(&mut system, cluster);
        let core = system.metadata(cluster).unwrap().core_position;
        assert!(update_at(
            &mut system,
            &field,
            joining,
            core,
            Duration::from_millis(2_000)
        ));
        assert_eq!(system.commit_join_candidate(&mut field, joining), None);
        assert!(!system.registry().is_cluster_member(joining));
        assert!(system.join_candidate.is_none());
    }
}
