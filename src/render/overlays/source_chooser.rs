use std::error::Error;

use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::{Kind, render_elements};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Logical, Physical, Rectangle};

use super::capture_assets::{CaptureIcon, DISPLAY_SIZE};

render_elements! {
    pub SourceChooserElement<=GlesRenderer>;
    Icon=MemoryRenderBufferRenderElement<GlesRenderer>,
    Card=crate::render::node::LabelRenderElement,
}

#[allow(clippy::too_many_arguments)]
pub fn menu_elements(
    renderer: &mut GlesRenderer,
    node_renderer: &mut crate::render::node::NodeRenderer,
    output: Rectangle<i32, Logical>,
    selected: usize,
    hovered: Option<usize>,
    monitor_available: bool,
    window_available: bool,
    visuals: crate::render::overlays::shell::OverlayVisuals,
) -> Result<Vec<SourceChooserElement>, Box<dyn Error>> {
    let layout = crate::capture::source_chooser::layout(output);
    let localize = |rectangle: Rectangle<i32, Logical>| {
        Rectangle::<i32, Physical>::new(
            (rectangle.loc - output.loc).to_physical(1),
            rectangle.size.to_physical(1),
        )
    };
    let mut elements = Vec::new();
    let bar = localize(layout.bar);
    elements.push(SourceChooserElement::Card(
        crate::render::overlays::shell::card_element(
            renderer,
            node_renderer,
            bar,
            visuals,
            visuals.fill,
            0.96,
        )?,
    ));

    for (index, item) in layout.items.into_iter().enumerate() {
        let enabled = [monitor_available, window_available][index];
        let active = enabled && (selected == index || hovered == Some(index));
        let item = localize(item);
        let fill = if !enabled {
            visuals.key_fill.mix(visuals.fill, 0.55)
        } else if active {
            visuals.fill.mix(visuals.border, 0.12)
        } else {
            visuals.key_fill
        };
        let accent = if !enabled {
            visuals.subtext.mix(visuals.fill, 0.45)
        } else if active {
            visuals.border
        } else {
            visuals.subtext
        };
        let mut item_visuals = visuals;
        item_visuals.border = accent;
        item_visuals.border_px = if visuals.border_px > 0.0 { 2.0 } else { 0.0 };
        elements.push(SourceChooserElement::Card(
            crate::render::overlays::shell::card_element(
                renderer,
                node_renderer,
                item,
                item_visuals,
                fill,
                if !enabled {
                    0.50
                } else if active {
                    0.98
                } else {
                    0.94
                },
            )?,
        ));

        let icon_size = DISPLAY_SIZE
            .min(item.size.w - 12)
            .min(item.size.h - 12)
            .max(12);
        let location = (
            f64::from(item.loc.x + (item.size.w - icon_size) / 2),
            f64::from(item.loc.y + (item.size.h - icon_size) / 2),
        );
        let icon_rgb = if active {
            visuals.text.bytes()
        } else {
            visuals.subtext.bytes()
        };
        let icon = super::capture_assets::buffer(
            icon_rgb,
            [CaptureIcon::Monitor, CaptureIcon::Window][index],
        )?;
        let icon = MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            location,
            &icon,
            Some(if !enabled {
                0.30
            } else if active {
                1.0
            } else {
                0.72
            }),
            None,
            Some((icon_size, icon_size).into()),
            Kind::Unspecified,
        )?;
        elements.push(SourceChooserElement::Icon(icon));
    }
    Ok(elements)
}
