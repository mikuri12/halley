use std::collections::HashMap;
use std::error::Error;

use resvg::{tiny_skia, usvg};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::Id;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::{ContextId, ImportMem, Renderer};
use smithay::utils::{Physical, Rectangle};

use super::window_decoration::{CornerRadii, RoundedTextureElement, WindowDecorationRenderer};
use super::window_texture::WindowTexture;
use crate::titlebar::Control;

const RASTER_SIZE: u32 = 64;
const CLOSE: &[u8] = include_bytes!("../../assets/titlebars/close.svg");
const MINIMIZE: &[u8] = include_bytes!("../../assets/titlebars/minimize.svg");
const MAXIMIZE: &[u8] = include_bytes!("../../assets/titlebars/maximize.svg");
const UNMAXIMIZE: &[u8] = include_bytes!("../../assets/titlebars/unmaximize.svg");

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Icon {
    Close,
    Minimize,
    Maximize,
    Unmaximize,
}

impl Icon {
    fn for_control(control: Control, maximized: bool) -> Self {
        match control {
            Control::Close => Self::Close,
            Control::Minimize => Self::Minimize,
            Control::Maximize if maximized => Self::Unmaximize,
            Control::Maximize => Self::Maximize,
        }
    }
}

#[derive(Default)]
pub struct TitlebarRenderer {
    context: Option<ContextId<GlesTexture>>,
    icons: HashMap<Icon, WindowTexture>,
    /// Configured custom glyph sources; when set, these replace the bundled
    /// bytes at prepare time.
    custom_sources: HashMap<Icon, Vec<u8>>,
    /// Stable signature of the loaded customization, so a config reload with
    /// the same paths does not rebuild textures.
    custom_signature: u64,
    failed: bool,
}

/// One titlebar button glyph: which icon, drawn where, in what tint.
#[derive(Clone)]
pub struct ControlRequest {
    pub id: Id,
    pub control: Control,
    /// Selects the unmaximize glyph in place of the maximize one.
    pub maximized: bool,
    pub destination: Rectangle<i32, Physical>,
    pub color: halley_config::BorderColor,
    pub alpha: f32,
}

impl TitlebarRenderer {
    pub fn control_element(
        &mut self,
        renderer: &mut GlesRenderer,
        decorations: &mut WindowDecorationRenderer,
        request: ControlRequest,
    ) -> Option<RoundedTextureElement> {
        let ControlRequest {
            id,
            control,
            maximized,
            destination,
            color,
            alpha,
        } = request;
        if destination.size.w <= 0 || destination.size.h <= 0 || alpha <= 0.0 {
            return None;
        }
        if let Err(err) = self.ensure(renderer) {
            if !self.failed {
                eventline::warn!("titlebars: failed to prepare bundled button masks: {err}");
                self.failed = true;
            }
            return None;
        }
        let icon = self.icons.get(&Icon::for_control(control, maximized))?;
        let texture = icon.texture.clone();
        let base = icon.render_element(id, destination, alpha);
        decorations.texture_element_with_radii(
            renderer,
            base,
            texture,
            destination,
            CornerRadii::default(),
            (color.r, color.g, color.b, 1.0),
        )
    }

    fn ensure(&mut self, renderer: &mut GlesRenderer) -> Result<(), Box<dyn Error>> {
        let context = renderer.context_id();
        if self.context.as_ref() == Some(&context)
            && self.icons.len() == 4
            && !self.custom_sources.is_empty()
        {
            return Ok(());
        }
        if self.context.as_ref() == Some(&context) && self.icons.len() == 4 && self.custom_sources.is_empty() {
            return Ok(());
        }
        self.context = Some(context.clone());
        self.icons.clear();
        self.failed = false;
        for (icon, bundled) in [
            (Icon::Close, CLOSE),
            (Icon::Minimize, MINIMIZE),
            (Icon::Maximize, MAXIMIZE),
            (Icon::Unmaximize, UNMAXIMIZE),
        ] {
            // A configured custom glyph wins over the bundled artwork;
            // unreadable files fall back to the bundled mask with a warning.
            let source: &[u8] = match self.custom_sources.get(&icon) {
                Some(bytes) => bytes,
                None => bundled,
            };
            let pixels = match raster_mask(source) {
                Some(pixels) => pixels,
                None if self.custom_sources.contains_key(&icon) => {
                    eventline::warn!(
                        "titlebars: custom glyph could not be rasterized; using the bundled mask"
                    );
                    raster_mask(bundled).ok_or("bundled SVG did not produce an alpha mask")?
                }
                None => return Err("bundled SVG did not produce an alpha mask".into()),
            };
            let texture = renderer.import_memory(
                &pixels,
                Fourcc::Abgr8888,
                (RASTER_SIZE as i32, RASTER_SIZE as i32).into(),
                false,
            )?;
            self.icons.insert(
                icon,
                WindowTexture {
                    texture,
                    context: context.clone(),
                },
            );
        }
        Ok(())
    }

