use std::collections::HashMap;
use std::error::Error;
use std::hash::{DefaultHasher, Hash, Hasher};

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{
    GlesError, GlesFrame, GlesRenderer, GlesTexProgram, GlesTexture, Uniform, UniformName,
    UniformType,
};
use smithay::backend::renderer::utils::{CommitCounter, OpaqueRegions};
use smithay::backend::renderer::{ContextId, ImportMem, Renderer, Texture};
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Physical, Rectangle, Scale, Transform};

const SHADER: &str = include_str!("shaders/shadow.frag");

struct Resources {
    context: ContextId<GlesTexture>,
    texture: GlesTexture,
    program: GlesTexProgram,
}

#[derive(Default)]
pub struct ShadowRenderer {
    resources: Option<Resources>,
    failed_context: Option<ContextId<GlesTexture>>,
    ids: HashMap<String, Id>,
}

pub struct ShadowElement {
    id: Id,
    commit: CommitCounter,
    destination: Rectangle<i32, Physical>,
    caster_size: (f32, f32),
    caster_center: (f32, f32),
    hole_center: (f32, f32),
    corner_radii: crate::render::window_decoration::CornerRadii,
    spread: f32,
    blur_radius: f32,
    color: (f32, f32, f32, f32),
    alpha: f32,
    texture: GlesTexture,
    program: GlesTexProgram,
}

impl ShadowRenderer {
    pub fn element(
        &mut self,
        renderer: &mut GlesRenderer,
        identity: impl Into<String>,
        caster: Rectangle<i32, Physical>,
        corner_radius: f32,
        alpha: f32,
        config: halley_config::ShadowLayer,
    ) -> Result<Option<ShadowElement>, Box<dyn Error>> {
        self.element_with_radii(
            renderer,
            identity,
            caster,
            crate::render::window_decoration::CornerRadii::all(corner_radius),
            alpha,
            config,
        )
    }

    pub fn element_with_radii(
        &mut self,
        renderer: &mut GlesRenderer,
        identity: impl Into<String>,
        caster: Rectangle<i32, Physical>,
        corner_radii: crate::render::window_decoration::CornerRadii,
        alpha: f32,
        config: halley_config::ShadowLayer,
    ) -> Result<Option<ShadowElement>, Box<dyn Error>> {
        if !config.enabled
            || config.color.a <= 0.0
            || config.blur_radius <= 0.0
            || alpha <= 0.0
            || caster.size.w <= 0
            || caster.size.h <= 0
        {
            return Ok(None);
        }
        if self.ensure_resources(renderer).is_err() {
            return Ok(None);
        }
        let resources = self.resources.as_ref().expect("ensured above");
        let (destination, pad) = shadow_geometry(caster, config);
        let (caster_center, hole_center) = shadow_centers(caster, pad, config);
        let identity = identity.into();
        let id = self.ids.entry(identity).or_insert_with(Id::new).clone();
        Ok(Some(ShadowElement {
            id,
            commit: shadow_commit(caster, corner_radii, alpha, config),
            destination,
            caster_size: (caster.size.w as f32, caster.size.h as f32),
            caster_center,
            hole_center,
            corner_radii: crate::render::window_decoration::CornerRadii {
                top: corner_radii.top.max(0.0),
                bottom: corner_radii.bottom.max(0.0),
            },
            spread: config.spread.max(0.0),
            blur_radius: config.blur_radius.max(0.0),
            color: (
                config.color.r.clamp(0.0, 1.0),
                config.color.g.clamp(0.0, 1.0),
                config.color.b.clamp(0.0, 1.0),
                config.color.a.clamp(0.0, 1.0),
            ),
            alpha: alpha.clamp(0.0, 1.0),
            texture: resources.texture.clone(),
            program: resources.program.clone(),
        }))
    }

    fn ensure_resources(&mut self, renderer: &mut GlesRenderer) -> Result<(), Box<dyn Error>> {
        let context = renderer.context_id();
        if self
            .resources
            .as_ref()
            .is_some_and(|resources| resources.context == context)
        {
            return Ok(());
        }
        if self.failed_context.as_ref() == Some(&context) {
            return Err("shadow shader previously failed for this GL context".into());
        }
        let result = (|| {
            let texture = renderer.import_memory(
                &[255_u8; 4 * 4 * 4],
                Fourcc::Abgr8888,
                (4, 4).into(),
                false,
            )?;
            let program = renderer.compile_custom_texture_shader(
                SHADER,
                &[
                    UniformName::new("rect_size", UniformType::_2f),
                    UniformName::new("caster_size", UniformType::_2f),
                    UniformName::new("caster_center", UniformType::_2f),
                    UniformName::new("hole_center", UniformType::_2f),
                    UniformName::new("corner_radii", UniformType::_2f),
                    UniformName::new("spread", UniformType::_1f),
                    UniformName::new("shadow_radius", UniformType::_1f),
                    UniformName::new("shadow_color", UniformType::_4f),
                ],
            )?;
            Ok::<_, Box<dyn Error>>(Resources {
                context: context.clone(),
                texture,
                program,
            })
        })();
        match result {
            Ok(resources) => {
                self.resources = Some(resources);
                self.failed_context = None;
                Ok(())
            }
            Err(error) => {
                self.resources = None;
                self.failed_context = Some(context);
                eventline::warn!(
                    "shadow rendering unavailable; disabling shadows for this GL context: {error}"
                );
                Err(error)
            }
        }
    }
}

