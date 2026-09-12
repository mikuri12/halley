//! Native port of hypr-dynamic-cursors (https://github.com/VirtCode/hypr-dynamic-cursors)
//! for Halley's cursor pipeline.
//!
//! The cursor shape reacts to pointer motion:
//!
//! - `rotate`: simulates a stick dragged on one end (rotates towards movement),
//! - `tilt`: tilts based on horizontal velocity,
//! - `stretch`: stretches/squishes along the movement direction,
//! - plus shake-to-find: magnifies the cursor when it is shaken.
//!
//! The upstream plugin renders through Hyprland's GL pipeline with a modified
//! projection matrix. Halley blits the cursor either as CPU rects (composed
//! frames) or as a `MemoryRenderBuffer` element (scanout path), so here the
//! same affine transform (rotation around the hotspot, one-sided stretch
//! around the shape center, uniform zoom around the hotspot) is applied on the
//! CPU by inverse-mapping each destination pixel onto the source sprite.
//!
//! Timing note: the plugin runs a timer at the output refresh rate. Halley
//! instead ticks this state from the frame loop (`advance_tty_redraw_frame`,
//! i.e. once per queued frame) and from pointer motion events, so the velocity
//! windows below are time-based rather than sample-count-based.

use std::collections::VecDeque;
use std::time::Instant;

use halley_config::{CursorActivation, DynamicCursorConfig, DynamicCursorMode};

/// Window of position samples kept for the tilt/stretch speed calculation.
const MODE_SAMPLE_WINDOW_MS: u64 = 1_000;
/// Window of position samples kept for shake detection (~1s, like upstream).
const SHAKE_SAMPLE_WINDOW_MS: u64 = 1_000;
/// Bounding box diagonal (px) a shake must cover to count as a shake.
const SHAKE_MIN_DIAGONAL_PX: f64 = 100.0;
/// Cap for the per-tick dt used to grow the shake magnification. Ticks can be
/// far apart after an idle gap; without a cap the magnification would jump.
const SHAKE_MAX_TICK_DT_S: f32 = 0.1;
/// Duration and easing of the magnification interpolation (upstream uses a
/// 400ms `cubic-bezier(0.22, 1, 0.36, 1)` animated variable).
const SHAKE_ZOOM_ANIM_MS: f32 = 400.0;
/// Snap the shown result to the freshly computed one after this many
/// consecutive ticks without a change, so decay always converges to the
/// identity transform (and continuous redraws stop) even when the configured
/// threshold would otherwise leave a tiny residual angle stuck.
const SNAP_AFTER_STALLED_TICKS: u32 = 30;

// ---------------------------------------------------------------------------
// Result transform
// ---------------------------------------------------------------------------

/// The composed cursor transform for one frame. Mirrors upstream's
/// `SModeResult`: rotation around the hotspot, uniform scale (shake zoom)
/// around the hotspot, and a stretch along `stretch_angle` applied around the
/// shape center with a one-sided anchor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CursorTransform {
    /// Rotation around the hotspot, radians.
    pub rotation: f32,
    /// Uniform zoom around the hotspot (1 = none).
    pub scale: f32,
    /// Stretch axis angle, radians.
    pub stretch_angle: f32,
    /// Per-axis stretch magnitude on the (rotated) x/y axes (1, 1 = none).
    pub stretch_magnitude: (f32, f32),
}

impl Default for CursorTransform {
    fn default() -> Self {
        Self {
            rotation: 0.0,
            scale: 1.0,
            stretch_angle: 0.0,
            stretch_magnitude: (1.0, 1.0),
        }
    }
}

impl CursorTransform {
    pub(crate) fn is_identity(&self) -> bool {
        self.rotation == 0.0
            && self.scale == 1.0
            && self.stretch_magnitude == (1.0, 1.0)
    }

    /// Zeroes components below the given thresholds so the cursor is rendered
    /// pixel-perfectly when no effect is visible (upstream `SModeResult::clamp`).
    fn clamp_thresholds(&mut self, angle: f32, scale: f32, stretch: f32) {
        if self.rotation.abs() < angle {
            self.rotation = 0.0;
        }
        if (1.0 - self.scale).abs() < scale {
            self.scale = 1.0;
        }
        if (1.0 - self.stretch_magnitude.0).abs() < stretch
            && (1.0 - self.stretch_magnitude.1).abs() < stretch
        {
            self.stretch_magnitude = (1.0, 1.0);
        }
    }

    fn has_difference(&self, other: &Self, angle: f32, scale: f32, stretch: f32) -> bool {
        (other.rotation - self.rotation).abs() > angle
            || (other.scale - self.scale).abs() > scale
            || (other.stretch_angle - self.stretch_angle).abs() > angle
            || (other.stretch_magnitude.0 - self.stretch_magnitude.0).abs() > stretch
            || (other.stretch_magnitude.1 - self.stretch_magnitude.1).abs() > stretch
    }
}

// ---------------------------------------------------------------------------
// Activation functions (upstream `src/mode/utils.cpp`)
// ---------------------------------------------------------------------------