    /// Applies custom glyph files from the titlebar configuration. Relative
    /// paths resolve against `base_dir` (the configuration file's
    /// directory). Returns `true` when the glyph set changed and the
    /// textures must be rebuilt; callers schedule a redraw.
    pub fn reload_custom_icons(
        &mut self,
        config: &halley_config::TitlebarIconPaths,
        base_dir: Option<&std::path::Path>,
    ) -> bool {
        let mut sources = HashMap::new();
        let mut signature = 0u64;
        for (icon, path) in [
            (Icon::Close, config.close.as_ref()),
            (Icon::Minimize, config.minimize.as_ref()),
            (Icon::Maximize, config.maximize.as_ref()),
            (Icon::Unmaximize, config.unmaximize.as_ref()),
        ] {
            let Some(path) = path else {
                continue;
            };
            let resolved = match base_dir {
                Some(dir) if path.is_relative() => dir.join(path),
                _ => path.clone(),
            };
            // Expand a leading `~` to $HOME like the screenshot directory.
            let resolved = if let Some(rest) = resolved
                .to_str()
                .and_then(|text| text.strip_prefix("~/"))
            {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                std::path::PathBuf::from(home).join(rest)
            } else {
                resolved
            };
            signature = signature.wrapping_mul(31).wrapping_add(path_signature(&resolved));
            match std::fs::read(&resolved) {
                Ok(bytes) => {
                    sources.insert(icon, bytes);
                }
                Err(err) => {
                    eventline::warn!(
                        "titlebars: could not read custom glyph {resolved:?}: {err}"
                    );
                }
            }
        }
        if signature == self.custom_signature {
            return false;
        }
        self.custom_signature = signature;
        self.custom_sources = sources;
        // Force the next ensure() to rebuild textures from the new sources.
        self.icons.clear();
        true
    }
}

/// Stable hash of one resolved glyph path, used to detect reloads that did
/// not actually change the configured files.
fn path_signature(path: &std::path::Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    hasher.finish()
}

fn raster_mask(source: &[u8]) -> Option<Vec<u8>> {
    let tree = usvg::Tree::from_data(source, &usvg::Options::default()).ok()?;
    let size = tree.size().to_int_size();
    if size.width() == 0 || size.height() == 0 {
        return None;
    }
    let mut pixmap = tiny_skia::Pixmap::new(RASTER_SIZE, RASTER_SIZE)?;
    let scale =
        (RASTER_SIZE as f32 / size.width() as f32).min(RASTER_SIZE as f32 / size.height() as f32);
    let transform = tiny_skia::Transform::from_scale(scale, scale).post_translate(
        (RASTER_SIZE as f32 - size.width() as f32 * scale) / 2.0,
        (RASTER_SIZE as f32 - size.height() as f32 * scale) / 2.0,
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let mut pixels = pixmap.data().to_vec();
    let mut visible = false;
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = pixel[3];
        visible |= alpha != 0;
        pixel[0] = alpha;
        pixel[1] = alpha;
        pixel[2] = alpha;
    }
    visible.then_some(pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_icons_are_nonempty_alpha_masks() {
        for source in [CLOSE, MINIMIZE, MAXIMIZE, UNMAXIMIZE] {
            let pixels = raster_mask(source).expect("valid bundled SVG");
            assert_eq!(
                pixels.len(),
                RASTER_SIZE as usize * RASTER_SIZE as usize * 4
            );
            assert!(pixels.chunks_exact(4).any(|pixel| pixel[3] != 0));
            assert!(
                pixels.chunks_exact(4).all(|pixel| pixel[0] == pixel[3]
                    && pixel[1] == pixel[3]
                    && pixel[2] == pixel[3])
            );
        }
    }

    #[test]
    fn maximized_control_uses_unmaximize_icon() {
        assert_eq!(Icon::for_control(Control::Maximize, false), Icon::Maximize);
        assert_eq!(Icon::for_control(Control::Maximize, true), Icon::Unmaximize);
        assert_eq!(Icon::for_control(Control::Close, true), Icon::Close);
    }
}