impl Element for ShadowElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        Rectangle::new(
            (0.0, 0.0).into(),
            (
                f64::from(self.texture.size().w),
                f64::from(self.texture.size().h),
            )
                .into(),
        )
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.destination
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }
}

impl RenderElement<GlesRenderer> for ShadowElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        frame.render_texture_from_to(
            &self.texture,
            src,
            dst,
            damage,
            &[],
            Transform::Normal,
            self.alpha,
            Some(&self.program),
            &[
                Uniform::new(
                    "rect_size",
                    (
                        self.destination.size.w as f32,
                        self.destination.size.h as f32,
                    ),
                ),
                Uniform::new("caster_size", self.caster_size),
                Uniform::new("caster_center", self.caster_center),
                Uniform::new("hole_center", self.hole_center),
                Uniform::new(
                    "corner_radii",
                    (self.corner_radii.top, self.corner_radii.bottom),
                ),
                Uniform::new("spread", self.spread),
                Uniform::new("shadow_radius", self.blur_radius),
                Uniform::new("shadow_color", self.color),
            ],
        )
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

fn shadow_offset(config: halley_config::ShadowLayer) -> (i32, i32) {
    (
        config.offset_x.round() as i32,
        config.offset_y.round() as i32,
    )
}

fn shadow_geometry(
    caster: Rectangle<i32, Physical>,
    config: halley_config::ShadowLayer,
) -> (Rectangle<i32, Physical>, i32) {
    let falloff = (config.blur_radius.max(0.0) * 3.0).ceil() as i32;
    let pad = falloff + config.spread.max(0.0).ceil() as i32 + 2;
    let (offset_x, offset_y) = shadow_offset(config);
    (
        Rectangle::new(
            (caster.loc.x + offset_x - pad, caster.loc.y + offset_y - pad).into(),
            (
                (caster.size.w + pad * 2).max(1),
                (caster.size.h + pad * 2).max(1),
            )
                .into(),
        ),
        pad,
    )
}

fn shadow_centers(
    caster: Rectangle<i32, Physical>,
    pad: i32,
    config: halley_config::ShadowLayer,
) -> ((f32, f32), (f32, f32)) {
    let (offset_x, offset_y) = shadow_offset(config);
    let caster_center = (
        pad as f32 + caster.size.w as f32 * 0.5,
        pad as f32 + caster.size.h as f32 * 0.5,
    );
    let hole_center = (
        caster_center.0 - offset_x as f32,
        caster_center.1 - offset_y as f32,
    );
    (caster_center, hole_center)
}

fn shadow_commit(
    caster: Rectangle<i32, Physical>,
    corner_radii: crate::render::window_decoration::CornerRadii,
    alpha: f32,
    config: halley_config::ShadowLayer,
) -> CommitCounter {
    let mut hash = DefaultHasher::new();
    caster.loc.x.hash(&mut hash);
    caster.loc.y.hash(&mut hash);
    caster.size.w.hash(&mut hash);
    caster.size.h.hash(&mut hash);
    corner_radii.top.to_bits().hash(&mut hash);
    corner_radii.bottom.to_bits().hash(&mut hash);
    alpha.to_bits().hash(&mut hash);
    config.enabled.hash(&mut hash);
    config.blur_radius.to_bits().hash(&mut hash);
    config.spread.to_bits().hash(&mut hash);
    config.offset_x.to_bits().hash(&mut hash);
    config.offset_y.to_bits().hash(&mut hash);
    config.color.r.to_bits().hash(&mut hash);
    config.color.g.to_bits().hash(&mut hash);
    config.color.b.to_bits().hash(&mut hash);
    config.color.a.to_bits().hash(&mut hash);
    CommitCounter::from(hash.finish() as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_includes_gaussian_tail_spread_and_offset() {
        let config = halley_config::ShadowLayer {
            blur_radius: 8.0,
            spread: 2.0,
            offset_x: 3.0,
            offset_y: 5.0,
            ..halley_config::Shadows::default().window
        };
        let caster = Rectangle::new((100, 200).into(), (300, 150).into());
        let (geometry, pad) = shadow_geometry(caster, config);
        assert_eq!(pad, 28);
        assert_eq!(geometry.loc, (75, 177).into());
        assert_eq!(geometry.size, (356, 206).into());
    }

    #[test]
    fn hole_matches_caster_when_the_shadow_is_not_offset() {
        let config = halley_config::ShadowLayer {
            offset_x: 0.0,
            offset_y: 0.0,
            ..halley_config::Shadows::default().window
        };
        let caster = Rectangle::new((40, 80).into(), (200, 100).into());
        let (_, pad) = shadow_geometry(caster, config);
        let (caster_center, hole_center) = shadow_centers(caster, pad, config);
        assert_eq!(hole_center, caster_center);
        assert_eq!(hole_center, (pad as f32 + 100.0, pad as f32 + 50.0));
    }

    #[test]
    fn hole_stays_on_the_window_when_the_shadow_is_offset() {
        let config = halley_config::ShadowLayer {
            offset_x: 4.0,
            offset_y: 9.0,
            ..halley_config::Shadows::default().window
        };
        let caster = Rectangle::new((40, 80).into(), (200, 100).into());
        let (_, pad) = shadow_geometry(caster, config);
        let (caster_center, hole_center) = shadow_centers(caster, pad, config);
        assert_eq!(caster_center, (pad as f32 + 100.0, pad as f32 + 50.0));
        assert_eq!(hole_center, (caster_center.0 - 4.0, caster_center.1 - 9.0));
    }
}
