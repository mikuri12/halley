use std::collections::BTreeSet;

use smithay::utils::{Logical, Point, Rectangle, Size};
use smithay::xwayland::X11Surface;
use x11rb::properties::{AspectRatio, WmSizeHints};

#[derive(Clone, Copy, Debug)]
struct Ratio {
    numerator: i64,
    denominator: i64,
}

impl Ratio {
    fn new(value: AspectRatio) -> Option<Self> {
        (value.numerator > 0 && value.denominator > 0).then_some(Self {
            numerator: i64::from(value.numerator),
            denominator: i64::from(value.denominator),
        })
    }
}

#[derive(Clone, Copy, Debug)]
struct AxisPolicy {
    lower: i32,
    upper: i32,
    base: i32,
    increment: i32,
}

impl AxisPolicy {
    fn new(minimum: Option<i32>, maximum: Option<i32>, base: i32, increment: Option<i32>) -> Self {
        let lower = minimum.filter(|value| *value > 0).unwrap_or(1);
        let upper = maximum.filter(|value| *value >= lower).unwrap_or(i32::MAX);
        let (base, increment) = increment
            .filter(|value| *value > 0)
            .map(|increment| (base.clamp(0, upper), increment))
            .unwrap_or((0, 1));
        Self {
            lower,
            upper,
            base,
            increment,
        }
    }

    fn floor(self, requested: i64) -> i32 {
        self.project(requested, false)
    }

    fn ceil(self, requested: i64) -> i32 {
        self.project(requested, true)
    }

    fn project(self, requested: i64, upwards: bool) -> i32 {
        let base = i64::from(self.base);
        let increment = i64::from(self.increment);
        let minimum_index = div_ceil(i64::from(self.lower) - base, increment).max(0);
        let maximum_index = (i64::from(self.upper) - base).div_euclid(increment);
        if maximum_index < minimum_index {
            return self.lower;
        }
        let offset = requested.clamp(i64::from(self.lower), i64::from(self.upper)) - base;
        let index = if upwards {
            div_ceil(offset, increment)
        } else {
            offset.div_euclid(increment)
        }
        .clamp(minimum_index, maximum_index);
        i32::try_from(base + index * increment).unwrap_or(self.upper)
    }

    fn seeds(self, preferred: i32) -> impl Iterator<Item = i32> {
        let mut values = [preferred, self.floor(i64::from(self.lower)), preferred];
        if self.upper != i32::MAX {
            values[2] = self.floor(i64::from(self.upper));
        }
        values.into_iter()
    }
}

#[derive(Clone, Copy, Debug)]
struct SizePolicy {
    width: AxisPolicy,
    height: AxisPolicy,
    aspect_base: Size<i32, Logical>,
    minimum_aspect: Option<Ratio>,
    maximum_aspect: Option<Ratio>,
}

impl SizePolicy {
    fn from_hints(hints: Option<WmSizeHints>) -> Self {
        let hints = hints.unwrap_or_default();
        let base = hints.base_size.or(hints.min_size).unwrap_or((0, 0));
        let minimum = hints.min_size.or(hints.base_size);
        let increment = hints.size_increment;
        let (minimum_aspect, maximum_aspect) = hints
            .aspect
            .map(|(minimum, maximum)| (Ratio::new(minimum), Ratio::new(maximum)))
            .unwrap_or((None, None));
        let valid_aspect_range =
            minimum_aspect
                .zip(maximum_aspect)
                .is_none_or(|(minimum, maximum)| {
                    minimum.numerator * maximum.denominator
                        <= maximum.numerator * minimum.denominator
                });

        Self {
            width: AxisPolicy::new(
                minimum.map(|size| size.0),
                hints.max_size.map(|size| size.0),
                base.0.max(0),
                increment.map(|size| size.0),
            ),
            height: AxisPolicy::new(
                minimum.map(|size| size.1),
                hints.max_size.map(|size| size.1),
                base.1.max(0),
                increment.map(|size| size.1),
            ),
            aspect_base: Size::from((base.0.max(0), base.1.max(0))),
            minimum_aspect: valid_aspect_range.then_some(minimum_aspect).flatten(),
            maximum_aspect: valid_aspect_range.then_some(maximum_aspect).flatten(),
        }
    }

