use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use smithay::desktop::Window;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Size};

use super::rules;

/// Session-local normal-size hints for clients that close while presented at
/// an output-sized geometry.
///
/// Wayland has no cross-process restore-size mechanism. Some clients persist
/// their last buffer size, including a fullscreen buffer, and then use it as
/// the next window's preferred size. Keeping the pre-presentation size lets
/// Halley give those clients one normal-sized initial configure instead.
#[derive(Debug, Default)]
pub(crate) struct PresentationCloseSizeRecovery {
    sizes: HashMap<String, Size<i32, Logical>>,
    observed_surfaces: HashSet<WlSurface>,
}

fn size_for_first_observation<K: Eq + Hash>(
    sizes: &HashMap<String, Size<i32, Logical>>,
    observed: &mut HashSet<K>,
    surface: K,
    app_id: &str,
) -> Option<Size<i32, Logical>> {
    observed
        .insert(surface)
        .then(|| sizes.get(app_id).copied())
        .flatten()
}

impl PresentationCloseSizeRecovery {
    pub(crate) fn remember(&mut self, app_id: String, size: Size<i32, Logical>) {
        if size.w <= 0 || size.h <= 0 {
            return;
        }
        self.sizes.insert(app_id, size);
    }

    #[cfg(test)]
    fn size_for(&self, app_id: &str) -> Option<Size<i32, Logical>> {
        self.sizes.get(app_id).copied()
    }

    pub(crate) fn size_for_surface(
        &mut self,
        surface: &WlSurface,
        app_id: &str,
    ) -> Option<Size<i32, Logical>> {
        size_for_first_observation(
            &self.sizes,
            &mut self.observed_surfaces,
            surface.clone(),
            app_id,
        )
    }

    pub(crate) fn forget_surface(&mut self, surface: &WlSurface) {
        self.observed_surfaces.remove(surface);
    }

    pub(crate) fn clear(&mut self, app_id: &str) {
        self.sizes.remove(app_id);
    }
}

/// Returns the stable identity used for native, independent application
/// toplevels. Transient dialogs are intentionally excluded so their close
/// history cannot affect a future main window.
pub(crate) fn independent_toplevel_app_id(window: &Window) -> Option<String> {
    let toplevel = window.toplevel()?;
    if toplevel.parent().is_some() {
        return None;
    }
    rules::identity(window).app_id
}

/// Converts a pre-presentation size into a safe future window size.
///
/// An output-sized restore is exactly the poisoned state this policy exists to
/// repair: the client previously reopened with its persisted fullscreen size,
/// so entering fullscreen again cannot reveal an older normal geometry. Match
/// the existing XWayland recovery policy and choose a bounded three-quarter
/// output fallback in that case.
pub(crate) fn presentation_close_recovery_size(
    restore: Size<i32, Logical>,
    output: Option<Size<i32, Logical>>,
) -> Size<i32, Logical> {
    let Some(output) = output.filter(|output| output.w > 0 && output.h > 0) else {
        return restore;
    };
    if restore.w < output.w || restore.h < output.h {
        return restore;
    }
    Size::from((
        (output.w.saturating_mul(3) / 4).clamp(1, output.w),
        (output.h.saturating_mul(3) / 4).clamp(1, output.h),
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use smithay::utils::{Logical, Size};

    use super::{
        PresentationCloseSizeRecovery, presentation_close_recovery_size, size_for_first_observation,
    };

    #[test]
    fn retains_a_genuine_pre_presentation_size() {
        let normal = (960, 600).into();

        assert_eq!(
            presentation_close_recovery_size(normal, Some((1920, 1200).into())),
            normal
        );
    }

    #[test]
    fn output_sized_restore_uses_a_bounded_fallback() {
        assert_eq!(
            presentation_close_recovery_size((1920, 1200).into(), Some((1920, 1200).into())),
            (1440, 900).into()
        );
    }

    #[test]
    fn restore_is_preserved_when_output_geometry_is_unavailable() {
        assert_eq!(
            presentation_close_recovery_size((960, 600).into(), None),
            (960, 600).into()
        );
    }

    #[test]
    fn hints_are_scoped_to_an_app_and_clear_after_a_normal_close() {
        let mut recovery = PresentationCloseSizeRecovery::default();
        recovery.remember("kitty".to_string(), (960, 600).into());

        assert_eq!(recovery.size_for("kitty"), Some((960, 600).into()));
        assert_eq!(recovery.size_for("other-app"), None);

        recovery.clear("kitty");
        assert_eq!(recovery.size_for("kitty"), None);
    }

    #[test]
    fn a_later_hint_never_targets_an_existing_surface() {
        let mut sizes = HashMap::new();
        let mut observed = HashSet::new();
        let recovery = Size::<i32, Logical>::from((960, 600));

        assert_eq!(
            size_for_first_observation(&sizes, &mut observed, 1_u64, "kitty"),
            None
        );

        sizes.insert("kitty".to_string(), recovery);

        assert_eq!(
            size_for_first_observation(&sizes, &mut observed, 1_u64, "kitty"),
            None
        );
    }

    #[test]
    fn a_hint_targets_each_future_surface_only_once() {
        let recovery = Size::<i32, Logical>::from((960, 600));
        let sizes = HashMap::from([("kitty".to_string(), recovery)]);
        let mut observed = HashSet::new();

        assert_eq!(
            size_for_first_observation(&sizes, &mut observed, 1_u64, "kitty"),
            Some(recovery)
        );
        assert_eq!(
            size_for_first_observation(&sizes, &mut observed, 1_u64, "kitty"),
            None
        );
        assert_eq!(
            size_for_first_observation(&sizes, &mut observed, 2_u64, "kitty"),
            Some(recovery)
        );
    }

    #[test]
    fn observing_without_a_matching_app_hint_still_marks_the_surface_seen() {
        let recovery = Size::<i32, Logical>::from((1200, 800));
        let mut sizes = HashMap::from([("firefox".to_string(), recovery)]);
        let mut observed = HashSet::new();

        assert_eq!(
            size_for_first_observation(&sizes, &mut observed, 1_u64, "kitty"),
            None
        );

        sizes.insert("kitty".to_string(), recovery);

        assert_eq!(
            size_for_first_observation(&sizes, &mut observed, 1_u64, "kitty"),
            None
        );
    }
}
