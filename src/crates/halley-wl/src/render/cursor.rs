use smithay::{
    backend::renderer::{Color32F, Frame},
    input::pointer::{CursorImageStatus, CursorImageSurfaceData},
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Physical, Rectangle},
    wayland::compositor::{get_parent, with_states, SurfaceAttributes},
};

use super::cursor_theme::SoftwareCursorSprite;
use super::draw_primitives::draw_rect;
use super::dynamic_cursor::{
    CursorTransform, nearest_sampling_enabled, transform_cursor_frame,
};

// ---------------------------------------------------------------------------
// Hotspot
// ---------------------------------------------------------------------------

/// Extract the cursor hotspot advertised by a client-side cursor surface.
pub(crate) fn cursor_surface_hotspot(surface: &WlSurface) -> (i32, i32) {
    with_states(surface, |states| {
        states
            .data_map
            .get::<CursorImageSurfaceData>()
            .and_then(|attrs| {
                attrs
                    .lock()
                    .ok()
                    .map(|attr| (attr.hotspot.x, attr.hotspot.y))
            })
            .unwrap_or((0, 0))
    })
}

pub(crate) fn handle_cursor_surface_commit(
    cursor_image: &CursorImageStatus,
    surface: &WlSurface,
) -> bool {
    let CursorImageStatus::Surface(cursor_surface) = cursor_image else {
        return false;
    };
    let root = surface_tree_root(surface);
    if cursor_surface != &root {
        return false;
    }

    if surface == &root {
        with_states(surface, |states| {
            let cursor_image_attributes = states.data_map.get::<CursorImageSurfaceData>();
            if let Some(mut cursor_image_attributes) =
                cursor_image_attributes.map(|attrs| attrs.lock().unwrap())
            {
                let buffer_delta = states
                    .cached_state
                    .get::<SurfaceAttributes>()
                    .current()
                    .buffer_delta
                    .take();
                if let Some(buffer_delta) = buffer_delta {
                    cursor_image_attributes.hotspot -= buffer_delta;
                }
            }
        });
    }

    true
}

fn surface_tree_root(surface: &WlSurface) -> WlSurface {
    let mut root = surface.clone();
    while let Some(parent) = get_parent(&root) {
        root = parent;
    }
    root
}

// ---------------------------------------------------------------------------
// Software sprite rasterisation
// ---------------------------------------------------------------------------

/// Blit a software cursor sprite to `frame` using run-length compressed rows.
///
/// Coordinates are in the same screen-space used for hit-testing so the
/// rendered cursor stays aligned with pointer events on every backend.
///
/// `elapsed_ms` is the time elapsed since the active cursor animation
/// started; it is used to select the frame of an animated cursor.
///
/// `transform` is the dynamic-cursor transform (rotation/zoom/stretch) to
/// apply around the sprite hotspot; the identity transform takes the fast
/// path with no per-pixel work. `nearest_cfg` is the
/// `cursor.dynamic.shake.nearest` setting used when magnifying.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_cursor_sprite<F: Frame>(
    frame: &mut F,
    damage: Rectangle<i32, Physical>,
    cursor_screen: (f32, f32),
    sprite: &SoftwareCursorSprite,
    elapsed_ms: u64,
    transform: &CursorTransform,
    nearest_cfg: u8,
) -> Result<(), F::Error> {
    let (sx, sy) = cursor_screen;
    let frame_data = sprite.frame_at(elapsed_ms);
    let pixels = &frame_data.pixels_bgra;

    if !transform.is_identity() {
        let nearest = nearest_sampling_enabled(nearest_cfg, transform.scale);
        let transformed = transform_cursor_frame(
            pixels,
            sprite.width,
            sprite.height,
            (sprite.hotspot_x, sprite.hotspot_y),
            transform,
            nearest,
        );
        let x0 = sx.round() as i32 - transformed.hotspot_x;
        let y0 = sy.round() as i32 - transformed.hotspot_y;
        blit_bgra_runlength(
            frame,
            damage,
            &transformed.pixels,
            transformed.width,
            transformed.height,
            x0,
            y0,
        )
    } else {
        let x0 = sx.round() as i32 - sprite.hotspot_x;
        let y0 = sy.round() as i32 - sprite.hotspot_y;
        blit_bgra_runlength(frame, damage, pixels, sprite.width, sprite.height, x0, y0)
    }
}

fn blit_bgra_runlength<F: Frame>(
    frame: &mut F,
    damage: Rectangle<i32, Physical>,
    pixels: &[u8],
    w: usize,
    h: usize,
    x0: i32,
    y0: i32,
) -> Result<(), F::Error> {
    for y in 0..h {
        let mut x = 0usize;
        while x < w {
            let base = (y * w + x) * 4;
            let a = pixels[base + 3];
            if a == 0 {
                x += 1;
                continue;
            }

            let b = pixels[base];
            let g = pixels[base + 1];
            let r = pixels[base + 2];

            // Merge identical neighbouring pixels into a single rect call.
            let mut run_end = x + 1;
            while run_end < w {
                let i = (y * w + run_end) * 4;
                if pixels[i] != b || pixels[i + 1] != g || pixels[i + 2] != r || pixels[i + 3] != a
                {
                    break;
                }
                run_end += 1;
            }

            draw_rect(
                frame,
                x0 + x as i32,
                y0 + y as i32,
                (run_end - x) as i32,
                1,
                Color32F::new(
                    r as f32 / 255.0,
                    g as f32 / 255.0,
                    b as f32 / 255.0,
                    a as f32 / 255.0,
                ),
                damage,
            )?;
            x = run_end;
        }
    }
    Ok(())
}
