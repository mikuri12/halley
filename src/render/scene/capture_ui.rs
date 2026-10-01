use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn capture_overlay_elements(
    renderer: &mut GlesRenderer,
    output: &Output,
    output_geometry: Rectangle<i32, Logical>,
    overlay: crate::capture::CaptureOverlay<'_>,
    node_renderer: &mut crate::render::node::NodeRenderer,
    overlay_config: &halley_config::Overlays,
    _decorations: &halley_config::Decorations,
) -> Result<Vec<SceneElement>, Box<dyn Error>> {
    let visuals = crate::render::overlays::shell::resolve_visuals(overlay_config);
    match overlay {
        crate::capture::CaptureOverlay::None => Ok(Vec::new()),
        crate::capture::CaptureOverlay::Region(region) => {
            let mut elements = capture_picker_elements(
                renderer,
                node_renderer,
                &output.name(),
                output_geometry,
                region,
                true,
                visuals,
            )?;
            // Built back-to-front; the scene consumes front-to-back.
            elements.reverse();
            Ok(elements)
        }
        crate::capture::CaptureOverlay::Highlight(region) => {
            let mut elements = capture_picker_elements(
                renderer,
                node_renderer,
                &output.name(),
                output_geometry,
                region,
                false,
                visuals,
            )?;
            elements.reverse();
            Ok(elements)
        }
        crate::capture::CaptureOverlay::Menu {
            output_name,
            selected,
            hovered,
            window_available,
        } if output.name() == output_name => Ok(crate::render::overlays::capture::menu_elements(
            renderer,
            node_renderer,
            output_geometry,
            selected,
            hovered,
            window_available,
            visuals,
        )?
        .into_iter()
        .rev()
        .map(SceneElement::CaptureOverlay)
        .collect()),
        crate::capture::CaptureOverlay::Menu { .. } => Ok(Vec::new()),
        crate::capture::CaptureOverlay::SourceMenu {
            output_name,
            selected,
            hovered,
            monitor_available,
            window_available,
        } if output.name() == output_name => {
            let mut elements = Vec::new();
            elements.extend(
                crate::render::overlays::source_chooser::menu_elements(
                    renderer,
                    node_renderer,
                    output_geometry,
                    selected,
                    hovered,
                    monitor_available,
                    window_available,
                    visuals,
                )?
                .into_iter()
                .rev()
                .map(SceneElement::SourceChooser),
            );
            elements.push(source_chooser_backdrop(output_geometry, node_renderer));
            Ok(elements)
        }
        crate::capture::CaptureOverlay::SourceMenu { .. } => Ok(vec![source_chooser_backdrop(
            output_geometry,
            node_renderer,
        )]),
    }
}

fn source_chooser_backdrop(
    output: Rectangle<i32, Logical>,
    node_renderer: &mut crate::render::node::NodeRenderer,
) -> SceneElement {
    SceneElement::Border(crate::render::solid_color_element(
        node_renderer.active_slot_id(crate::render::node::NodeSlot::SourceChooserBackdrop),
        Rectangle::from_size(output.size.to_physical(1)),
        crate::render::overlays::shell::backdrop_dim(0.45),
    ))
}

pub(super) const PICKER_HANDLE_SIZE: i32 = 12;
pub(super) const PICKER_BORDER_PX: f32 = 2.0;
const PICKER_DASH_PERIOD: f32 = 16.0;
const PICKER_DASH_LENGTH: f32 = 10.0;

/// Where every part of the area selector goes on one output.
///
/// The dimmed surround is necessarily per-output, but the outline and the
/// corner grips are derived from the **unclipped** selection so a region
/// spanning two monitors reads as one region with one set of four handles,
/// rather than as a separate box per output.
pub(super) struct PickerLayout {
    pub dim: Vec<Rectangle<i32, Physical>>,
    pub outline: Rectangle<i32, Physical>,
    pub handles: [Rectangle<i32, Physical>; 4],
}

