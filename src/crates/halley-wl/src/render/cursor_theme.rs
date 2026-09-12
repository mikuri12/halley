use std::collections::HashMap;
use std::fs;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use halley_config::CursorConfig;
use once_cell::sync::Lazy;
use smithay::input::pointer::{CursorIcon, CursorImageStatus};
use smithay::utils::IsAlive;
use xcursor::{CursorTheme, parser::Image};

use super::dynamic_cursor;

// ---------------------------------------------------------------------------
// Sprite data (multi-frame support for animated XCursor cursors)
// ---------------------------------------------------------------------------

/// A single frame of an animated cursor.
#[derive(Clone)]
pub(crate) struct FrameData {
    pub(crate) pixels_bgra: Vec<u8>,
    pub(crate) delay_ms: u32,
}

#[derive(Clone)]
pub(crate) struct SoftwareCursorSprite {
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) hotspot_x: i32,
    pub(crate) hotspot_y: i32,
    pub(crate) frames: Vec<FrameData>,
}

impl SoftwareCursorSprite {
    /// Returns the frame corresponding to the elapsed time (ms).
    /// For single-frame sprites (static cursors) it skips the arithmetic.
    pub(crate) fn frame_at(&self, elapsed_ms: u64) -> &FrameData {
        if self.frames.len() == 1 {
            return &self.frames[0];
        }
        let cycle_ms: u64 = self.frames.iter().map(|f| f.delay_ms as u64).sum();
        let pos = elapsed_ms % cycle_ms.max(1);
        let mut acc = 0u64;
        for frame in &self.frames {
            acc += frame.delay_ms as u64;
            if pos < acc {
                return frame;
            }
        }
        // Fallback (no debería alcanzarse)
        &self.frames[0]
    }

    /// Acceso rápido al primer frame (para paths que no necesitan animación).
    pub(crate) fn first_frame(&self) -> &FrameData {
        &self.frames[0]
    }
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

type CursorSpriteCache = HashMap<(String, u32, String), Option<Arc<SoftwareCursorSprite>>>;

#[derive(Default)]
struct CursorSpriteManager {
    cache: CursorSpriteCache,
}

impl CursorSpriteManager {
    fn sprite_with_fallback(
        &mut self,
        cursor: &CursorConfig,
        icon: CursorIcon,
    ) -> Option<Arc<SoftwareCursorSprite>> {
        self.sprite(cursor, icon).or_else(|| {
            if icon == CursorIcon::Default {
                None
            } else {
                self.sprite(cursor, CursorIcon::Default)
            }
        })
    }

    fn sprite(
        &mut self,
        cursor: &CursorConfig,
        icon: CursorIcon,
    ) -> Option<Arc<SoftwareCursorSprite>> {
        let theme = cursor.theme.trim();
        let theme = if theme.is_empty() { "Adwaita" } else { theme };
        let size = cursor.size.clamp(8, 128);
        let icon_key = icon.name().to_string();
        let cache_key = (theme.to_string(), size, icon_key);

        self.cache
            .entry(cache_key)
            .or_insert_with(|| {
                load_cursor_from_theme(theme, size, icon).or_else(|| {
                    if theme == "Adwaita" {
                        None
                    } else {
                        load_cursor_from_theme("Adwaita", size, icon)
                    }
                })
            })
            .clone()
    }
}

static CURSOR_SPRITES: Lazy<Mutex<CursorSpriteManager>> =
    Lazy::new(|| Mutex::new(CursorSpriteManager::default()));

// ---------------------------------------------------------------------------
// Estado de animación del cursor
// ---------------------------------------------------------------------------

pub(crate) struct CursorAnimState {
    started_at: Option<Instant>,
    /// Duración total del ciclo en ms (0 == cursor estático).
    cycle_ms: u64,
}

impl Default for CursorAnimState {
    fn default() -> Self {
        Self {
            started_at: None,
            cycle_ms: 0,
        }
    }
}

pub(crate) struct CursorManager {
    current_cursor: CursorImageStatus,
    sprites: CursorSpriteManager,
    anim: CursorAnimState,
    /// Último icono named resuelto, para detectar cambios y resetear la animación.
    last_named_icon: Option<CursorIcon>,
    /// Estado de dynamic-cursors (rotación/tilt/stretch + shake to find).
    pub(crate) dynamic: dynamic_cursor::DynamicCursorState,
}

impl Default for CursorManager {
    fn default() -> Self {
        Self {
            current_cursor: CursorImageStatus::default_named(),
            sprites: CursorSpriteManager::default(),
            anim: CursorAnimState::default(),
            last_named_icon: None,
            dynamic: dynamic_cursor::DynamicCursorState::default(),
        }
    }
}

impl CursorManager {
    pub(crate) fn cursor_image(&self) -> &CursorImageStatus {
        &self.current_cursor
    }

    pub(crate) fn set_cursor_image(&mut self, cursor: CursorImageStatus) {
        // If we no longer hold a named cursor, clear the tracking so the next
        // time a Named is resolved the animation is reset.
        if !matches!(cursor, CursorImageStatus::Named(_)) {
            self.last_named_icon = None;
        }
        self.current_cursor = cursor;
    }

    pub(crate) fn check_cursor_image_surface_alive(&mut self) {
        if let CursorImageStatus::Surface(surface) = &self.current_cursor
            && !surface.alive()
        {
            self.current_cursor = CursorImageStatus::default_named();
            // On fallback to default_named the next render will resolve Default
            // and reset the animation in sprite_with_fallback.
        }
    }

