use std::error::Error;

use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::surface::{
    WaylandSurfaceRenderElement, render_elements_from_surface_tree,
};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::utils::{SurfaceView, with_renderer_surface_state};
use smithay::input::pointer::CursorIcon;
use smithay::output::Output;
use smithay::utils::{Buffer, Logical, Physical, Point, Rectangle, Scale, Size, Transform};

use super::{CursorFrame, CursorManager, CursorSurfaceSnapshot, RenderCursor};

smithay::backend::renderer::element::render_elements! {
    pub CursorRenderElement<=GlesRenderer>;
    Named=MemoryRenderBufferRenderElement<GlesRenderer>,
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
}

pub fn elements(
    renderer: &mut GlesRenderer,
    manager: &CursorManager,
    output: &Output,
    output_geometry: Rectangle<i32, Logical>,
    pointer_position: (f64, f64),
    time: std::time::Duration,
    presentation_override: Option<CursorIcon>,
) -> Result<Vec<CursorRenderElement>, Box<dyn Error>> {
    match manager.render_cursor_with_override(
        output.current_scale().integer_scale(),
        time,
        presentation_override,
    ) {
        RenderCursor::Hidden => Ok(Vec::new()),
        RenderCursor::Named(frame) => {
            let Some(position) =
                named_cursor_origin(output, output_geometry, pointer_position, &frame)
            else {
                return Ok(Vec::new());
            };
            Ok(vec![
                MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    position,
                    &frame.buffer,
                    None,
                    None,
                    None,
                    Kind::Cursor,
                )?
                .into(),
            ])
        }
        RenderCursor::Surface { surface, snapshot } => {
            let Some(position) = surface_cursor_origin(
                output,
                output_geometry,
                pointer_position,
                super::surface::hotspot(&surface),
            ) else {
                return Ok(Vec::new());
            };
            let elements = surface_elements(
                renderer,
                &surface,
                snapshot.as_deref(),
                position,
                Scale::from(output.current_scale().fractional_scale()),
                Kind::Cursor,
            )?;
            // A live, bufferless cursor surface is a valid client request for
            // an invisible cursor. Firefox uses this while video controls are
            // hidden. Session resume and surface destruction already reset
            // genuinely stale cursor state, so synthesizing an arrow here
            // overrides the client's visibility decision.
            match client_surface_presentation(elements) {
                ClientSurfacePresentation::Hidden => Ok(Vec::new()),
                ClientSurfacePresentation::Visible(elements) => Ok(elements),
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ClientSurfacePresentation<T> {
    Hidden,
    Visible(Vec<T>),
}

fn client_surface_presentation<T>(elements: Vec<T>) -> ClientSurfacePresentation<T> {
    if elements.is_empty() {
        ClientSurfacePresentation::Hidden
    } else {
        ClientSurfacePresentation::Visible(elements)
    }
}

pub(crate) fn surface_elements(
    renderer: &mut GlesRenderer,
    surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    snapshot: Option<&CursorSurfaceSnapshot>,
    position: Point<i32, Physical>,
    scale: Scale<f64>,
    kind: Kind,
) -> Result<Vec<CursorRenderElement>, Box<dyn Error>> {
    // SHM cursors are snapshotted before Smithay consumes the pending buffer.
    // Importing the live tree afterward still walks that surface-scoped GLES
    // cache: leftover damage from a 24x24 cursor is TexSubImage'd into the
    // previous 10x16 texture (`GL_INVALID_VALUE`). Draw only the snapshot.
    if let Some(snapshot) = snapshot {
        let view = with_renderer_surface_state(surface, |state| state.view()).flatten();
        return Ok(vec![snapshot_root(
            renderer, snapshot, view, position, scale, kind,
        )?]);
    }
    let elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
        render_elements_from_surface_tree(renderer, surface, position, scale, 1.0, kind);
    Ok(elements.into_iter().map(Into::into).collect())
}

struct SnapshotPlacement {
    location: Point<f64, Physical>,
    src: Option<Rectangle<f64, Logical>>,
    dst: Option<Size<i32, Logical>>,
}

fn snapshot_root(
    renderer: &mut GlesRenderer,
    snapshot: &CursorSurfaceSnapshot,
    view: Option<SurfaceView>,
    position: Point<i32, Physical>,
    scale: Scale<f64>,
    kind: Kind,
) -> Result<CursorRenderElement, Box<dyn Error>> {
    let placement = snapshot_placement(snapshot, view, position, scale);
    Ok(MemoryRenderBufferRenderElement::from_buffer(
        renderer,
        placement.location,
        &snapshot.buffer,
        None,
        placement.src,
        placement.dst,
        kind,
    )?
    .into())
}

fn snapshot_logical_size(
    width: u32,
    height: u32,
    scale: i32,
    transform: Transform,
) -> Size<i32, Logical> {
    Size::<i32, Buffer>::from((width as i32, height as i32)).to_logical(scale.max(1), transform)
}

fn snapshot_placement(
    snapshot: &CursorSurfaceSnapshot,
    view: Option<SurfaceView>,
    position: Point<i32, Physical>,
    scale: Scale<f64>,
) -> SnapshotPlacement {
    snapshot_placement_from_size(
        snapshot_logical_size(
            snapshot.width,
            snapshot.height,
            snapshot.scale,
            snapshot.transform,
        ),
        view,
        position,
        scale,
    )
}

fn snapshot_placement_from_size(
    logical: Size<i32, Logical>,
    view: Option<SurfaceView>,
    position: Point<i32, Physical>,
    scale: Scale<f64>,
) -> SnapshotPlacement {
    match view.filter(|view| view.dst == logical) {
        Some(view) => SnapshotPlacement {
            location: position.to_f64() + view.offset.to_f64().to_physical(scale),
            src: Some(view.src),
            dst: Some(view.dst),
        },
        None => SnapshotPlacement {
            location: position.to_f64(),
            src: None,
            dst: Some(logical),
        },
    }
}

fn named_cursor_origin(
    output: &Output,
    output_geometry: Rectangle<i32, Logical>,
    pointer_position: (f64, f64),
    frame: &CursorFrame,
) -> Option<Point<f64, Physical>> {
    let pointer = Point::<f64, Logical>::from(pointer_position);
    if !output_geometry.to_f64().contains(pointer) {
        return None;
    }
    let local = pointer - output_geometry.loc.to_f64();
    let hotspot = Point::<f64, Logical>::from((
        f64::from(frame.hotspot_x) / f64::from(frame.scale.max(1)),
        f64::from(frame.hotspot_y) / f64::from(frame.scale.max(1)),
    ));
    Some((local - hotspot).to_physical(Scale::from(output.current_scale().fractional_scale())))
}

fn surface_cursor_origin(
    output: &Output,
    output_geometry: Rectangle<i32, Logical>,
    pointer_position: (f64, f64),
    hotspot: Point<i32, Logical>,
) -> Option<Point<i32, Physical>> {
    let pointer = Point::<f64, Logical>::from(pointer_position);
    if !output_geometry.to_f64().contains(pointer) {
        return None;
    }
    Some(
        (pointer - output_geometry.loc.to_f64() - hotspot.to_f64())
            .to_physical_precise_round(Scale::from(output.current_scale().fractional_scale())),
    )
}

#[cfg(test)]
mod tests {
    use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
    use smithay::output::{Mode, PhysicalProperties, Subpixel};
    use smithay::utils::{Physical, Size, Transform};

    use super::*;

    fn output(scale: i32) -> Output {
        let output = Output::new(
            "test".to_string(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "test".into(),
                model: "test".into(),
                serial_number: "test".into(),
            },
        );
        output.change_current_state(
            Some(Mode {
                size: Size::<i32, Physical>::from((1920, 1080)),
                refresh: 60_000,
            }),
            Some(Transform::Normal),
            Some(smithay::output::Scale::Integer(scale)),
            Some((0, 0).into()),
        );
        output
    }

    #[test]
    fn bufferless_client_cursor_surface_remains_invisible() {
        assert_eq!(
            client_surface_presentation::<()>(Vec::new()),
            ClientSurfacePresentation::Hidden
        );
    }

    #[test]
    fn buffered_client_cursor_surface_remains_visible() {
        assert_eq!(
            client_surface_presentation(vec!["cursor-element"]),
            ClientSurfacePresentation::Visible(vec!["cursor-element"])
        );
    }

    #[test]
    fn hotspot_is_removed_in_logical_then_output_physical_space() {
        let output = output(2);
        let geometry = Rectangle::new((100, 50).into(), (960, 540).into());

        assert_eq!(
            named_cursor_origin(
                &output,
                geometry,
                (120.0, 70.0),
                &CursorFrame {
                    buffer: MemoryRenderBuffer::from_slice(
                        &[0, 0, 0, 0],
                        smithay::backend::allocator::Fourcc::Abgr8888,
                        (1, 1),
                        2,
                        Transform::Normal,
                        None,
                    ),
                    metadata_bgra: vec![0, 0, 0, 0].into(),
                    width: 1,
                    height: 1,
                    hotspot_x: 4,
                    hotspot_y: 6,
                    scale: 2,
                }
            ),
            Some(Point::from((36.0, 34.0)))
        );
    }

    #[test]
    fn pointer_outside_output_produces_no_cursor() {
        let output = output(1);
        let geometry = Rectangle::new((0, 0).into(), (1920, 1080).into());

        assert_eq!(
            named_cursor_origin(
                &output,
                geometry,
                (1920.0, 100.0),
                &CursorFrame {
                    buffer: MemoryRenderBuffer::from_slice(
                        &[0, 0, 0, 0],
                        smithay::backend::allocator::Fourcc::Abgr8888,
                        (1, 1),
                        1,
                        Transform::Normal,
                        None,
                    ),
                    metadata_bgra: vec![0, 0, 0, 0].into(),
                    width: 1,
                    height: 1,
                    hotspot_x: 0,
                    hotspot_y: 0,
                    scale: 1,
                }
            ),
            None
        );
    }

    #[test]
    fn secondary_output_localizes_global_pointer_coordinates() {
        let output = output(1);
        let geometry = Rectangle::new((2560, 0).into(), (1920, 1200).into());

        assert_eq!(
            named_cursor_origin(
                &output,
                geometry,
                (3000.0, 500.0),
                &CursorFrame {
                    buffer: MemoryRenderBuffer::from_slice(
                        &[0, 0, 0, 0],
                        smithay::backend::allocator::Fourcc::Abgr8888,
                        (1, 1),
                        1,
                        Transform::Normal,
                        None,
                    ),
                    metadata_bgra: vec![0, 0, 0, 0].into(),
                    width: 1,
                    height: 1,
                    hotspot_x: 0,
                    hotspot_y: 0,
                    scale: 1,
                }
            ),
            Some(Point::from((440.0, 500.0)))
        );
    }

    #[test]
    fn matching_surface_view_keeps_offset_and_crop() {
        let logical = Size::<i32, Logical>::from((10, 16));
        let view = SurfaceView {
            src: Rectangle::new((0.0, 0.0).into(), (10.0, 16.0).into()),
            dst: logical,
            offset: Point::from((2, -1)),
        };
        let placement = snapshot_placement_from_size(
            logical,
            Some(view),
            Point::from((40, 80)),
            Scale::from(1.0),
        );

        assert_eq!(placement.location, Point::from((42.0, 79.0)));
        assert_eq!(placement.src, Some(view.src));
        assert_eq!(placement.dst, Some(logical));
    }

    #[test]
    fn stale_24x24_view_does_not_crop_a_10x16_snapshot() {
        let logical = Size::<i32, Logical>::from((10, 16));
        let view = SurfaceView {
            src: Rectangle::new((0.0, 0.0).into(), (24.0, 24.0).into()),
            dst: Size::from((24, 24)),
            offset: Point::from((0, 0)),
        };
        let placement = snapshot_placement_from_size(
            logical,
            Some(view),
            Point::from((40, 80)),
            Scale::from(1.0),
        );

        assert_eq!(placement.location, Point::from((40.0, 80.0)));
        assert_eq!(placement.src, None);
        assert_eq!(placement.dst, Some(logical));
    }
}