fn activation(function: CursorActivation, max: f64, value: f64) -> f64 {
    let max = max.max(1.0);
    let result = match function {
        CursorActivation::Linear => value / max,
        CursorActivation::Quadratic => (value * value) / (max * max) * value.signum(),
        CursorActivation::NegativeQuadratic => {
            let x = value.abs();
            // (-1/m^2)*(x-m)^2 + 1 reaches 1 at m; clamp manually past m.
            let mut r = (-1.0 / (max * max)) * ((x - max) * (x - max)) + 1.0;
            if x > max {
                r = 1.0;
            }
            r * value.signum()
        }
    };
    result.clamp(-1.0, 1.0)
}

/// Angle of the movement direction for the stretch axis, replicating the
/// upstream `-atan(x/y) + PI (+PI if y > 0)` including its div-by-zero guards.
fn movement_angle(x: f64, y: f64) -> f64 {
    let base = if y == 0.0 {
        if x == 0.0 {
            0.0
        } else {
            // atan(+/-inf) = +/- PI/2, so -atan is sign-flipped.
            -std::f64::consts::FRAC_PI_2 * x.signum()
        }
    } else {
        -(x / y).atan()
    };
    let mut angle = base + std::f64::consts::PI;
    if y > 0.0 {
        angle += std::f64::consts::PI;
    }
    angle
}

/// Wraps an angle into (-PI, PI]. The GL pipeline upstream renders rotation
/// mod 2PI implicitly; here the wrap keeps `is_identity` exact (an upright
/// stick computes an angle of 2PI, which is visually the identity).
fn wrap_angle(angle: f64) -> f64 {
    let two_pi = 2.0 * std::f64::consts::PI;
    let wrapped = (angle + std::f64::consts::PI).rem_euclid(two_pi) - std::f64::consts::PI;
    if wrapped.abs() < 1e-9 {
        0.0
    } else {
        wrapped
    }
}

// ---------------------------------------------------------------------------
// Shake zoom easing (upstream: 400ms cubic-bezier(0.22, 1, 0.36, 1))
// ---------------------------------------------------------------------------

fn bezier_component(t: f32, p1: f32, p2: f32) -> f32 {
    let inv = 1.0 - t;
    3.0 * inv * inv * t * p1 + 3.0 * inv * t * t * p2 + t * t * t
}

