use crate::field::{Rect, Vec2};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    /// Center position in Field coordinates.
    pub center: Vec2,

    /// Size of the visible region in Field coordinates.
    pub size: Vec2,

    /// Home position for Return.
    pub home: Vec2,
}

impl Viewport {
    pub fn new(center: Vec2, size: Vec2) -> Self {
        Self {
            center,
            size,
            home: center,
        }
    }

    /// Axis-aligned view rectangle in Field space.
    pub fn rect(&self) -> Rect {
        let half = Vec2 {
            x: self.size.x * 0.5,
            y: self.size.y * 0.5,
        };

        Rect {
            min: Vec2 {
                x: self.center.x - half.x,
                y: self.center.y - half.y,
            },
            max: Vec2 {
                x: self.center.x + half.x,
                y: self.center.y + half.y,
            },
        }
    }

    /// Move camera to a new center.
    pub fn move_to(&mut self, center: Vec2) {
        self.center = center;
    }

    /// Offset camera by delta.
    pub fn pan(&mut self, delta: Vec2) {
        self.center.x += delta.x;
        self.center.y += delta.y;
    }

    /// Set current position as home.
    pub fn set_home(&mut self) {
        self.home = self.center;
    }

    /// Return to home position.
    pub fn return_home(&mut self) {
        self.center = self.home;
    }
}

/// Which focus zone a point is in (relative to a viewport center).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusZone {
    Inside,
    Outside,
}

/// A focus ring modeled as an axis-aligned ellipse in Field coordinates,
/// with an offset relative to the viewport center.
///
/// We use normalized ellipse distance:
///   d2 = (x/rx)^2 + (y/ry)^2
/// If d2 <= 1 => inside.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FocusRing {
    pub radius_x: f32,
    pub radius_y: f32,
    pub offset_x: f32,
    pub offset_y: f32,
}

impl FocusRing {
    pub fn new(radius_x: f32, radius_y: f32, offset_x: f32, offset_y: f32) -> Self {
        Self {
            radius_x,
            radius_y,
            offset_x,
            offset_y,
        }
    }

    pub fn contains(&self, center: Vec2, p: Vec2) -> bool {
        self.normalized_distance2(center, p) <= 1.0
    }

    pub fn zone(&self, vp_center: Vec2, p: Vec2) -> FocusZone {
        if self.contains(vp_center, p) {
            FocusZone::Inside
        } else {
            FocusZone::Outside
        }
    }

    /// Return normalized squared distance inside this ellipse:
    /// d2 = (x/rx)^2 + (y/ry)^2
    /// - d2 <= 1.0: inside/on boundary
    /// - d2 > 1.0: outside
    pub fn normalized_distance2(&self, center: Vec2, p: Vec2) -> f32 {
        let ring_center = Vec2 {
            x: center.x + self.offset_x,
            y: center.y + self.offset_y,
        };

        let dx = p.x - ring_center.x;
        let dy = p.y - ring_center.y;

        let rx = self.radius_x.max(0.0001);
        let ry = self.radius_y.max(0.0001);

        let nx = dx / rx;
        let ny = dy / ry;

        nx * nx + ny * ny
    }

    /// Area-sampling variant of `zone()`: classifies which zone a whole
    /// footprint rect (not just a single point) dominantly falls in, by
    /// sampling a 5x5 grid across it and taking a majority vote. Moved here
    /// from `decay.rs` (as `dominant_focus_zone`) - it's geometry/focus-ring
    /// classification, not decay policy; decay just happened to be the only
    /// consumer.
    pub fn dominant_zone(&self, vp_center: Vec2, pos: Vec2, footprint: Vec2) -> FocusZone {
        let w = footprint.x.abs();
        let h = footprint.y.abs();

        if w < 1.0 || h < 1.0 {
            return self.zone(vp_center, pos);
        }

        let sx = 5usize;
        let sy = 5usize;
        let mut inside = 0usize;

        let min_x = pos.x - w * 0.5;
        let min_y = pos.y - h * 0.5;

        for iy in 0..sy {
            for ix in 0..sx {
                let tx = (ix as f32 + 0.5) / sx as f32;
                let ty = (iy as f32 + 0.5) / sy as f32;
                let p = Vec2 {
                    x: min_x + tx * w,
                    y: min_y + ty * h,
                };

                if self.zone(vp_center, p) == FocusZone::Inside {
                    inside += 1;
                }
            }
        }

        let total = (sx * sy) as f32;
        let frac_inside = inside as f32 / total;

        if frac_inside > 0.5 {
            FocusZone::Inside
        } else {
            FocusZone::Outside
        }
    }

