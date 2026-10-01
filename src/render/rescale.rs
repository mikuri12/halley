use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{GlesError, GlesFrame, GlesRenderer};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::utils::{
    Buffer, Physical, Point, Rectangle, Scale, Transform, user_data::UserDataMap,
};

/// Wraps a `WaylandSurfaceRenderElement` so it draws into an arbitrary
/// destination rect instead of its native buffer size.
///
/// This is the actual mechanism that makes window *content* shrink during
/// zoom - not the `Scale` parameter passed to `Window::render_elements`.
/// That parameter looked like the right knob but isn't: Smithay's own
/// `WaylandSurfaceRenderElement::geometry(&self, scale)` recomputes its size
/// from whatever `scale` is passed at *query* time (confirmed by reading
/// `smithay/src/backend/renderer/element/surface.rs`), and the generic
/// `draw_render_elements` driver both backends use always queries geometry
/// at `1.0` (matching this project's choice not to touch the
/// protocol-visible output scale) - so a `zoom_scale` passed at construction
/// time never survives to the actual draw call. Old halley hit this same
/// wall and solved it with a custom render element
/// (`halley-wl/src/render/rescale.rs::RescaledSurfaceElement`) that hardcodes
/// its `geometry()` to a precomputed rect, ignoring the query scale
/// entirely.
///
/// This is a stripped-down port of that idea: old halley's version also
/// handled corner-radius clipping (a shader + uniforms) and per-subsurface
/// seam-precision via a base-window/visual-window relative remapping -
/// neither applies here (no rounded corners, and `zoom_scale` is a single
/// global scalar applied via the same affine transform to every element, so
/// relative positions between a window's own render elements stay
/// consistent without that extra machinery). `draw()` doesn't need to know
/// about textures vs. solid colors either, unlike old halley's version -
/// `RenderElement::draw`'s `src`/`dst` are explicit arguments the driver
/// computes from `geometry()`/`src()`, so simply forwarding them to the
/// inner element's own `draw()` is the entire fix.
pub struct RescaledElement {
    inner: WaylandSurfaceRenderElement<GlesRenderer>,
    dst: Rectangle<i32, Physical>,
}

impl RescaledElement {
    pub fn new(
        inner: WaylandSurfaceRenderElement<GlesRenderer>,
        dst: Rectangle<i32, Physical>,
    ) -> Self {
        Self { inner, dst }
    }
}

impl Element for RescaledElement {
    fn id(&self) -> &Id {
        self.inner.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.dst
    }

    fn location(&self, _scale: Scale<f64>) -> Point<i32, Physical> {
        self.dst.loc
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.inner.src()
    }

    fn transform(&self) -> Transform {
        self.inner.transform()
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }

    fn kind(&self) -> Kind {
        // A hardware plane would bypass zoom and any backdrop blur behind
        // the window, which shows up as the surface flickering between
        // scan-out and composition.
        Kind::Unspecified
    }

    // `damage_since` uses `Element`'s default impl, which derives from
    // `geometry()` - already correct once `geometry()` is overridden above.
    // `opaque_regions`' default reports *none*, deliberately left as-is: the
    // inner element's own regions are in unscaled surface coordinates, so
    // forwarding them would over-report coverage whenever `zoom_scale < 1.0`
    // and let `draw_render_elements` cull windows that are actually visible.
    // Reporting none only costs some overdraw, which neither backend
    // optimizes for anyway (both clear + redraw the full output every frame).
}

impl RenderElement<GlesRenderer> for RescaledElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        // `src`/`dst`/`damage` were already computed by the caller from this
        // element's own `geometry()`/`src()` (see `draw_render_elements`),
        // so they're already correct - just forward them to the inner
        // element, which is what actually issues the scaled GL draw call.
        self.inner
            .draw(frame, src, dst, damage, opaque_regions, cache)
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}