/// Solves the x(u) = t equation of the cubic bezier and returns y(u).
fn bezier_ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let mut lo = 0.0f32;
    let mut hi = 1.0f32;
    for _ in 0..24 {
        let mid = 0.5 * (lo + hi);
        if bezier_component(mid, 0.22, 0.36) < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let u = 0.5 * (lo + hi);
    bezier_component(u, 1.0, 1.0)
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Sample {
    x: f64,
    y: f64,
    dist: f64,
    at: Option<Instant>,
}

/// Mutable simulation state. Owned by `CursorManager` so that both the input
/// path (motion events -> `on_move`) and the frame loop (ticks ->
/// `on_tick`) reach the same instance the renderers read from.
pub(crate) struct DynamicCursorState {
    /// Last pointer position seen by the simulation (global screen coords).
    last_pos: Option<(f64, f64)>,
    /// End of the simulated stick (rotate mode), absolute screen coords.
    stick_end: (f64, f64),
    /// Position samples for the tilt/stretch speed window.
    mode_samples: VecDeque<Sample>,
    /// Position samples for shake detection (with per-sample trail distance).
    shake_samples: VecDeque<Sample>,
    shake_started: bool,
    shake_end_at: Option<Instant>,
    shake_goal: f32,
    shake_from: f32,
    shake_anim_start: Option<Instant>,
    /// Displayed (interpolated) shake zoom, refreshed every tick.
    shake_displayed: f32,
    last_tick_at: Option<Instant>,
    /// Result of the active mode from the last update that ran for it.
    mode_result: CursorTransform,
    /// Last mode the simulation ran with; a change resets mode state.
    last_mode: Option<DynamicCursorMode>,
    /// The result actually shown to the renderers.
    shown: CursorTransform,
    stalled_ticks: u32,
}

impl Default for DynamicCursorState {
    fn default() -> Self {
        Self {
            last_pos: None,
            stick_end: (0.0, 0.0),
            mode_samples: VecDeque::new(),
            shake_samples: VecDeque::new(),
            shake_started: false,
            shake_end_at: None,
            shake_goal: 1.0,
            shake_from: 1.0,
            shake_anim_start: None,
            shake_displayed: 1.0,
            last_tick_at: None,
            mode_result: CursorTransform::default(),
            last_mode: None,
            shown: CursorTransform::default(),
            stalled_ticks: 0,
        }
    }
}

impl DynamicCursorState {
    pub(crate) fn reset(&mut self) {
        *self = Self {
            // Keep the zoom goal at 1 so a reset while magnified settles down
            // through the normal animation instead of snapping.
            shake_goal: 1.0,
            shake_from: 1.0,
            ..Self::default()
        };
    }

    /// Transform the renderers should apply this frame.
    pub(crate) fn shown(&self) -> CursorTransform {
        self.shown
    }

    /// Whether the frame loop should keep redrawing: decay in flight for the
    /// tick-driven modes (tilt/stretch), or a shake magnification that is
    /// active or still interpolating. The rotate mode intentionally holds its
    /// angle between motion events (upstream behaviour: no decay), so it must
    /// not keep redraws alive by itself.
    pub(crate) fn animation_active(&self, now: Instant) -> bool {
        if self.shake_animating(now) {
            return true;
        }
        matches!(
            self.last_mode,
            Some(DynamicCursorMode::Tilt) | Some(DynamicCursorMode::Stretch)
        ) && !self.shown.is_identity()
    }

    fn shake_animating(&self, now: Instant) -> bool {
        (self.shake_goal - 1.0).abs() > 0.01
            || (self.shake_goal - self.shake_displayed_value(now)).abs() > 0.01
    }

    fn shake_displayed_value(&self, now: Instant) -> f32 {
        let Some(start) = self.shake_anim_start else {
            return self.shake_goal;
        };
        let t = now.duration_since(start).as_millis() as f32 / SHAKE_ZOOM_ANIM_MS;
        let e = bezier_ease(t);
        self.shake_from + (self.shake_goal - self.shake_from) * e
    }

    fn set_shake_goal(&mut self, goal: f32, now: Instant) {
        self.shake_from = self.shake_displayed_value(now);
        self.shake_goal = goal;
        self.shake_anim_start = Some(now);
    }

    /// Called on pointer motion events (upstream `onCursorMoved`).
    pub(crate) fn on_move(
        &mut self,
        pos: (f64, f64),
        delta: (f64, f64),
        cfg: &DynamicCursorConfig,
        now: Instant,
    ) {
        if !cfg.enabled {
            return;
        }

        // Upstream ignores cursor warps (programmatic jumps) by translating
        // the simulation state instead of treating the jump as motion. Halley
        // funnels everything through absolute motion events, so a warp is
        // detected heuristically: the position jumped far beyond what the
        // event's own relative delta explains.
        if cfg.ignore_warps
            && let Some(last) = self.last_pos
        {
            let jump = (pos.0 - last.0, pos.1 - last.1);
            let jump_len = jump.0.hypot(jump.1);
            let mismatch = (jump.0 - delta.0).hypot(jump.1 - delta.1);
            if jump_len > 50.0 && mismatch > 25.0 {
                self.warp(last, pos);
            }
        }
        self.last_pos = Some(pos);

        self.calculate(UpdateKind::Move, pos, now, cfg);
    }

    /// Called once per queued frame (upstream `onTick`).
    pub(crate) fn on_tick(&mut self, pos: (f64, f64), now: Instant, cfg: &DynamicCursorConfig) {
        if !cfg.enabled {
            return;
        }
        self.last_pos = Some(pos);
        self.calculate(UpdateKind::Tick, pos, now, cfg);
    }

    fn warp(&mut self, old: (f64, f64), pos: (f64, f64)) {
        let d = (pos.0 - old.0, pos.1 - old.1);
        self.stick_end = (self.stick_end.0 + d.0, self.stick_end.1 + d.1);
        for sample in &mut self.mode_samples {
            sample.x += d.0;
            sample.y += d.1;
        }
        for sample in &mut self.shake_samples {
            sample.x += d.0;
            sample.y += d.1;
        }
    }

    fn calculate(&mut self, kind: UpdateKind, pos: (f64, f64), now: Instant, cfg: &DynamicCursorConfig) {
        let mode = if cfg.enabled { cfg.mode } else { DynamicCursorMode::None };
        if self.last_mode != Some(mode) {
            self.reset_mode_state();
        }
        self.last_mode = Some(mode);

        // Mode result: rotate updates on motion events, tilt/stretch per tick.
        match mode {
            DynamicCursorMode::Rotate if kind == UpdateKind::Move => {
                self.update_rotate(pos, cfg);
            }
            DynamicCursorMode::Tilt if kind == UpdateKind::Tick => {
                self.push_mode_sample(pos, now, cfg.tilt.window_ms);
                self.mode_result = update_tilt(now, &self.mode_samples, cfg);
            }
            DynamicCursorMode::Stretch if kind == UpdateKind::Tick => {
                self.push_mode_sample(pos, now, cfg.stretch.window_ms);
                self.mode_result = update_stretch(now, &self.mode_samples, cfg);
            }
            _ => {}
        }

        // Shake detection runs per tick; the mode suppression while magnified
        // also applies to motion-event updates (upstream behaviour).
        if cfg.shake.enabled {
            if kind == UpdateKind::Tick {
                let displayed = self.update_shake(pos, now, cfg);
                self.shake_displayed = displayed;
            }
            if self.shake_displayed > 1.0 && !cfg.shake.effects {
                self.mode_result = CursorTransform::default();
            }
        }

        let mut result = self.mode_result;
        if cfg.shake.enabled {
            result.scale *= self.shake_displayed.max(0.01);
        }

        let threshold_rad = cfg.threshold_deg.to_radians();
        if self.shown.has_difference(&result, threshold_rad, 0.01, 0.01) {
            self.shown = result;
            self.shown.clamp_thresholds(threshold_rad, 0.01, 0.01);
            self.stalled_ticks = 0;
        } else if !self.shown.is_identity() || !result.is_identity() {
            self.stalled_ticks += 1;
            if self.stalled_ticks >= SNAP_AFTER_STALLED_TICKS {
                self.shown = result;
                self.shown.clamp_thresholds(threshold_rad, 0.01, 0.01);
                self.stalled_ticks = 0;
            }
        }
    }

    fn reset_mode_state(&mut self) {
        self.stick_end = (0.0, 0.0);
        self.mode_samples.clear();
        self.mode_result = CursorTransform::default();
    }

    /// Stick simulation (upstream `ModeRotate`): the cursor is a stick of
    /// `length` px dragged at the pointer position; it rotates towards the
    /// movement direction.
    fn update_rotate(&mut self, pos: (f64, f64), cfg: &DynamicCursorConfig) {
        let length = cfg.rotate.length.max(1.0) as f64;

        // This mode has just started: begin at upright orientation.
        if self.stick_end == (0.0, 0.0) {
            self.stick_end = (pos.0, pos.1 + length);
        }

        // Translate to origin.
        let mut ex = self.stick_end.0 - pos.0;
        let mut ey = self.stick_end.1 - pos.1;

        // Normalize, then scale to the stick length.
        let size = ex.hypot(ey);
        let size = if size == 0.0 { 1.0 } else { size };
        ex = ex / size * length;
        ey = ey / size * length;

        // Calculate the angle (upstream keeps the atan div-by-zero quirk and
        // overrides the angle when the stick is exactly horizontal), wrapped
        // into (-PI, PI] so an upright stick (2PI) is exactly the identity.
        let mut angle = -(ex / ey).atan();
        if ey > 0.0 {
            angle += std::f64::consts::PI;
        }
        angle += std::f64::consts::PI;
        angle += (cfg.rotate.offset_deg as f64).to_radians();
        if ey == 0.0 {
            angle = 0.0;
        }
        let angle = wrap_angle(angle);

        // Translate back.
        self.stick_end = (ex + pos.0, ey + pos.1);

        self.mode_result = CursorTransform {
            rotation: angle as f32,
            ..CursorTransform::default()
        };
    }

    fn push_mode_sample(&mut self, pos: (f64, f64), now: Instant, window_ms: u64) {
        self.mode_samples.push_back(Sample {
            x: pos.0,
            y: pos.1,
            dist: 0.0,
            at: Some(now),
        });
        // Keep [window] ms of history (at least two samples so a speed is
        // always computable), like the upstream per-refresh ring buffer.
        let window_ms = window_ms.clamp(16, MODE_SAMPLE_WINDOW_MS);
        while self.mode_samples.len() > 2 {
            let cutoff = now.duration_since(self.mode_samples[0].at.unwrap_or(now));
            if cutoff.as_millis() as u64 >= window_ms {
                self.mode_samples.pop_front();
            } else {
                break;
            }
        }
    }

    fn update_shake(&mut self, pos: (f64, f64), now: Instant, cfg: &DynamicCursorConfig) -> f32 {
        let dist = self
            .shake_samples
            .back()
            .map(|last| (pos.0 - last.x).hypot(pos.1 - last.y))
            .unwrap_or(0.0);
        self.shake_samples.push_back(Sample {
            x: pos.0,
            y: pos.1,
            dist,
            at: Some(now),
        });
        while self.shake_samples.len() > 2 {
            let cutoff = now.duration_since(self.shake_samples[0].at.unwrap_or(now));
            if cutoff.as_millis() as u64 >= SHAKE_SAMPLE_WINDOW_MS {
                self.shake_samples.pop_front();
            } else {
                break;
            }
        }

        // Shake detection, inspired by KDE Plasma's shake detector: compare
        // the travelled trail with the diagonal of the travelled bounding box.
        let mut trail = 0.0;
        let mut left = f64::MAX;
        let mut right = f64::MIN;
        let mut top = f64::MAX;
        let mut bottom = f64::MIN;
        for sample in &self.shake_samples {
            trail += sample.dist;
            left = left.min(sample.x);
            right = right.max(sample.x);
            top = top.min(sample.y);
            bottom = bottom.max(sample.y);
        }
        let diagonal = (right - left).hypot(bottom - top);
        let amount = trail / diagonal.max(1.0) - cfg.shake.threshold as f64;

        let tick_dt = self
            .last_tick_at
            .map(|last| now.duration_since(last).as_secs_f32())
            .unwrap_or(0.0)
            .min(SHAKE_MAX_TICK_DT_S);
        self.last_tick_at = Some(now);

        if diagonal > SHAKE_MIN_DIAGONAL_PX && amount > 0.0 {
            let mut next = self.shake_goal;
            if !self.shake_started {
                next = cfg.shake.base;
            }
            next += tick_dt
                * (cfg.shake.speed + (amount * amount) as f32 * cfg.shake.influence);
            if cfg.shake.limit > 1.0 {
                next = next.min(cfg.shake.limit);
            }
            self.set_shake_goal(next, now);
            self.shake_end_at = Some(
                now + std::time::Duration::from_millis(cfg.shake.timeout_ms),
            );
            self.shake_started = true;
        } else if self.shake_started
            && self
                .shake_end_at
                .is_some_and(|end| end < now)
        {
            self.set_shake_goal(1.0, now);
            self.shake_started = false;
        }

        self.shake_displayed_value(now)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UpdateKind {
    Move,
    Tick,
}

// ---------------------------------------------------------------------------
// Modes
// ---------------------------------------------------------------------------

fn update_tilt(now: Instant, samples: &VecDeque<Sample>, cfg: &DynamicCursorConfig) -> CursorTransform {
    let speed = horizontal_speed(samples, now);
    let rotation =
        activation(cfg.tilt.activation, cfg.tilt.limit_px_s as f64, speed)
            * (cfg.tilt.full_deg as f64).to_radians();
    CursorTransform {
        rotation: rotation as f32,
        ..CursorTransform::default()
    }
}

fn update_stretch(now: Instant, samples: &VecDeque<Sample>, cfg: &DynamicCursorConfig) -> CursorTransform {
    let speed = velocity(samples, now);
    let mag = speed.0.hypot(speed.1);

    let angle = if mag == 0.0 {
        0.0
    } else {
        movement_angle(speed.0, speed.1)
    };
    let scale = activation(cfg.stretch.activation, cfg.stretch.limit_px_s as f64, mag);

    CursorTransform {
        stretch_angle: angle as f32,
        // Upstream caps the magnitude because of the buffer size around the
        // shape: (1 - 0.5*s, 1 + s) with s in [0, 1].
        stretch_magnitude: (1.0 - scale as f32 * 0.5, 1.0 + scale as f32),
        ..CursorTransform::default()
    }
}

fn horizontal_speed(samples: &VecDeque<Sample>, now: Instant) -> f64 {
    let (Some(first), Some(last)) = (samples.front(), samples.back()) else {
        return 0.0;
    };
    let dt_ms = now.duration_since(first.at.unwrap_or(now)).as_millis() as f64;
    if dt_ms < 1.0 {
        return 0.0;
    }
    (last.x - first.x) / dt_ms * 1000.0
}

fn velocity(samples: &VecDeque<Sample>, now: Instant) -> (f64, f64) {
    let (Some(first), Some(last)) = (samples.front(), samples.back()) else {
        return (0.0, 0.0);
    };
    let dt_ms = now.duration_since(first.at.unwrap_or(now)).as_millis() as f64;
    if dt_ms < 1.0 {
        return (0.0, 0.0);
    }
    (
        (last.x - first.x) / dt_ms * 1000.0,
        (last.y - first.y) / dt_ms * 1000.0,
    )
}

// ---------------------------------------------------------------------------
// Pixel transform (CPU)
// ---------------------------------------------------------------------------

/// A cursor frame after applying the dynamic transform: pixels in the same
/// BGRA-in-memory layout the sprite cache uses, with the bounding box grown
/// to fit the transformed shape and the hotspot re-anchored so the pointer
/// position stays under the (transformed) hotspot.
pub(crate) struct TransformedCursorFrame {
    pub(crate) pixels: Vec<u8>,
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) hotspot_x: i32,
    pub(crate) hotspot_y: i32,
}

/// Whether pixelated (nearest-neighbour) sampling should be used for the
/// current zoom, from the `cursor.dynamic.shake.nearest` config value.
pub(crate) fn nearest_sampling_enabled(nearest: u8, scale: f32) -> bool {
    match nearest {
        2 => true,
        1 => scale > 1.0,
        _ => false,
    }
}

/// Applies `transform` to a cursor frame.
///
/// The forward map (matching upstream `toTransform`, in image coordinates
/// relative to the hotspot) is: rotate around the hotspot, then stretch
/// around the shape center with a one-sided vertical anchor, then uniform
/// zoom. Dest = pointer position + F(image point - hotspot). Each destination
/// pixel is inverse-mapped and sampled from the source (bilinear with
/// premultiplied alpha, or nearest-neighbour).
pub(crate) fn transform_cursor_frame(
    pixels: &[u8],
    width: usize,
    height: usize,
    hotspot: (i32, i32),
    transform: &CursorTransform,
    nearest: bool,
) -> TransformedCursorFrame {
    let identity = TransformedCursorFrame {
        pixels: pixels.to_vec(),
        width,
        height,
        hotspot_x: hotspot.0,
        hotspot_y: hotspot.1,
    };
    if transform.is_identity() || width == 0 || height == 0 {
        return identity;
    }

    let w = width as f32;
    let h = height as f32;
    let hs = (hotspot.0 as f32, hotspot.1 as f32);

    // Linear part and translation of the forward map on hotspot-relative
    // coordinates: F(q) = L*q + f0.
    let rot = rot2(transform.rotation);
    let (a_mat, a_vec) = stretch_affine(transform, w, h, hs);
    let zoom = transform.scale.max(0.01);
    // L = zoom * A * R, f0 = zoom * a.
    let l = mat_mul(&a_mat, &rot);
    let l00 = zoom * l[0];
    let l01 = zoom * l[1];
    let l10 = zoom * l[2];
    let l11 = zoom * l[3];
    let f0 = (zoom * a_vec.0, zoom * a_vec.1);

    let det = l00 * l11 - l01 * l10;
    if det.abs() < 1e-9 {
        return identity;
    }
    // Inverse of L.
    let i00 = l11 / det;
    let i01 = -l01 / det;
    let i10 = -l10 / det;
    let i11 = l00 / det;

    // Bounding box of the transformed source rect (an affine map of a rect is
    // a parallelogram: the bbox is over the four corners), padded by 1px.
    let corners = [
        (-hs.0, -hs.1),
        (w - hs.0, -hs.1),
        (-hs.0, h - hs.1),
        (w - hs.0, h - hs.1),
    ];
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    for (qx, qy) in corners {
        let dx = l00 * qx + l01 * qy + f0.0;
        let dy = l10 * qx + l11 * qy + f0.1;
        min_x = min_x.min(dx);
        max_x = max_x.max(dx);
        min_y = min_y.min(dy);
        max_y = max_y.max(dy);
    }
    min_x -= 1.0;
    min_y -= 1.0;
    max_x += 1.0;
    max_y += 1.0;

    let x0 = min_x.floor() as i32;
    let y0 = min_y.floor() as i32;
    let out_w = ((max_x.ceil() as i32) - x0).max(0) as usize;
    let out_h = ((max_y.ceil() as i32) - y0).max(0) as usize;
    if out_w == 0 || out_h == 0 || out_w > 4096 || out_h > 4096 {
        return identity;
    }

    // The pointer position maps to F-origin (0, 0); the buffer's top-left
    // corner sits at (x0, y0) relative to it.
    let new_hotspot = (-x0, -y0);

    let mut out = vec![0u8; out_w * out_h * 4];
    for dy in 0..out_h {
        let py = y0 as f32 + dy as f32 + 0.5;
        for dx in 0..out_w {
            let px = x0 as f32 + dx as f32 + 0.5;
            let rx = px - f0.0;
            let ry = py - f0.1;
            let qx = i00 * rx + i01 * ry;
            let qy = i10 * rx + i11 * ry;
            // Back to absolute image coordinates.
            let sx = qx + hs.0;
            let sy = qy + hs.1;
            let base = (dy * out_w + dx) * 4;
            if nearest {
                sample_nearest(pixels, width, height, sx, sy, &mut out[base..base + 4]);
            } else {
                sample_bilinear(pixels, width, height, sx, sy, &mut out[base..base + 4]);
            }
        }
    }

    TransformedCursorFrame {
        pixels: out,
        width: out_w,
        height: out_h,
        hotspot_x: new_hotspot.0,
        hotspot_y: new_hotspot.1,
    }
}

fn rot2(angle: f32) -> [f32; 4] {
    let (s, c) = angle.sin_cos();
    [c, -s, s, c]
}

fn mat_mul(a: &[f32; 4], b: &[f32; 4]) -> [f32; 4] {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
    ]
}

/// Affine part of the upstream stretch: rotate into the stretch axis, apply a
/// one-sided vertical shift (anchor the leading edge), scale, shift back,
/// rotate out. Returns (A, a) with ST(q) = A*q + a, both relative to the
/// hotspot. `c` is the shape center and `d = (0, h/2)` the anchor shift, both
/// in hotspot-relative coordinates.
fn stretch_affine(
    transform: &CursorTransform,
    w: f32,
    h: f32,
    hs: (f32, f32),
) -> ([f32; 4], (f32, f32)) {
    let (mx, my) = transform.stretch_magnitude;
    if (mx, my) == (1.0, 1.0) {
        return ([1.0, 0.0, 0.0, 1.0], (0.0, 0.0));
    }
    let sa = rot2(transform.stretch_angle);
    let sa_inv = rot2(-transform.stretch_angle);
    let scale = [mx, 0.0, 0.0, my];
    let a_mat = mat_mul(&sa, &mat_mul(&scale, &sa_inv));

    let c = (w * 0.5 - hs.0, h * 0.5 - hs.1);
    let d = (0.0, h * 0.5);
    // ST(q) = A*(q - c) - R(sa)*S*d + d + c
    let sd = (sa[0] * (scale[0] * d.0) + sa[1] * (scale[3] * d.1),
              sa[2] * (scale[0] * d.0) + sa[3] * (scale[3] * d.1));
    let a_c = (a_mat[0] * c.0 + a_mat[1] * c.1, a_mat[2] * c.0 + a_mat[3] * c.1);
    let a_vec = (c.0 - a_c.0 + d.0 - sd.0, c.1 - a_c.1 + d.1 - sd.1);
    (a_mat, a_vec)
}

fn sample_nearest(src: &[u8], w: usize, h: usize, x: f32, y: f32, out: &mut [u8]) {
    let px = (x - 0.5).round();
    let py = (y - 0.5).round();
    if !(0.0..w as f32).contains(&px) || !(0.0..h as f32).contains(&py) {
        out.fill(0);
        return;
    }
    let base = (py as usize * w + px as usize) * 4;
    out.copy_from_slice(&src[base..base + 4]);
}

fn sample_bilinear(src: &[u8], w: usize, h: usize, x: f32, y: f32, out: &mut [u8]) {
    let fx = x - 0.5;
    let fy = y - 0.5;
    let x0 = fx.floor();
    let y0 = fy.floor();
    let tx = fx - x0;
    let ty = fy - y0;

    let mut premul = [0.0f32; 4];
    for (ox, oy, weight) in [
        (0.0f32, 0.0f32, (1.0 - tx) * (1.0 - ty)),
        (1.0, 0.0, tx * (1.0 - ty)),
        (0.0, 1.0, (1.0 - tx) * ty),
        (1.0, 1.0, tx * ty),
    ] {
        let px = x0 + ox;
        let py = y0 + oy;
        if !(0.0..w as f32).contains(&px) || !(0.0..h as f32).contains(&py) {
            continue;
        }
        let base = (py as usize * w + px as usize) * 4;
        let b = src[base] as f32 / 255.0;
        let g = src[base + 1] as f32 / 255.0;
        let r = src[base + 2] as f32 / 255.0;
        let a = src[base + 3] as f32 / 255.0;
        premul[0] += b * a * weight;
        premul[1] += g * a * weight;
        premul[2] += r * a * weight;
        premul[3] += a * weight;
    }

    let a = premul[3];
    if a <= 0.0001 {
        out.fill(0);
        return;
    }
    // The sprite bytes are consumed as B, G, R, A in memory by both cursor
    // draw paths, so write them back in the same order.
    out[0] = to_u8(premul[0] / a);
    out[1] = to_u8(premul[1] / a);
    out[2] = to_u8(premul[2] / a);
    out[3] = to_u8(a);
}

fn to_u8(v: f32) -> u8 {
    (v * 255.0 + 0.5).clamp(0.0, 255.0) as u8
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(mode: DynamicCursorMode) -> DynamicCursorConfig {
        DynamicCursorConfig {
            enabled: true,
            mode,
            ..DynamicCursorConfig::default()
        }
    }

    #[test]
    fn activation_functions_match_upstream() {
        let max = 10.0;
        // linear
        assert!((activation(CursorActivation::Linear, max, 5.0) - 0.5).abs() < 1e-9);
        // quadratic: (1/m^2)*x^2 * sign
        assert!((activation(CursorActivation::Quadratic, max, 5.0) - 0.25).abs() < 1e-9);
        assert!((activation(CursorActivation::Quadratic, max, -5.0) + 0.25).abs() < 1e-9);
        // negative quadratic: 1 - ((x-m)^2 / m^2)
        assert!((activation(CursorActivation::NegativeQuadratic, max, 5.0) - 0.75).abs() < 1e-9);
        assert!((activation(CursorActivation::NegativeQuadratic, max, 10.0) - 1.0).abs() < 1e-9);
        assert!((activation(CursorActivation::NegativeQuadratic, max, 20.0) - 1.0).abs() < 1e-9);
        assert!((activation(CursorActivation::NegativeQuadratic, max, -5.0) + 0.75).abs() < 1e-9);
        // clamped
        assert_eq!(activation(CursorActivation::Linear, max, 100.0), 1.0);
        assert_eq!(activation(CursorActivation::Linear, max, -100.0), -1.0);
        assert_eq!(activation(CursorActivation::Linear, max, 0.0), 0.0);
    }

    #[test]
    fn movement_angle_covers_cardinal_directions() {
        let right = movement_angle(10.0, 0.0);
        let left = movement_angle(-10.0, 0.0);
        let down = movement_angle(0.0, 10.0);
        let up = movement_angle(0.0, -10.0);
        // Raw angles as computed by the upstream formula; equal mod 2PI.
        assert!((right - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
        assert!((left - 3.0 * std::f64::consts::FRAC_PI_2).abs() < 1e-9);
        assert!((down - 2.0 * std::f64::consts::PI).abs() < 1e-9);
        assert!((up - std::f64::consts::PI).abs() < 1e-9);
    }

    #[test]
    fn identity_transform_is_reported_identity() {
        assert!(CursorTransform::default().is_identity());
        assert!(!CursorTransform {
            rotation: 0.05,
            ..Default::default()
        }
        .is_identity());
    }

    #[test]
    fn transform_keeps_hotspot_fixed_for_rotation_and_zoom() {
        // 3x3 opaque white square, hotspot at (1, 1).
        let mut pixels = vec![0u8; 3 * 3 * 4];
        for p in pixels.chunks_exact_mut(4) {
            p[0] = 10;
            p[1] = 20;
            p[2] = 30;
            p[3] = 255;
        }
        let t = CursorTransform {
            rotation: 0.4,
            scale: 2.0,
            ..Default::default()
        };
        let out = transform_cursor_frame(&pixels, 3, 3, (1, 1), &t, false);
        // The buffer must have grown.
        assert!(out.width > 3 && out.height > 3);
        // The pixel covering the pointer position (the new hotspot) must be
        // opaque and match the source colour, because the hotspot maps to
        // itself under rotation and zoom.
        let base = (out.hotspot_y as usize * out.width + out.hotspot_x as usize) * 4;
        assert_eq!(out.pixels[base + 3], 255, "hotspot pixel must be opaque");
        assert_eq!(out.pixels[base], 10);
        assert_eq!(out.pixels[base + 1], 20);
        assert_eq!(out.pixels[base + 2], 30);
    }

    #[test]
    fn transform_zoom_scales_size() {
        let pixels = vec![255u8; 4 * 4 * 4];
        let t = CursorTransform {
            scale: 2.0,
            ..Default::default()
        };
        let out = transform_cursor_frame(&pixels, 4, 4, (0, 0), &t, false);
        assert_eq!(out.width, 10); // 4*2 + 2px padding
        assert_eq!(out.height, 10);
        // Center-ish pixel remains opaque.
        let base = (out.height / 2 * out.width + out.width / 2) * 4;
        assert_eq!(out.pixels[base + 3], 255);
    }

    #[test]
    fn bezier_ease_is_monotonic_and_bounded() {
        let mut last = 0.0f32;
        for i in 0..=10 {
            let t = i as f32 / 10.0;
            let e = bezier_ease(t);
            assert!((0.0..=1.0).contains(&e));
            assert!(e >= last - 1e-4);
            last = e;
        }
        assert!((bezier_ease(0.0)).abs() < 1e-4);
        assert!((bezier_ease(1.0) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn rotate_mode_follows_movement_direction() {
        let c = cfg(DynamicCursorMode::Rotate);
        let mut state = DynamicCursorState::default();
        let t0 = Instant::now();

        // First move initialises the stick upright.
        state.on_move((100.0, 100.0), (0.0, 0.0), &c, t0);
        assert_eq!(state.shown(), CursorTransform::default());

        // Moving right rotates the stick; the exact angle depends on the
        // stick state, but it must become non-zero.
        state.on_move((120.0, 100.0), (20.0, 0.0), &c, t0);
        let shown = state.shown();
        assert!(
            shown.rotation != 0.0,
            "rotate mode should produce a rotation after horizontal movement"
        );
    }

    #[test]
    fn tilt_mode_reacts_to_horizontal_speed_and_decays() {
        let c = cfg(DynamicCursorMode::Tilt);
        let mut state = DynamicCursorState::default();
        let t0 = Instant::now();

        state.on_tick((100.0, 100.0), t0, &c);
        assert_eq!(state.shown().rotation, 0.0);

        // Fast rightward movement within the window -> tilt.
        let t1 = t0 + std::time::Duration::from_millis(50);
        state.on_tick((350.0, 100.0), t1, &c);
        assert!(
            state.shown().rotation != 0.0,
            "tilt mode should tilt on horizontal movement"
        );

        // Stationary pointer for long enough -> samples drain -> no tilt.
        let mut now = t1;
        for _ in 0..40 {
            now += std::time::Duration::from_millis(50);
            state.on_tick((350.0, 100.0), now, &c);
        }
        // The shown result must have converged back to the identity.
        assert!(
            state.shown().is_identity(),
            "tilt should decay to identity when the pointer stops"
        );
        assert!(!state.animation_active(now));
    }

    #[test]
    fn stretch_mode_produces_axis_aligned_stretch_for_horizontal_motion() {
        let c = cfg(DynamicCursorMode::Stretch);
        let mut state = DynamicCursorState::default();
        let t0 = Instant::now();

        state.on_tick((100.0, 100.0), t0, &c);
        let t1 = t0 + std::time::Duration::from_millis(50);
        state.on_tick((250.0, 100.0), t1, &c);
        let shown = state.shown();
        assert!(shown.stretch_magnitude != (1.0, 1.0));
        // Rightward motion: the stretch angle is +90deg (see movement_angle).
        let normalized = (shown.stretch_angle + std::f32::consts::PI) % (2.0 * std::f32::consts::PI)
            - std::f32::consts::PI;
        assert!((normalized - std::f32::consts::FRAC_PI_2).abs() < 1e-3);
    }

    #[test]
    fn shake_detects_back_and_forth_motion() {
        let c = cfg(DynamicCursorMode::None);
        let mut state = DynamicCursorState::default();
        let mut now = Instant::now();

        // Shake: rapid back-and-forth. The bounding box diagonal is the step
        // amplitude (120px > the 100px detection floor) while the travelled
        // trail within the 1s window is huge.
        let mut x = 500.0;
        let mut dir = 1.0;
        for _ in 0..120 {
            now += std::time::Duration::from_millis(8);
            x += dir * 120.0;
            dir = -dir;
            state.on_tick((x, 500.0), now, &c);
        }
        assert!(
            state.shown().scale > 1.0,
            "shake-to-find should magnify the cursor during a shake"
        );
        assert!(state.animation_active(now));
    }

    #[test]
    fn shake_times_out_back_to_identity() {
        let c = cfg(DynamicCursorMode::None);
        let mut state = DynamicCursorState::default();
        let mut now = Instant::now();

        let mut x = 500.0;
        let mut dir = 1.0;
        for _ in 0..120 {
            now += std::time::Duration::from_millis(8);
            x += dir * 120.0;
            dir = -dir;
            state.on_tick((x, 500.0), now, &c);
        }
        assert!(state.shown().scale > 1.0);

        // Hold still past the timeout (plus the 400ms zoom animation).
        for _ in 0..100 {
            now += std::time::Duration::from_millis(50);
            state.on_tick((x, 500.0), now, &c);
        }
        assert!(
            state.shown().is_identity(),
            "shake magnification should return to identity after the timeout"
        );
    }

    #[test]
    fn warp_does_not_count_as_movement() {
        let c = cfg(DynamicCursorMode::Tilt);
        let mut state = DynamicCursorState::default();
        let t0 = Instant::now();

        state.on_tick((100.0, 100.0), t0, &c);
        // A programmatic warp: big jump with a zero relative delta.
        state.on_move((900.0, 700.0), (0.0, 0.0), &c, t0);
        let t1 = t0 + std::time::Duration::from_millis(50);
        state.on_tick((900.0, 700.0), t1, &c);
        assert!(
            state.shown().is_identity(),
            "a warp must not introduce tilt"
        );
    }

    #[test]
    fn disabled_state_stays_identity() {
        let mut c = cfg(DynamicCursorMode::Tilt);
        c.enabled = false;
        let mut state = DynamicCursorState::default();
        let mut now = Instant::now();
        let mut x = 100.0;
        for _ in 0..20 {
            now += std::time::Duration::from_millis(16);
            x += 40.0;
            state.on_tick((x, 100.0), now, &c);
        }
        assert!(state.shown().is_identity());
        assert!(!state.animation_active(now));
    }
}