    /// Fraction of a rectangular footprint that falls outside this ring,
    /// sampled at the center of a 10x10 grid. The denser grid makes a 90%
    /// outside threshold stable and deterministic without pretending an
    /// ellipse/rectangle intersection is rectangular.
    pub fn outside_fraction(&self, vp_center: Vec2, pos: Vec2, footprint: Vec2) -> f32 {
        let w = footprint.x.abs();
        let h = footprint.y.abs();
        if w < 1.0 || h < 1.0 {
            return if self.contains(vp_center, pos) {
                0.0
            } else {
                1.0
            };
        }

        const SAMPLES: usize = 10;
        let min_x = pos.x - w * 0.5;
        let min_y = pos.y - h * 0.5;
        let mut outside = 0usize;
        for iy in 0..SAMPLES {
            for ix in 0..SAMPLES {
                let p = Vec2 {
                    x: min_x + (ix as f32 + 0.5) / SAMPLES as f32 * w,
                    y: min_y + (iy as f32 + 0.5) / SAMPLES as f32 * h,
                };
                outside += usize::from(!self.contains(vp_center, p));
            }
        }
        outside as f32 / (SAMPLES * SAMPLES) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_is_correct() {
        let vp = Viewport::new(Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 100.0, y: 50.0 });

        let r = vp.rect();

        assert_eq!(r.min, Vec2 { x: -50.0, y: -25.0 });
        assert_eq!(r.max, Vec2 { x: 50.0, y: 25.0 });
    }

    #[test]
    fn return_home_works() {
        let mut vp = Viewport::new(Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 100.0, y: 50.0 });

        vp.pan(Vec2 { x: 10.0, y: 5.0 });
        assert_eq!(vp.center, Vec2 { x: 10.0, y: 5.0 });

        vp.return_home();
        assert_eq!(vp.center, Vec2 { x: 0.0, y: 0.0 });
    }

    #[test]
    fn focus_ring_contains_axis_aligned() {
        let ring = FocusRing::new(10.0, 5.0, 0.0, 0.0);
        let c = Vec2 { x: 0.0, y: 0.0 };

        assert!(ring.contains(c, Vec2 { x: 0.0, y: 0.0 }));
        assert!(ring.contains(c, Vec2 { x: 10.0, y: 0.0 }));
        assert!(ring.contains(c, Vec2 { x: 0.0, y: 5.0 }));

        assert!(!ring.contains(c, Vec2 { x: 10.01, y: 0.0 }));
        assert!(!ring.contains(c, Vec2 { x: 0.0, y: 5.01 }));
    }

    #[test]
    fn focus_ring_respects_offset() {
        let ring = FocusRing::new(10.0, 5.0, 4.0, -2.0);
        let c = Vec2 { x: 0.0, y: 0.0 };

        assert!(ring.contains(c, Vec2 { x: 4.0, y: -2.0 }));
        assert!(ring.contains(c, Vec2 { x: 14.0, y: -2.0 }));
        assert!(ring.contains(c, Vec2 { x: 4.0, y: 3.0 }));

        assert!(!ring.contains(c, Vec2 { x: 14.01, y: -2.0 }));
        assert!(!ring.contains(c, Vec2 { x: 4.0, y: 3.01 }));
    }

    #[test]
    fn focus_zone_classifies() {
        let ring = FocusRing::new(10.0, 10.0, 0.0, 0.0);
        let c = Vec2 { x: 0.0, y: 0.0 };

        assert_eq!(ring.zone(c, Vec2 { x: 0.0, y: 0.0 }), FocusZone::Inside);
        assert_eq!(ring.zone(c, Vec2 { x: 20.0, y: 0.0 }), FocusZone::Outside);
    }

    #[test]
    fn dominant_zone_majority_votes_across_footprint() {
        let ring = FocusRing::new(50.0, 30.0, 0.0, 0.0);
        let c = Vec2 { x: 0.0, y: 0.0 };

        // Small footprint centered inside the ring: dominant zone is Inside.
        assert_eq!(
            ring.dominant_zone(c, Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 }),
            FocusZone::Inside
        );

        // Small footprint far outside the ring: dominant zone is Outside.
        assert_eq!(
            ring.dominant_zone(c, Vec2 { x: 500.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 }),
            FocusZone::Outside
        );

        // A sub-1px-wide/tall footprint falls back to a plain point check.
        assert_eq!(
            ring.dominant_zone(c, Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 0.0, y: 0.0 }),
            FocusZone::Inside
        );
    }
}