    fn constrain(self, requested: Size<i32, Logical>) -> Size<i32, Logical> {
        let preferred = Size::from((
            self.width.floor(i64::from(requested.w.max(1))),
            self.height.floor(i64::from(requested.h.max(1))),
        ));
        if self.aspect_valid(preferred) {
            return preferred;
        }

        let mut candidates = BTreeSet::new();
        candidates.insert((preferred.w, preferred.h));
        for height in self.height.seeds(preferred.h) {
            self.add_width_candidates(height, self.minimum_aspect, &mut candidates);
            self.add_width_candidates(height, self.maximum_aspect, &mut candidates);
        }
        for width in self.width.seeds(preferred.w) {
            self.add_height_candidates(width, self.minimum_aspect, &mut candidates);
            self.add_height_candidates(width, self.maximum_aspect, &mut candidates);
        }

        candidates
            .into_iter()
            .map(Size::from)
            .filter(|size| self.aspect_valid(*size))
            .min_by_key(|size| size_distance(*size, requested))
            .unwrap_or(preferred)
    }

    fn add_width_candidates(
        self,
        height: i32,
        ratio: Option<Ratio>,
        candidates: &mut BTreeSet<(i32, i32)>,
    ) {
        let Some(ratio) = ratio else {
            return;
        };
        let content_height = i64::from(height - self.aspect_base.h).max(0);
        let numerator = content_height * ratio.numerator;
        let target = i64::from(self.aspect_base.w) + numerator.div_euclid(ratio.denominator);
        let target_ceil = i64::from(self.aspect_base.w) + div_ceil(numerator, ratio.denominator);
        candidates.insert((self.width.floor(target), height));
        candidates.insert((self.width.ceil(target_ceil), height));
    }

    fn add_height_candidates(
        self,
        width: i32,
        ratio: Option<Ratio>,
        candidates: &mut BTreeSet<(i32, i32)>,
    ) {
        let Some(ratio) = ratio else {
            return;
        };
        let content_width = i64::from(width - self.aspect_base.w).max(0);
        let numerator = content_width * ratio.denominator;
        let target = i64::from(self.aspect_base.h) + numerator.div_euclid(ratio.numerator);
        let target_ceil = i64::from(self.aspect_base.h) + div_ceil(numerator, ratio.numerator);
        candidates.insert((width, self.height.floor(target)));
        candidates.insert((width, self.height.ceil(target_ceil)));
    }

    fn aspect_valid(self, size: Size<i32, Logical>) -> bool {
        let width = i64::from(size.w - self.aspect_base.w).max(0);
        let height = i64::from(size.h - self.aspect_base.h).max(0);
        let minimum_valid = self
            .minimum_aspect
            .is_none_or(|ratio| width * ratio.denominator >= height * ratio.numerator);
        let maximum_valid = self
            .maximum_aspect
            .is_none_or(|ratio| width * ratio.denominator <= height * ratio.numerator);
        minimum_valid && maximum_valid
    }
}

fn div_ceil(numerator: i64, denominator: i64) -> i64 {
    let quotient = numerator.div_euclid(denominator);
    quotient + i64::from(numerator.rem_euclid(denominator) != 0)
}

fn size_distance(left: Size<i32, Logical>, right: Size<i32, Logical>) -> i64 {
    (i64::from(left.w) - i64::from(right.w)).abs() + (i64::from(left.h) - i64::from(right.h)).abs()
}

pub(super) fn constrain_surface_size(
    surface: &X11Surface,
    requested: Size<i32, Logical>,
) -> Size<i32, Logical> {
    // Halley fixes the XWayland client scale at 1.0 during startup, so the raw
    // WM_NORMAL_HINTS dimensions use the same logical units as `requested`.
    SizePolicy::from_hints(surface.size_hints()).constrain(requested)
}