    /// Tiempo transcurrido desde que empezó la animación actual (ms).
    pub(crate) fn cursor_elapsed_ms(&self) -> u64 {
        self.anim.started_at.map(|t| t.elapsed().as_millis() as u64).unwrap_or(0)
    }

    /// ¿El cursor Named actual es animado (multi-frame)? Sólo lectura: NO resetea
    /// la animación. Se basa en `cycle_ms` que `sprite_with_fallback` puebla la
    /// primera vez que se resuelve un icono Named con frames animados (valor > 0).
    /// Esto evita el deadlock de mirar el cache (que aún podría estar vacío en
    /// el primer tick), porque `cycle_ms` queda fijado POR el primer render.
    pub(crate) fn named_cursor_is_animated(&self) -> bool {
        matches!(self.current_cursor, CursorImageStatus::Named(_)) && self.anim.cycle_ms > 0
    }

    pub(crate) fn sprite_with_fallback(
        &mut self,
        cursor: &CursorConfig,
        icon: CursorIcon,
    ) -> Option<Arc<SoftwareCursorSprite>> {
        let sprite = self.sprites.sprite_with_fallback(cursor, icon)?;
        // Resetear la animación solo cuando el icono named cambia de verdad,
        // no en cada frame de render.
        if self.last_named_icon != Some(icon) {
            self.last_named_icon = Some(icon);
            self.anim.started_at = Some(Instant::now());
            self.anim.cycle_ms = if sprite.frames.len() <= 1 {
                0
            } else {
                sprite.frames.iter().map(|f| f.delay_ms as u64).sum()
            };
        }
        Some(sprite)
    }

    // -- dynamic cursors ----------------------------------------------------

    /// Motion-event update for the dynamic cursor simulation.
    pub(crate) fn dynamic_on_move(
        &mut self,
        pos: (f64, f64),
        delta: (f64, f64),
        cfg: &halley_config::DynamicCursorConfig,
    ) {
        self.dynamic.on_move(pos, delta, cfg, Instant::now());
    }

    /// Per-frame tick for the dynamic cursor simulation.
    pub(crate) fn dynamic_on_tick(
        &mut self,
        pos: (f64, f64),
        cfg: &halley_config::DynamicCursorConfig,
    ) {
        self.dynamic.on_tick(pos, Instant::now(), cfg);
    }

    /// Resets the dynamic cursor simulation (config/mode changes).
    pub(crate) fn dynamic_reset(&mut self) {
        self.dynamic.reset();
    }
}

// ---------------------------------------------------------------------------
// Theme loading
// ---------------------------------------------------------------------------

/// Picks the target size with the same criteria as the previous version
/// (smallest delta of nominal size vs real width; delay only breaks ties) and
/// returns ALL frames of that size, in file order.
fn pick_best_size_frames(images: &[Image], requested_size: u32) -> Vec<Image> {
    let target = images
        .iter()
        .min_by_key(|img| {
            let nominal_delta = img.size.abs_diff(requested_size);
            let width_delta = img.width.abs_diff(requested_size);
            (nominal_delta, width_delta, img.delay)
        })
        .map(|img| img.size)
        .unwrap_or(requested_size);
    images
        .iter()
        .filter(|img| img.size == target && img.width == target)
        .cloned()
        .collect()
}

fn load_cursor_from_theme(
    theme_name: &str,
    requested_size: u32,
    icon: CursorIcon,
) -> Option<Arc<SoftwareCursorSprite>> {
    let theme = CursorTheme::load(theme_name);
    for icon_name in std::iter::once(icon.name()).chain(icon.alt_names().iter().copied()) {
        let Some(icon_path) = theme.load_icon(icon_name) else {
            continue;
        };
        let Some(bytes) = fs::read(icon_path).ok() else {
            continue;
        };
        let Some(images) = xcursor::parser::parse_xcursor(&bytes) else {
            continue;
        };
        let frames = pick_best_size_frames(&images, requested_size);
        if frames.is_empty() {
            continue;
        }
        // All frames of the chosen size share dimensions and hotspot.
        // Pull everything from the first frame BEFORE moving `frames`
        // (the `.into_iter()` consumes the Vec and would break the `first`
        // borrow).
        let first = &frames[0];
        let width = usize::try_from(first.width).ok()?;
        let height = usize::try_from(first.height).ok()?;
        let max_hotspot_x = i32::try_from(width.saturating_sub(1)).ok().unwrap_or(0);
        let max_hotspot_y = i32::try_from(height.saturating_sub(1)).ok().unwrap_or(0);
        let hotspot_x = (first.xhot as i32).clamp(0, max_hotspot_x);
        let hotspot_y = (first.yhot as i32).clamp(0, max_hotspot_y);
        let frame_data: Vec<FrameData> = frames
            .into_iter()
            .map(|img| FrameData {
                // Same bytes as before: the field is named bgra but upstream
                // copies pixels_rgba verbatim (no R<->B swap). Keep the existing
                // semantics to avoid a visual regression.
                pixels_bgra: img.pixels_rgba,
                delay_ms: img.delay,
            })
            .collect();
        return Some(Arc::new(SoftwareCursorSprite {
            width,
            height,
            hotspot_x,
            hotspot_y,
            frames: frame_data,
        }));
    }
    None
}

// ---------------------------------------------------------------------------
// Public sprite resolution with fallback chain
// ---------------------------------------------------------------------------

pub(crate) fn themed_cursor_sprite_with_fallback(
    cursor: &CursorConfig,
    icon: CursorIcon,
) -> Option<Arc<SoftwareCursorSprite>> {
    CURSOR_SPRITES
        .lock()
        .ok()?
        .sprite_with_fallback(cursor, icon)
}