pub(super) fn picker_layout(
    output: Rectangle<i32, Logical>,
    selection: Rectangle<i32, Logical>,
) -> Option<PickerLayout> {
    let output_local = Rectangle::<i32, Physical>::from_size(output.size.to_physical(1));
    let clipped = output.intersection(selection).map(|intersection| {
        Rectangle::<i32, Physical>::new(
            (intersection.loc - output.loc).to_physical(1),
            intersection.size.to_physical(1),
        )
    })?;

    let right = clipped.loc.x + clipped.size.w;
    let bottom = clipped.loc.y + clipped.size.h;
    let dim = [
        Rectangle::new(
            (0, 0).into(),
            (output_local.size.w, clipped.loc.y.max(0)).into(),
        ),
        Rectangle::new(
            (0, bottom).into(),
            (output_local.size.w, (output_local.size.h - bottom).max(0)).into(),
        ),
        Rectangle::new(
            (0, clipped.loc.y).into(),
            (clipped.loc.x.max(0), clipped.size.h).into(),
        ),
        Rectangle::new(
            (right, clipped.loc.y).into(),
            ((output_local.size.w - right).max(0), clipped.size.h).into(),
        ),
    ]
    .into_iter()
    .filter(|rect| rect.size.w > 0 && rect.size.h > 0)
    .collect();

    // Unclipped: may extend past this output on either side, in which case the
    // damage tracker simply skips the parts with no visible area.
    let outline = Rectangle::<i32, Physical>::new(
        (selection.loc - output.loc).to_physical(1),
        selection.size.to_physical(1),
    );
    let outline_right = outline.loc.x + outline.size.w;
    let outline_bottom = outline.loc.y + outline.size.h;
    let handles = [
        outline.loc,
        (outline_right, outline.loc.y).into(),
        (outline.loc.x, outline_bottom).into(),
        (outline_right, outline_bottom).into(),
    ]
    .map(|point| {
        Rectangle::new(
            (
                point.x - PICKER_HANDLE_SIZE / 2,
                point.y - PICKER_HANDLE_SIZE / 2,
            )
                .into(),
            (PICKER_HANDLE_SIZE, PICKER_HANDLE_SIZE).into(),
        )
    });

    Some(PickerLayout {
        dim,
        outline,
        handles,
    })
}

fn capture_picker_elements(
    renderer: &mut GlesRenderer,
    node_renderer: &mut crate::render::node::NodeRenderer,
    output_name: &str,
    output: Rectangle<i32, Logical>,
    selection: Rectangle<i32, Logical>,
    region_style: bool,
    visuals: crate::render::overlays::shell::OverlayVisuals,
) -> Result<Vec<SceneElement>, Box<dyn Error>> {
    use crate::render::node::NodeSlot;

    let dim_color = crate::render::overlays::shell::backdrop_dim(0.62);
    let accent = (
        visuals.border.r,
        visuals.border.g,
        visuals.border.b,
        visuals.border.a,
    );

    let Some(layout) = picker_layout(output, selection) else {
        // The selection misses this output entirely: dim all of it.
        let id = node_renderer.slot_id(output_name, NodeSlot::PickerBackdrop);
        return Ok(vec![SceneElement::Border(SolidColorRenderElement::new(
            id,
            Rectangle::<i32, Physical>::from_size(output.size.to_physical(1)),
            CommitCounter::default(),
            dim_color,
            Kind::Unspecified,
        ))]);
    };

    let mut elements = Vec::with_capacity(9);
    for (index, rect) in layout.dim.iter().enumerate() {
        let id = node_renderer.slot_id(output_name, NodeSlot::PickerDim(index as u8));
        elements.push(SceneElement::Border(SolidColorRenderElement::new(
            id,
            *rect,
            CommitCounter::default(),
            dim_color,
            Kind::Unspecified,
        )));
    }

    // A dashed outline for an adjustable region, a solid one for a hover
    // highlight. Both are a single shader element rather than one quad per
    // dash, which used to be several hundred elements on a wide selection.
    let dash = if region_style {
        (PICKER_DASH_PERIOD, PICKER_DASH_LENGTH)
    } else {
        (1.0, 1.0)
    };
    elements.push(SceneElement::DashedOutline(
        node_renderer.dashed_outline_element(
            renderer,
            output_name,
            NodeSlot::PickerOutline,
            layout.outline,
            accent,
            PICKER_BORDER_PX,
            dash,
        )?,
    ));

    if region_style {
        for (index, rect) in layout.handles.iter().enumerate() {
            let id = node_renderer.slot_id(output_name, NodeSlot::PickerHandle(index as u8));
            elements.push(SceneElement::Border(SolidColorRenderElement::new(
                id,
                *rect,
                CommitCounter::default(),
                smithay::backend::renderer::Color32F::new(accent.0, accent.1, accent.2, accent.3),
                Kind::Unspecified,
            )));
        }
    }
    Ok(elements)
}

#[cfg(test)]
mod dim_tests {
    #[test]
    fn screenshot_backdrop_uses_the_shell_dim_color() {
        assert_eq!(
            crate::render::overlays::shell::backdrop_dim(0.62).components(),
            [0.02, 0.03, 0.05, 0.62]
        );
    }
}
