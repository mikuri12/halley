use std::collections::HashMap;

use halley_core::field::{NodeId, Vec2};
use halley_core::overlap_physics::{
    CONTACT_SKIN, MAX_PHYSICS_SPEED, PHYSICS_REST_EPSILON, POSITION_SOLVER_ITERS,
    resolve_contact_pair,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BodyKind {
    Node,
    Window,
}

#[derive(Clone, Debug)]
pub(super) struct Body {
    pub id: NodeId,
    pub kind: BodyKind,
    pub pos: Vec2,
    pub extents: CollisionExtents,
    pub gap: f32,
    pub pinned: bool,
    pub output: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct CollisionExtents {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

impl CollisionExtents {
    pub fn symmetric(half: Vec2) -> Self {
        Self {
            left: half.x,
            right: half.x,
            top: half.y,
            bottom: half.y,
        }
    }

    fn minimum_half_extent(self) -> f32 {
        ((self.left + self.right) * 0.5)
            .min((self.top + self.bottom) * 0.5)
            .max(1.0)
    }
}

fn required_separation(a: &Body, b: &Body, dx: f32, dy: f32) -> (f32, f32) {
    let gap = a.gap.max(b.gap);
    let x = if dx >= 0.0 {
        a.extents.right + b.extents.left
    } else {
        a.extents.left + b.extents.right
    } + gap;
    let y = if dy >= 0.0 {
        a.extents.bottom + b.extents.top
    } else {
        a.extents.top + b.extents.bottom
    } + gap;
    (x, y)
}

fn pair_participates(a: &Body, b: &Body) -> bool {
    a.output == b.output && !(a.kind == BodyKind::Window && b.kind == BodyKind::Window)
}

fn overlap(a: &Body, a_pos: Vec2, b: &Body, b_pos: Vec2) -> Option<(f32, f32, f32, f32)> {
    if !pair_participates(a, b) {
        return None;
    }
    let dx = b_pos.x - a_pos.x;
    let dy = b_pos.y - a_pos.y;
    let (required_x, required_y) = required_separation(a, b, dx, dy);
    let ox = required_x - dx.abs();
    let oy = required_y - dy.abs();
    (ox > 0.0 && oy > 0.0).then_some((dx, dy, ox, oy))
}

fn pair_inverse_mass(a: &Body, b: &Body, authority: Option<NodeId>, physics: bool) -> (f32, f32) {
    let mut a_locked = a.pinned;
    let mut b_locked = b.pinned;

    match authority {
        Some(id) if id == a.id => {
            a_locked = true;
            if b.pinned {
                a_locked = false;
            } else if a.kind == BodyKind::Node && b.kind == BodyKind::Window && !physics {
                a_locked = false;
                b_locked = true;
            }
        }
        Some(id) if id == b.id => {
            b_locked = true;
            if a.pinned {
                b_locked = false;
            } else if b.kind == BodyKind::Node && a.kind == BodyKind::Window && !physics {
                b_locked = false;
                a_locked = true;
            }
        }
        _ => {
            // Old Halley's passive rule treats a landmark as the stable
            // reference and makes an active window yield.
            if a.kind == BodyKind::Node && b.kind == BodyKind::Window {
                a_locked = true;
            } else if b.kind == BodyKind::Node && a.kind == BodyKind::Window {
                b_locked = true;
            }
        }
    }

    (
        if a_locked { 0.0 } else { 1.0 },
        if b_locked { 0.0 } else { 1.0 },
    )
}

fn solve_static_with_mass(
    bodies: &[Body],
    authority: Option<(NodeId, Vec2)>,
    inverse_mass: impl Fn(&Body, &Body, Option<NodeId>) -> (f32, f32),
) -> HashMap<NodeId, Vec2> {
    let authority_id = authority.map(|(id, _)| id);
    let mut positions = bodies
        .iter()
        .map(|body| (body.id, body.pos))
        .collect::<HashMap<_, _>>();
    if let Some((id, desired)) = authority {
        positions.insert(id, desired);
    }

    for _ in 0..24 {
        let mut changed = false;
        for i in 0..bodies.len() {
            for j in (i + 1)..bodies.len() {
                let a = &bodies[i];
                let b = &bodies[j];
                let Some(a_pos) = positions.get(&a.id).copied() else {
                    continue;
                };
                let Some(b_pos) = positions.get(&b.id).copied() else {
                    continue;
                };
                let Some((dx, dy, ox, oy)) = overlap(a, a_pos, b, b_pos) else {
                    continue;
                };
                let (inv_a, inv_b) = inverse_mass(a, b, authority_id);
                let total = inv_a + inv_b;
                if total <= 0.0 {
                    continue;
                }
                let solve_x = ox < oy;
                let amount = if solve_x { ox + 0.3 } else { oy + 0.3 };
                let direction = if solve_x {
                    Vec2 {
                        x: if dx.abs() > f32::EPSILON {
                            dx.signum()
                        } else if a.id.as_u64() < b.id.as_u64() {
                            -1.0
                        } else {
                            1.0
                        },
                        y: 0.0,
                    }
                } else {
                    Vec2 {
                        x: 0.0,
                        y: if dy.abs() > f32::EPSILON {
                            dy.signum()
                        } else {
                            1.0
                        },
                    }
                };
                let move_a = amount * inv_a / total;
                let move_b = amount * inv_b / total;
                positions.insert(
                    a.id,
                    Vec2 {
                        x: a_pos.x - direction.x * move_a,
                        y: a_pos.y - direction.y * move_a,
                    },
                );
                positions.insert(
                    b.id,
                    Vec2 {
                        x: b_pos.x + direction.x * move_b,
                        y: b_pos.y + direction.y * move_b,
                    },
                );
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    positions
}

pub(super) fn solve_static(
    bodies: &[Body],
    authority: Option<(NodeId, Vec2)>,
) -> HashMap<NodeId, Vec2> {
    solve_static_with_mass(bodies, authority, |a, b, authority_id| {
        pair_inverse_mass(a, b, authority_id, false)
    })
}

/// Reflow fixed-pixel landmarks after zoom-out expands their world-space
/// footprint. Active windows are stationary obstacles; ordinary nodes and
/// collapsed cluster cores share the node body kind and yield equally unless
/// their persistent pin policy locks them.
pub(super) fn solve_zoom_reflow(bodies: &[Body]) -> HashMap<NodeId, Vec2> {
    solve_static_with_mass(bodies, None, |a, b, _| {
        let inverse = |body: &Body| {
            if body.pinned || body.kind == BodyKind::Window {
                0.0
            } else {
                1.0
            }
        };
        (inverse(a), inverse(b))
    })
}

/// Positions closer than this are treated as the same landmark destination.
pub(super) const LANDMARK_POSITION_EPSILON: f32 = 0.001;

/// How many settle passes the zoom-return solver may run before giving up on
/// further progress. Each pass lets a landmark that was waiting on a neighbour
/// move again, so a short chain still resolves completely.
const ZOOM_RETURN_PASSES: usize = 8;
const ZOOM_RETURN_STEPS: usize = 12;

fn position_reached(a: Vec2, b: Vec2) -> bool {
    (a.x - b.x).abs() <= LANDMARK_POSITION_EPSILON && (a.y - b.y).abs() <= LANDMARK_POSITION_EPSILON
}

/// Walk one landmark as far toward `home` as the rest of the arrangement
/// allows and leave its best legal position in `positions`.
fn advance_toward_home(
    bodies: &[Body],
    positions: &mut HashMap<NodeId, Vec2>,
    id: NodeId,
    home: Vec2,
) -> bool {
    let Some(current) = positions.get(&id).copied() else {
        return false;
    };
    if position_reached(current, home) {
        return false;
    }
    // Landing exactly on home is better than any fraction of the way there, so
    // test it before falling back to the incremental search.
    positions.insert(id, home);
    if positions_are_legal(bodies, positions) {
        return true;
    }
    positions.insert(id, current);
    let along = |fraction: f32| Vec2 {
        x: current.x + (home.x - current.x) * fraction,
        y: current.y + (home.y - current.y) * fraction,
    };
    let mut low = 0.0_f32;
    let mut high = 1.0_f32;
    let mut best = current;
    for _ in 0..ZOOM_RETURN_STEPS {
        let fraction = (low + high) * 0.5;
        positions.insert(id, along(fraction));
        let legal = positions_are_legal(bodies, positions);
        positions.insert(id, current);
        if legal {
            low = fraction;
            best = along(fraction);
        } else {
            high = fraction;
        }
    }
    if position_reached(best, current) {
        return false;
    }
    positions.insert(id, best);
    true
}

/// Returns landmarks that camera zoom displaced back toward their remembered
/// pre-zoom homes as the presented footprint shrinks again. Windows and pinned
/// landmarks stay put as obstacles, and a blocked landmark stops at the
/// farthest legal point on its way home so a later zoom-in can continue.
pub(super) fn solve_zoom_return(
    bodies: &[Body],
    homes: &HashMap<NodeId, Vec2>,
) -> HashMap<NodeId, Vec2> {
    let mut positions = bodies
        .iter()
        .map(|body| (body.id, body.pos))
        .collect::<HashMap<_, _>>();
    let mut eligible = bodies
        .iter()
        .filter(|body| body.kind == BodyKind::Node && !body.pinned && homes.contains_key(&body.id))
        .map(|body| body.id)
        .collect::<Vec<_>>();
    // Stable NodeId order keeps the incremental result reproducible.
    eligible.sort_by_key(|id| id.as_u64());
    if eligible.is_empty() {
        return positions;
    }

    // Restoring the whole arrangement exactly is the common case once the
    // original footprint is legal again, so try it before anything incremental.
    let mut exact = positions.clone();
    for id in &eligible {
        exact.insert(*id, homes[id]);
    }
    if positions_are_legal(bodies, &exact) {
        return exact;
    }

    for _ in 0..ZOOM_RETURN_PASSES {
        let mut progressed = false;
        for id in &eligible {
            let Some(home) = homes.get(id).copied() else {
                continue;
            };
            progressed |= advance_toward_home(bodies, &mut positions, *id, home);
        }
        if !progressed {
            break;
        }
    }
    positions
}

fn positions_are_legal(bodies: &[Body], positions: &HashMap<NodeId, Vec2>) -> bool {
    for i in 0..bodies.len() {
        for j in (i + 1)..bodies.len() {
            let a = &bodies[i];
            let b = &bodies[j];
            let Some(a_pos) = positions.get(&a.id).copied() else {
                continue;
            };
            let Some(b_pos) = positions.get(&b.id).copied() else {
                continue;
            };
            if overlap(a, a_pos, b, b_pos).is_some() {
                return false;
            }
        }
    }
    true
}

fn update_body_positions(bodies: &mut [Body], positions: &HashMap<NodeId, Vec2>) {
    for body in bodies {
        if let Some(position) = positions.get(&body.id).copied() {
            body.pos = position;
        }
    }
}

fn sweep_steps(bodies: &[Body], authority: NodeId, desired: Vec2) -> (usize, Vec2) {
    let Some(body) = bodies.iter().find(|body| body.id == authority) else {
        return (1, Vec2 { x: 0.0, y: 0.0 });
    };
    let delta = Vec2 {
        x: desired.x - body.pos.x,
        y: desired.y - body.pos.y,
    };
    let max_step = bodies
        .iter()
        .filter(|candidate| candidate.kind == BodyKind::Node && candidate.output == body.output)
        .map(|candidate| candidate.extents.minimum_half_extent())
        .reduce(f32::min)
        .unwrap_or(64.0);
    let steps = ((delta.x.abs().max(delta.y.abs()) / max_step).ceil() as usize).clamp(1, 512);
    (
        steps,
        Vec2 {
            x: delta.x / steps as f32,
            y: delta.y / steps as f32,
        },
    )
}

/// Resolve an interactive rigid move in short segments. Besides preventing a
/// fast pointer event from tunnelling through a landmark, the legal-position
/// search makes a chain ending at a pinned node push back on the authority.
pub(super) fn solve_static_swept(
    mut bodies: Vec<Body>,
    authority: NodeId,
    desired: Vec2,
) -> HashMap<NodeId, Vec2> {
    let (steps, delta) = sweep_steps(&bodies, authority, desired);
    let mut positions = bodies
        .iter()
        .map(|body| (body.id, body.pos))
        .collect::<HashMap<_, _>>();

    for _ in 0..steps {
        let Some(origin) = positions.get(&authority).copied() else {
            break;
        };
        let target = Vec2 {
            x: origin.x + delta.x,
            y: origin.y + delta.y,
        };
        let candidate = solve_static(&bodies, Some((authority, target)));
        if positions_are_legal(&bodies, &candidate) {
            positions = candidate;
            update_body_positions(&mut bodies, &positions);
            continue;
        }

        // The local solver cannot satisfy this whole increment, normally
        // because a movable chain has reached a pinned endpoint. Keep the
        // farthest legal fraction so contact remains exact without a pop.
        let mut low = 0.0_f32;
        let mut high = 1.0_f32;
        let mut best = positions.clone();
        for _ in 0..10 {
            let fraction = (low + high) * 0.5;
            let partial = Vec2 {
                x: origin.x + delta.x * fraction,
                y: origin.y + delta.y * fraction,
            };
            let resolved = solve_static(&bodies, Some((authority, partial)));
            if positions_are_legal(&bodies, &resolved) {
                low = fraction;
                best = resolved;
            } else {
                high = fraction;
            }
        }
        positions = best;
        update_body_positions(&mut bodies, &positions);
    }
    positions
}

/// Resolves one newly admitted landmark without displacing any established
/// field body. Creation uses this before the core's first visible frame so a
/// later physics tick cannot turn the initial overlap into a jump.
pub(super) fn solve_new_landmark(
    mut bodies: Vec<Body>,
    landmark: NodeId,
    desired: Vec2,
) -> HashMap<NodeId, Vec2> {
    for body in &mut bodies {
        body.pinned = body.id != landmark;
    }
    solve_static(&bodies, Some((landmark, desired)))
}

pub(super) fn solve_physics(
    bodies: &[Body],
    velocities: &mut HashMap<NodeId, Vec2>,
    authority: Option<(NodeId, Vec2, Vec2)>,
    dt: f32,
    damping: f32,
) -> HashMap<NodeId, Vec2> {
    let authority_id = authority.map(|(id, _, _)| id);
    let mut positions = bodies
        .iter()
        .map(|body| (body.id, body.pos))
        .collect::<HashMap<_, _>>();
    let mut frame_velocities = bodies
        .iter()
        .map(|body| {
            let velocity = authority
                .filter(|(id, _, _)| *id == body.id)
                .map(|(_, _, velocity)| velocity)
                .unwrap_or_else(|| {
                    velocities
                        .get(&body.id)
                        .copied()
                        .unwrap_or(Vec2 { x: 0.0, y: 0.0 })
                });
            (body.id, clamp_speed(velocity))
        })
        .collect::<HashMap<_, _>>();
    if let Some((id, desired, _)) = authority {
        positions.insert(id, desired);
    }

    let dt = dt.clamp(0.0, 1.0 / 30.0);
    let frame_damping = (-damping_per_second(damping) * dt).exp();
    for body in bodies {
        if body.pinned || authority_id == Some(body.id) {
            continue;
        }
        if let (Some(pos), Some(velocity)) = (
            positions.get_mut(&body.id),
            frame_velocities.get_mut(&body.id),
        ) {
            pos.x += velocity.x * dt;
            pos.y += velocity.y * dt;
            velocity.x *= frame_damping;
            velocity.y *= frame_damping;
        }
    }

    for _ in 0..POSITION_SOLVER_ITERS {
        for i in 0..bodies.len() {
            for j in (i + 1)..bodies.len() {
                let a = &bodies[i];
                let b = &bodies[j];
                if !pair_participates(a, b) {
                    continue;
                }
                let Some(a_pos) = positions.get(&a.id).copied() else {
                    continue;
                };
                let Some(b_pos) = positions.get(&b.id).copied() else {
                    continue;
                };
                let dx = b_pos.x - a_pos.x;
                let dy = b_pos.y - a_pos.y;
                let (required_x, required_y) = required_separation(a, b, dx, dy);
                let gap_x = dx.abs() - required_x;
                let gap_y = dy.abs() - required_y;
                if gap_x > CONTACT_SKIN || gap_y > CONTACT_SKIN {
                    continue;
                }
                let (inv_a, inv_b) = pair_inverse_mass(a, b, authority_id, true);
                if inv_a + inv_b <= 0.0 {
                    continue;
                }
                resolve_contact_pair(
                    &mut positions,
                    &mut frame_velocities,
                    a.id,
                    b.id,
                    dx,
                    dy,
                    gap_x,
                    gap_y,
                    inv_a,
                    inv_b,
                );
            }
        }
    }

    velocities.clear();
    for body in bodies {
        if body.pinned || authority_id == Some(body.id) {
            continue;
        }
        let velocity = clamp_speed(
            frame_velocities
                .get(&body.id)
                .copied()
                .unwrap_or(Vec2 { x: 0.0, y: 0.0 }),
        );
        if velocity.x.abs() >= PHYSICS_REST_EPSILON || velocity.y.abs() >= PHYSICS_REST_EPSILON {
            velocities.insert(body.id, velocity);
        }
    }
    positions
}

/// Physics contacts use the same swept pointer path as rigid movement, while
/// dividing the frame time so impulses and damping are not multiplied.
pub(super) fn solve_physics_swept(
    mut bodies: Vec<Body>,
    velocities: &mut HashMap<NodeId, Vec2>,
    authority: (NodeId, Vec2, Vec2),
    dt: f32,
    damping: f32,
) -> HashMap<NodeId, Vec2> {
    let (id, desired, velocity) = authority;
    let (steps, delta) = sweep_steps(&bodies, id, desired);
    let step_dt = dt / steps as f32;
    let mut positions = bodies
        .iter()
        .map(|body| (body.id, body.pos))
        .collect::<HashMap<_, _>>();
    for _ in 0..steps {
        let Some(origin) = positions.get(&id).copied() else {
            break;
        };
        let target = Vec2 {
            x: origin.x + delta.x,
            y: origin.y + delta.y,
        };
        positions = solve_physics(
            &bodies,
            velocities,
            Some((id, target, velocity)),
            step_dt,
            damping,
        );
        update_body_positions(&mut bodies, &positions);
    }
    positions
}

pub(super) fn damping_per_second(user: f32) -> f32 {
    if !user.is_finite() {
        return 4.5;
    }
    let x = user.clamp(0.0, 1.0);
    let curved = 1.0 - (1.0 - x) * (1.0 - x);
    3.0 + curved * 5.0
}

fn clamp_speed(velocity: Vec2) -> Vec2 {
    let speed_squared = velocity.x * velocity.x + velocity.y * velocity.y;
    if speed_squared <= MAX_PHYSICS_SPEED * MAX_PHYSICS_SPEED {
        return velocity;
    }
    let factor = MAX_PHYSICS_SPEED / speed_squared.sqrt().max(f32::EPSILON);
    Vec2 {
        x: velocity.x * factor,
        y: velocity.y * factor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(id: u64, kind: BodyKind, x: f32) -> Body {
        Body {
            id: NodeId::new(id),
            kind,
            pos: Vec2 { x, y: 0.0 },
            extents: CollisionExtents::symmetric(Vec2 { x: 10.0, y: 10.0 }),
            gap: 0.0,
            pinned: false,
            output: "DP-1".into(),
        }
    }

    #[test]
    fn dragged_window_is_authority_and_pushes_node_one_for_one() {
        let window = body(1, BodyKind::Window, 0.0);
        let node = body(2, BodyKind::Node, 25.0);
        let positions = solve_static(
            &[window, node],
            Some((NodeId::new(1), Vec2 { x: 10.0, y: 0.0 })),
        );
        assert_eq!(positions[&NodeId::new(1)].x, 10.0);
        assert!((positions[&NodeId::new(2)].x - 30.3).abs() < 0.01);
    }

    #[test]
    fn expanded_windows_may_overlap() {
        let a = body(1, BodyKind::Window, 0.0);
        let b = body(2, BodyKind::Window, 0.0);
        let positions = solve_static(&[a, b], None);
        assert_eq!(positions[&NodeId::new(1)].x, 0.0);
        assert_eq!(positions[&NodeId::new(2)].x, 0.0);
    }

    #[test]
    fn zoom_reflow_moves_nodes_and_cores_away_from_stationary_windows() {
        let window = body(1, BodyKind::Window, 0.0);
        let node_by_window = body(2, BodyKind::Node, 0.0);
        let ordinary_node = body(3, BodyKind::Node, 100.0);
        // Cluster cores use the same body kind with their larger extents.
        let mut cluster_core = body(4, BodyKind::Node, 100.0);
        cluster_core.extents = CollisionExtents::symmetric(Vec2 { x: 20.0, y: 20.0 });
        let bodies = vec![window, node_by_window, ordinary_node, cluster_core];
        let positions = solve_zoom_reflow(&bodies);

        assert_eq!(positions[&NodeId::new(1)], Vec2 { x: 0.0, y: 0.0 });
        assert_ne!(positions[&NodeId::new(2)], Vec2 { x: 0.0, y: 0.0 });
        assert_ne!(positions[&NodeId::new(3)], Vec2 { x: 100.0, y: 0.0 });
        assert_ne!(positions[&NodeId::new(4)], Vec2 { x: 100.0, y: 0.0 });
        assert!(positions_are_legal(&bodies, &positions));
    }

    #[test]
    fn zoom_reflow_keeps_a_pinned_core_fixed() {
        let ordinary_node = body(1, BodyKind::Node, 0.0);
        let mut pinned_core = body(2, BodyKind::Node, 0.0);
        pinned_core.pinned = true;
        pinned_core.extents = CollisionExtents::symmetric(Vec2 { x: 20.0, y: 20.0 });
        let bodies = vec![ordinary_node, pinned_core];
        let positions = solve_zoom_reflow(&bodies);

        assert_ne!(positions[&NodeId::new(1)], Vec2 { x: 0.0, y: 0.0 });
        assert_eq!(positions[&NodeId::new(2)], Vec2 { x: 0.0, y: 0.0 });
        assert!(positions_are_legal(&bodies, &positions));
    }

    fn homes(entries: &[(u64, f32)]) -> HashMap<NodeId, Vec2> {
        entries
            .iter()
            .map(|(id, x)| (NodeId::new(*id), Vec2 { x: *x, y: 0.0 }))
            .collect()
    }

    #[test]
    fn zoom_return_restores_every_displaced_landmark_once_its_home_is_legal() {
        // Both landmarks were pushed apart by a zoom-out; the smaller footprint
        // makes their original positions legal again.
        let displaced_a = body(1, BodyKind::Node, 200.0);
        let displaced_b = body(2, BodyKind::Node, 300.0);
        let bodies = vec![displaced_a, displaced_b];
        let positions = solve_zoom_return(&bodies, &homes(&[(1, 0.0), (2, 60.0)]));

        assert_eq!(positions[&NodeId::new(1)], Vec2 { x: 0.0, y: 0.0 });
        assert_eq!(positions[&NodeId::new(2)], Vec2 { x: 60.0, y: 0.0 });
        assert!(positions_are_legal(&bodies, &positions));
    }

    #[test]
    fn zoom_return_stops_at_the_legal_limit_when_a_home_stays_blocked() {
        // The pinned window keeps the node's home illegal, so the node may only
        // travel back as far as exact contact and stays recoverable.
        let window = body(1, BodyKind::Window, 0.0);
        let node = body(2, BodyKind::Node, 200.0);
        let bodies = vec![window, node];
        let positions = solve_zoom_return(&bodies, &homes(&[(2, 10.0)]));

        assert_eq!(positions[&NodeId::new(1)], Vec2 { x: 0.0, y: 0.0 });
        assert!(positions[&NodeId::new(2)].x < 200.0);
        assert!(positions[&NodeId::new(2)].x >= 20.0);
        assert!(!position_reached(
            positions[&NodeId::new(2)],
            Vec2 { x: 10.0, y: 0.0 }
        ));
        assert!(positions_are_legal(&bodies, &positions));
    }

    #[test]
    fn zoom_return_settles_a_displaced_chain_in_bounded_passes() {
        // The two remembered homes overlap, so only an incremental return is
        // possible: the landmark with the free path goes all the way home and
        // the other one comes back to exact contact and stays recoverable.
        let free = body(1, BodyKind::Node, 200.0);
        let blocked = body(2, BodyKind::Node, 400.0);
        let bodies = vec![free, blocked];
        let positions = solve_zoom_return(&bodies, &homes(&[(1, 0.0), (2, 10.0)]));

        assert_eq!(positions[&NodeId::new(1)], Vec2 { x: 0.0, y: 0.0 });
        assert!(!position_reached(
            positions[&NodeId::new(2)],
            Vec2 { x: 10.0, y: 0.0 }
        ));
        assert!(positions[&NodeId::new(2)].x >= 20.0);
        assert!(positions[&NodeId::new(2)].x < 400.0);
        assert!(positions_are_legal(&bodies, &positions));
    }

    #[test]
    fn zoom_return_never_moves_pinned_landmarks_or_windows() {
        let mut pinned_core = body(1, BodyKind::Node, 300.0);
        pinned_core.pinned = true;
        pinned_core.extents = CollisionExtents::symmetric(Vec2 { x: 20.0, y: 20.0 });
        let window = body(2, BodyKind::Window, 400.0);
        let bodies = vec![pinned_core, window];
        let positions = solve_zoom_return(&bodies, &homes(&[(1, 0.0), (2, 0.0)]));

        assert_eq!(positions[&NodeId::new(1)], Vec2 { x: 300.0, y: 0.0 });
        assert_eq!(positions[&NodeId::new(2)], Vec2 { x: 400.0, y: 0.0 });
    }

    #[test]
    fn zoom_return_keeps_outputs_isolated() {
        let mut local = body(1, BodyKind::Node, 200.0);
        local.output = "DP-1".into();
        let mut remote = body(2, BodyKind::Node, 200.0);
        remote.output = "DP-2".into();
        let bodies = vec![local, remote];
        let positions = solve_zoom_return(&bodies, &homes(&[(1, 0.0)]));

        assert_eq!(positions[&NodeId::new(1)], Vec2 { x: 0.0, y: 0.0 });
        assert_eq!(positions[&NodeId::new(2)], Vec2 { x: 200.0, y: 0.0 });
    }

    #[test]
    fn zoom_return_treats_cluster_cores_like_ordinary_landmarks() {
        let mut core = body(1, BodyKind::Node, 200.0);
        core.extents = CollisionExtents::symmetric(Vec2 { x: 20.0, y: 20.0 });
        let ordinary = body(2, BodyKind::Node, 90.0);
        let bodies = vec![core, ordinary];
        let positions = solve_zoom_return(&bodies, &homes(&[(1, 0.0)]));

        assert_eq!(positions[&NodeId::new(1)], Vec2 { x: 0.0, y: 0.0 });
        assert!(positions_are_legal(&bodies, &positions));
    }

    #[test]
    fn a_drag_chain_reports_every_neighbour_it_physically_moves() {
        // Every landmark the swept drag moves must be committed by the caller,
        // otherwise a pushed neighbour would later snap back to its old home.
        let grabbed = body(1, BodyKind::Node, 0.0);
        let pushed = body(2, BodyKind::Node, 25.0);
        let bodies = vec![grabbed, pushed];
        let start = bodies
            .iter()
            .map(|body| (body.id, body.pos))
            .collect::<HashMap<_, _>>();
        let positions = solve_static_swept(bodies, NodeId::new(1), Vec2 { x: 600.0, y: 0.0 });

        let moved = start
            .into_iter()
            .filter(|(id, from)| {
                let to = positions[id];
                (to.x - from.x).abs() > LANDMARK_POSITION_EPSILON
                    || (to.y - from.y).abs() > LANDMARK_POSITION_EPSILON
            })
            .map(|(id, _)| id)
            .collect::<Vec<_>>();
        assert_eq!(moved.len(), 2);
        assert!(moved.contains(&NodeId::new(1)));
        assert!(moved.contains(&NodeId::new(2)));
    }

    #[test]
    fn asymmetric_titlebar_extent_sets_the_visible_vertical_gap() {
        let mut window = body(1, BodyKind::Window, 0.0);
        window.extents = CollisionExtents {
            left: 10.0,
            right: 10.0,
            top: 30.0,
            bottom: 10.0,
        };
        window.gap = 5.0;
        let node = body(2, BodyKind::Node, 0.0);

        assert!(
            overlap(
                &window,
                Vec2 { x: 0.0, y: 0.0 },
                &node,
                Vec2 { x: 0.0, y: -44.0 }
            )
            .is_some()
        );
        assert!(
            overlap(
                &window,
                Vec2 { x: 0.0, y: 0.0 },
                &node,
                Vec2 { x: 0.0, y: -45.0 }
            )
            .is_none()
        );
        assert!(
            overlap(
                &window,
                Vec2 { x: 0.0, y: 0.0 },
                &node,
                Vec2 { x: 0.0, y: 25.0 }
            )
            .is_none()
        );
    }

    #[test]
    fn passive_window_yields_to_node() {
        let window = body(1, BodyKind::Window, 0.0);
        let node = body(2, BodyKind::Node, 0.0);
        let positions = solve_static(&[window, node], None);
        assert_ne!(positions[&NodeId::new(1)], Vec2 { x: 0.0, y: 0.0 });
        assert_eq!(positions[&NodeId::new(2)], Vec2 { x: 0.0, y: 0.0 });
    }

    #[test]
    fn swept_move_does_not_tunnel_through_a_node() {
        let window = body(1, BodyKind::Window, -40.0);
        let mut node = body(2, BodyKind::Node, 0.0);
        node.pinned = true;
        let positions =
            solve_static_swept(vec![window, node], NodeId::new(1), Vec2 { x: 40.0, y: 0.0 });
        assert!(positions[&NodeId::new(1)].x < 0.0);
        assert_eq!(positions[&NodeId::new(2)].x, 0.0);
    }

    #[test]
    fn pinned_end_stops_a_push_chain_without_overlap() {
        let window = body(1, BodyKind::Window, 0.0);
        let node = body(2, BodyKind::Node, 25.0);
        let mut pinned = body(3, BodyKind::Node, 50.0);
        pinned.pinned = true;
        let bodies = vec![window, node, pinned];
        let positions =
            solve_static_swept(bodies.clone(), NodeId::new(1), Vec2 { x: 30.0, y: 0.0 });
        assert!(positions_are_legal(&bodies, &positions));
        assert!(positions[&NodeId::new(1)].x < 10.0);
        assert_eq!(positions[&NodeId::new(3)].x, 50.0);
    }

    #[test]
    fn a_new_landmark_moves_away_without_displacing_existing_bodies() {
        let window = body(1, BodyKind::Window, 0.0);
        let node = body(2, BodyKind::Node, 0.0);
        let positions =
            solve_new_landmark(vec![window, node], NodeId::new(2), Vec2 { x: 0.0, y: 0.0 });

        assert_eq!(positions[&NodeId::new(1)], Vec2 { x: 0.0, y: 0.0 });
        assert_ne!(positions[&NodeId::new(2)], Vec2 { x: 0.0, y: 0.0 });
    }

    #[test]
    fn dragged_window_imparts_old_halley_contact_velocity() {
        let window = body(1, BodyKind::Window, 0.0);
        let node = body(2, BodyKind::Node, 19.0);
        let mut velocities = HashMap::new();
        let positions = solve_physics_swept(
            vec![window, node],
            &mut velocities,
            (
                NodeId::new(1),
                Vec2 { x: 0.0, y: 0.0 },
                Vec2 { x: 420.0, y: 0.0 },
            ),
            1.0 / 60.0,
            0.25,
        );
        assert_eq!(positions[&NodeId::new(1)], Vec2 { x: 0.0, y: 0.0 });
        assert!(positions[&NodeId::new(2)].x > 19.0);
        assert!(velocities[&NodeId::new(2)].x > 300.0);
    }

    #[test]
    fn physics_drag_yields_to_a_pinned_core() {
        let window = body(1, BodyKind::Window, -40.0);
        let mut core = body(2, BodyKind::Node, 0.0);
        core.pinned = true;
        let mut velocities = HashMap::new();
        let positions = solve_physics_swept(
            vec![window, core],
            &mut velocities,
            (
                NodeId::new(1),
                Vec2 { x: 0.0, y: 0.0 },
                Vec2 { x: 420.0, y: 0.0 },
            ),
            1.0 / 60.0,
            0.25,
        );
        assert!(positions[&NodeId::new(1)].x < 0.0);
        assert_eq!(positions[&NodeId::new(2)], Vec2 { x: 0.0, y: 0.0 });
    }

    #[test]
    fn damping_depends_on_render_time_not_step_count() {
        fn simulate(frames: usize) -> f32 {
            let mut bodies = vec![body(1, BodyKind::Node, 0.0)];
            let mut velocities = HashMap::from([(NodeId::new(1), Vec2 { x: 500.0, y: 0.0 })]);
            for _ in 0..frames {
                let positions =
                    solve_physics(&bodies, &mut velocities, None, 0.5 / frames as f32, 0.25);
                update_body_positions(&mut bodies, &positions);
            }
            velocities[&NodeId::new(1)].x
        }

        let at_60_hz = simulate(30);
        let at_120_hz = simulate(60);
        assert!((at_60_hz - at_120_hz).abs() < 0.01);
    }
}