pub(super) fn requested_geometry(
    surface: &X11Surface,
    x: Option<i32>,
    y: Option<i32>,
    width: Option<u32>,
    height: Option<u32>,
) -> Rectangle<i32, Logical> {
    let current = surface.geometry();
    let location = requested_location(current.loc, x, y, surface.is_transient_for().is_some());
    let requested_size = Size::from((
        width.map(saturating_i32).unwrap_or(current.size.w),
        height.map(saturating_i32).unwrap_or(current.size.h),
    ));
    let size = if width.is_some() || height.is_some() {
        constrain_surface_size(surface, requested_size)
    } else {
        current.size
    };
    Rectangle::new(location, size)
}

fn requested_location(
    current: Point<i32, Logical>,
    x: Option<i32>,
    y: Option<i32>,
    honor_client_position: bool,
) -> Point<i32, Logical> {
    if honor_client_position {
        Point::from((x.unwrap_or(current.x), y.unwrap_or(current.y)))
    } else {
        current
    }
}

fn saturating_i32(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX).max(1)
}

#[cfg(test)]
mod tests {
    use super::{SizePolicy, requested_location};
    use smithay::utils::{Logical, Point, Size};
    use x11rb::properties::{AspectRatio, WmSizeHints};

    fn constrain(hints: WmSizeHints, requested: (i32, i32)) -> Size<i32, Logical> {
        SizePolicy::from_hints(Some(hints)).constrain(Size::from(requested))
    }

    #[test]
    fn minimum_and_maximum_bounds_are_enforced() {
        let hints = WmSizeHints {
            min_size: Some((320, 200)),
            max_size: Some((1280, 720)),
            ..Default::default()
        };

        assert_eq!(constrain(hints, (10, 1000)), Size::from((320, 720)));
    }

    #[test]
    fn increments_form_a_progression_from_base_size() {
        let hints = WmSizeHints {
            min_size: Some((10, 10)),
            base_size: Some((10, 10)),
            size_increment: Some((8, 6)),
            ..Default::default()
        };

        assert_eq!(constrain(hints, (35, 31)), Size::from((34, 28)));
    }

    #[test]
    fn minimum_size_is_the_default_base_size() {
        let hints = WmSizeHints {
            min_size: Some((100, 50)),
            size_increment: Some((20, 10)),
            ..Default::default()
        };

        assert_eq!(constrain(hints, (139, 79)), Size::from((120, 70)));
    }

    #[test]
    fn explicit_unit_increment_still_starts_at_base_size() {
        let hints = WmSizeHints {
            min_size: Some((10, 10)),
            base_size: Some((20, 20)),
            size_increment: Some((1, 1)),
            ..Default::default()
        };

        assert_eq!(constrain(hints, (15, 15)), Size::from((20, 20)));
    }

    #[test]
    fn aspect_ratio_applies_to_the_area_above_base_size() {
        let square = AspectRatio::new(1, 1);
        let hints = WmSizeHints {
            base_size: Some((20, 10)),
            min_size: Some((20, 10)),
            aspect: Some((square, square)),
            ..Default::default()
        };

        assert_eq!(constrain(hints, (120, 80)), Size::from((90, 80)));
    }

    #[test]
    fn aspect_range_selects_the_nearest_valid_dimension() {
        let hints = WmSizeHints {
            aspect: Some((AspectRatio::new(4, 3), AspectRatio::new(16, 9))),
            ..Default::default()
        };

        assert_eq!(constrain(hints, (200, 200)), Size::from((200, 150)));
        assert_eq!(constrain(hints, (400, 100)), Size::from((400, 225)));
    }

    #[test]
    fn invalid_ranges_and_increments_degrade_to_safe_bounds() {
        let hints = WmSizeHints {
            min_size: Some((100, 80)),
            max_size: Some((40, 20)),
            size_increment: Some((0, -4)),
            aspect: Some((AspectRatio::new(16, 0), AspectRatio::new(1, 1))),
            ..Default::default()
        };

        assert_eq!(constrain(hints, (1, 1)), Size::from((100, 80)));
    }

    #[test]
    fn only_transient_policy_honors_client_position_requests() {
        let current = Point::<i32, Logical>::from((40, 60));
        assert_eq!(
            requested_location(current, Some(100), Some(120), false),
            current
        );
        assert_eq!(
            requested_location(current, Some(100), None, true),
            Point::from((100, 60))
        );
    }
}
