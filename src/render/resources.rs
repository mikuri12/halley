use halley_config::{Animations, Font};

use super::arrange_texture::ArrangeTextureTransitions;
use super::background::BackgroundRenderer;
use super::close::WindowCloseAnimations;
use super::effects::backdrop_blur::BackdropBlurRenderer;
use super::effects::shadow::ShadowRenderer;
use super::fullscreen_texture::FullscreenTextureTransitions;
use super::node::NodeRenderer;
use super::overlays::cluster_creation::ClusterCreationOverlay;
use super::overlays::fps::DebugFpsOverlay;
use super::overlays::preview::OverlayPreviewCache;
use super::pin::PinRenderer;
use super::text::UiTextRenderer;
use super::titlebar::TitlebarRenderer;
use super::window_decoration::WindowDecorationRenderer;
use super::window_shader::WindowAnimationShaders;
use crate::clusters::render::ClusterRenderer;

/// Renderer-owned caches and GPU resources shared by every output.
///
/// Keeping these behind one session field makes their lifetime and reload
/// boundary explicit. Compositor policy remains in `Session`; this aggregate
/// contains only presentation resources that must survive across frames.
pub struct RenderState {
    pub(crate) window_close_animations: WindowCloseAnimations,
    pub(crate) arrange_textures: ArrangeTextureTransitions,
    pub(crate) background_renderer: BackgroundRenderer,
    pub(crate) fullscreen_textures: FullscreenTextureTransitions,
    pub(crate) overlay_previews: OverlayPreviewCache,
    pub(crate) cluster_creation_overlay: ClusterCreationOverlay,
    pub(crate) debug_fps_overlay: DebugFpsOverlay,
    pub(crate) node_renderer: NodeRenderer,
    pub(crate) pin_renderer: PinRenderer,
    pub(crate) cluster_renderer: ClusterRenderer,
    pub(crate) window_decoration_renderer: WindowDecorationRenderer,
    pub(crate) backdrop_blur_renderer: BackdropBlurRenderer,
    pub(crate) shadow_renderer: ShadowRenderer,
    pub(crate) ui_text: UiTextRenderer,
    pub(crate) titlebar_renderer: TitlebarRenderer,
    pub(crate) window_shaders: WindowAnimationShaders,
}

/// Per-frame mutable view over [`RenderState`].
///
/// Splitting the aggregate once at the request boundary lets the scene builder
/// borrow independent caches without exposing many session-level fields.
pub struct RenderResources<'a> {
    pub window_close_animations: &'a mut WindowCloseAnimations,
    pub arrange_textures: &'a mut ArrangeTextureTransitions,
    pub background_renderer: &'a mut BackgroundRenderer,
    pub fullscreen_textures: &'a mut FullscreenTextureTransitions,
    pub overlay_previews: &'a mut OverlayPreviewCache,
    pub cluster_creation_overlay: &'a mut ClusterCreationOverlay,
    pub debug_fps_overlay: &'a mut DebugFpsOverlay,
    pub node_renderer: &'a mut NodeRenderer,
    pub pin_renderer: &'a mut PinRenderer,
    pub cluster_renderer: &'a mut ClusterRenderer,
    pub window_decoration_renderer: &'a mut WindowDecorationRenderer,
    pub backdrop_blur_renderer: &'a mut BackdropBlurRenderer,
    pub shadow_renderer: &'a mut ShadowRenderer,
    pub ui_text: &'a mut UiTextRenderer,
    pub titlebar_renderer: &'a mut TitlebarRenderer,
    pub window_shaders: &'a mut WindowAnimationShaders,
}

impl<'a> From<&'a mut RenderState> for RenderResources<'a> {
    fn from(state: &'a mut RenderState) -> Self {
        Self {
            window_close_animations: &mut state.window_close_animations,
            arrange_textures: &mut state.arrange_textures,
            background_renderer: &mut state.background_renderer,
            fullscreen_textures: &mut state.fullscreen_textures,
            overlay_previews: &mut state.overlay_previews,
            cluster_creation_overlay: &mut state.cluster_creation_overlay,
            debug_fps_overlay: &mut state.debug_fps_overlay,
            node_renderer: &mut state.node_renderer,
            pin_renderer: &mut state.pin_renderer,
            cluster_renderer: &mut state.cluster_renderer,
            window_decoration_renderer: &mut state.window_decoration_renderer,
            backdrop_blur_renderer: &mut state.backdrop_blur_renderer,
            shadow_renderer: &mut state.shadow_renderer,
            ui_text: &mut state.ui_text,
            titlebar_renderer: &mut state.titlebar_renderer,
            window_shaders: &mut state.window_shaders,
        }
    }
}

impl RenderState {
    pub fn new(animations: Animations, font: &Font) -> Self {
        Self {
            window_close_animations: WindowCloseAnimations::new(animations.clone()),
            arrange_textures: ArrangeTextureTransitions::default(),
            background_renderer: BackgroundRenderer::default(),
            fullscreen_textures: FullscreenTextureTransitions::default(),
            overlay_previews: OverlayPreviewCache::default(),
            cluster_creation_overlay: ClusterCreationOverlay::default(),
            debug_fps_overlay: DebugFpsOverlay::default(),
            node_renderer: NodeRenderer::default(),
            pin_renderer: PinRenderer::default(),
            cluster_renderer: ClusterRenderer::default(),
            window_decoration_renderer: WindowDecorationRenderer::default(),
            backdrop_blur_renderer: BackdropBlurRenderer::default(),
            shadow_renderer: ShadowRenderer::default(),
            ui_text: UiTextRenderer::new(font),
            titlebar_renderer: TitlebarRenderer::default(),
            window_shaders: {
                let mut shaders = WindowAnimationShaders::default();
                shaders.reload(&animations);
                shaders
            },
        }
    }
}
