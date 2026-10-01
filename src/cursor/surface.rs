use std::time::Duration;

use smithay::desktop::Space;
use smithay::desktop::utils::{bbox_from_surface_tree, output_update, send_frames_surface_tree};
use smithay::input::pointer::CursorImageSurfaceData;
use smithay::output::{self, Output};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Transform};
use smithay::wayland::compositor::{
    SurfaceAttributes, SurfaceData, send_surface_state as send_compositor_surface_state,
    with_states,
};
use smithay::wayland::fractional_scale::with_fractional_scale;

use super::CursorManager;

pub fn hotspot(surface: &WlSurface) -> Point<i32, Logical> {
    with_states(surface, |states| {
        states
            .data_map
            .get::<CursorImageSurfaceData>()
            .and_then(|data| data.lock().ok().map(|data| data.hotspot))
            .unwrap_or_default()
    })
}

pub fn handle_commit(manager: &CursorManager, committed: &WlSurface, root: &WlSurface) -> bool {
    // Presentation overrides must not make us lose hotspot deltas committed
    // by the client underneath them.
    if manager.client_surface() != Some(root) {
        return false;
    }
    if committed == root {
        with_states(root, |states| {
            let delta = states
                .cached_state
                .get::<SurfaceAttributes>()
                .current()
                .buffer_delta
                .take();
            if let Some(delta) = delta
                && let Some(attributes) = states.data_map.get::<CursorImageSurfaceData>()
                && let Ok(mut attributes) = attributes.lock()
            {
                attributes.hotspot -= delta;
            }
        });
    }
    true
}

pub fn clear_outputs(surface: &WlSurface, space: &Space<smithay::desktop::Window>) {
    for output in space.outputs() {
        output_update(output, None, surface);
    }
}

pub fn refresh_outputs(
    manager: &CursorManager,
    space: &Space<smithay::desktop::Window>,
    pointer_position: (f64, f64),
) {
    // Keep output enter/leave and preferred scale current even while a shell
    // or grab temporarily replaces the client's cursor image.
    let Some(surface) = manager.client_surface() else {
        return;
    };
    let surface_position = Point::<i32, Logical>::from((
        pointer_position.0.round() as i32,
        pointer_position.1.round() as i32,
    )) - hotspot(surface);
    let bounds = bbox_from_surface_tree(surface, surface_position);
    let mut preferred_scale = 1.0_f64;
    let mut preferred_transform = Transform::Normal;

    for output in space.outputs() {
        let Some(geometry) = space.output_geometry(output) else {
            continue;
        };
        if let Some(mut overlap) = geometry.intersection(bounds) {
            overlap.loc -= surface_position;
            output_update(output, Some(overlap), surface);
            let scale = output.current_scale().fractional_scale();
            if scale >= preferred_scale {
                preferred_scale = scale;
                preferred_transform = output.current_transform();
            }
        } else {
            output_update(output, None, surface);
        }
    }

    with_states(surface, |data| {
        send_scale_transform(
            surface,
            data,
            output::Scale::Fractional(preferred_scale),
            preferred_transform,
        );
    });
}

pub fn send_frame(
    manager: &CursorManager,
    space: &Space<smithay::desktop::Window>,
    output: &Output,
    pointer_position: (f64, f64),
    elapsed: Duration,
    sequence: u32,
) {
    let Some(surface) = manager.current_surface() else {
        return;
    };
    let Some(geometry) = space.output_geometry(output) else {
        return;
    };
    if !geometry
        .to_f64()
        .contains(Point::<f64, Logical>::from(pointer_position))
    {
        return;
    }
    send_frames_surface_tree(
        surface,
        output,
        elapsed,
        crate::wayland::frame_callbacks::FALLBACK_THROTTLE,
        |surface, states| {
            crate::wayland::frame_callbacks::callback_output(
                surface, states, output, sequence, false,
            )
        },
    );
}

fn send_scale_transform(
    surface: &WlSurface,
    data: &SurfaceData,
    scale: output::Scale,
    transform: Transform,
) {
    send_compositor_surface_state(surface, data, scale.integer_scale(), transform);
    with_fractional_scale(data, |fractional| {
        fractional.set_preferred_scale(scale.fractional_scale());
    });
}
