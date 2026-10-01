use smithay::utils::{Physical, Point, Rectangle};

use super::{VisualRect, motion::MotionSample};

const MAX_TRAVEL: f64 = 320.0;
const MAX_ARC: f64 = 24.0;
const MAX_TRAVEL_PROGRESS: f64 = 1.08;
const START_SCALE: f64 = 0.8;
const OVERSHOOT_SCALE: f64 = 1.02;
const OVERSHOOT_PROGRESS: f64 = 0.78;
pub(super) const START_ALPHA: f32 = 0.15;
const OPAQUE_PROGRESS: f64 = 0.65;

pub(super) fn rect(
    bounds: Rectangle<i32, Physical>,
    origin: Option<Point<f64, Physical>>,
    sample: MotionSample,
) -> VisualRect {
    rect_and_velocity(bounds, origin, sample).0
}

pub(super) fn path_rect(
    bounds: Rectangle<i32, Physical>,
    origin: Option<Point<f64, Physical>>,
    sample: MotionSample,
) -> VisualRect {
    path_rect_and_velocity(bounds, origin, sample).0
}

pub(super) fn rect_and_velocity(
    bounds: Rectangle<i32, Physical>,
    origin: Option<Point<f64, Physical>>,
    sample: MotionSample,
) -> (VisualRect, VisualRect) {
    sized_rect_and_velocity(bounds, origin, sample, true)
}

pub(super) fn path_rect_and_velocity(
    bounds: Rectangle<i32, Physical>,
    origin: Option<Point<f64, Physical>>,
    sample: MotionSample,
) -> (VisualRect, VisualRect) {
    sized_rect_and_velocity(bounds, origin, sample, false)
}

fn sized_rect_and_velocity(
    bounds: Rectangle<i32, Physical>,
    origin: Option<Point<f64, Physical>>,
    sample: MotionSample,
    apply_scale: bool,
) -> (VisualRect, VisualRect) {
    let end = rect_center(bounds);
    let origin = origin.unwrap_or(end);
    let delta = origin - end;
    let distance = delta.x.hypot(delta.y);
    let start = if distance > MAX_TRAVEL {
        let scale = MAX_TRAVEL / distance;
        end + Point::from((delta.x * scale, delta.y * scale))
    } else {
        origin
    };
    let travel_delta = start - end;
    let travel = travel_delta.x.hypot(travel_delta.y);
    let arc = (travel * 0.08).min(MAX_ARC);
    let control = Point::from(((start.x + end.x) / 2.0, (start.y + end.y) / 2.0 - arc));
    let path_progress = sample.value.clamp(0.0, MAX_TRAVEL_PROGRESS);
    let path_progress_velocity = if (0.0..=MAX_TRAVEL_PROGRESS).contains(&sample.value) {
        sample.velocity
    } else {
        0.0
    };
    let center = quadratic_point(start, control, end, path_progress);
    let path_velocity = quadratic_derivative(start, control, end, path_progress);
    let center_derivative = Point::<f64, Physical>::from((
        path_velocity.x * path_progress_velocity,
        path_velocity.y * path_progress_velocity,
    ));
    let (scale, scale_derivative) = if apply_scale {
        scale(sample.linear_progress)
    } else {
        (1.0, 0.0)
    };
    let width = f64::from(bounds.size.w) * scale;
    let height = f64::from(bounds.size.h) * scale;
    let width_derivative = f64::from(bounds.size.w) * scale_derivative * sample.linear_velocity;
    let height_derivative = f64::from(bounds.size.h) * scale_derivative * sample.linear_velocity;
    (
        VisualRect {
            x: center.x - width / 2.0,
            y: center.y - height / 2.0,
            width,
            height,
        },
        VisualRect {
            x: center_derivative.x - width_derivative / 2.0,
            y: center_derivative.y - height_derivative / 2.0,
            width: width_derivative,
            height: height_derivative,
        },
    )
}

pub(super) fn alpha(progress: f64) -> f32 {
    let progress = (progress / OPAQUE_PROGRESS).clamp(0.0, 1.0);
    START_ALPHA + (1.0 - START_ALPHA) * ease_out_cubic(progress) as f32
}

pub(super) fn rect_center(rect: Rectangle<i32, Physical>) -> Point<f64, Physical> {
    Point::from((
        f64::from(rect.loc.x) + f64::from(rect.size.w) / 2.0,
        f64::from(rect.loc.y) + f64::from(rect.size.h) / 2.0,
    ))
}

fn quadratic_point(
    start: Point<f64, Physical>,
    control: Point<f64, Physical>,
    end: Point<f64, Physical>,
    progress: f64,
) -> Point<f64, Physical> {
    let remaining = 1.0 - progress;
    Point::from((
        remaining.powi(2) * start.x
            + 2.0 * remaining * progress * control.x
            + progress.powi(2) * end.x,
        remaining.powi(2) * start.y
            + 2.0 * remaining * progress * control.y
            + progress.powi(2) * end.y,
    ))
}

fn quadratic_derivative(
    start: Point<f64, Physical>,
    control: Point<f64, Physical>,
    end: Point<f64, Physical>,
    progress: f64,
) -> Point<f64, Physical> {
    Point::from((
        2.0 * (1.0 - progress) * (control.x - start.x) + 2.0 * progress * (end.x - control.x),
        2.0 * (1.0 - progress) * (control.y - start.y) + 2.0 * progress * (end.y - control.y),
    ))
}

fn scale(progress: f64) -> (f64, f64) {
    if progress <= OVERSHOOT_PROGRESS {
        let segment = (progress / OVERSHOOT_PROGRESS).clamp(0.0, 1.0);
        let range = OVERSHOOT_SCALE - START_SCALE;
        (
            START_SCALE + range * smoothstep(segment),
            range * smoothstep_derivative(segment) / OVERSHOOT_PROGRESS,
        )
    } else {
        let duration = 1.0 - OVERSHOOT_PROGRESS;
        let segment = ((progress - OVERSHOOT_PROGRESS) / duration).clamp(0.0, 1.0);
        let range = 1.0 - OVERSHOOT_SCALE;
        (
            OVERSHOOT_SCALE + range * smoothstep(segment),
            range * smoothstep_derivative(segment) / duration,
        )
    }
}

fn ease_out_cubic(progress: f64) -> f64 {
    1.0 - (1.0 - progress.clamp(0.0, 1.0)).powi(3)
}

fn smoothstep(progress: f64) -> f64 {
    let progress = progress.clamp(0.0, 1.0);
    progress * progress * (3.0 - 2.0 * progress)
}

fn smoothstep_derivative(progress: f64) -> f64 {
    let progress = progress.clamp(0.0, 1.0);
    6.0 * progress * (1.0 - progress)
}
