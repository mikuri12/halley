use halley_api::{
    ApiError, LayerAnchorEdge, LayerInfo, LayerKeyboardInteractivity, LayerListResponse,
    LayerOutputGroup, LayerPromoteResponse, LayerRequest, LayerShellKind, Response,
};
use smithay::reexports::wayland_server::Resource;
use smithay::wayland::shell::wlr_layer::{Anchor, ExclusiveZone, KeyboardInteractivity, Layer};

use crate::compositor::layer_window::{demote_layer_surface_node, promote_layer_surface_to_node};
use crate::compositor::monitor::layer_shell::{
    LayerSurfaceSummary, layer_surface_summaries_for_monitor,
};
use crate::compositor::root::Halley;

use super::{sorted_outputs, validate_output};

pub(super) fn handle_layer_request(st: &mut Halley, request: LayerRequest) -> Response {
    match request {
        LayerRequest::List { output } => match list_layers(st, output.as_deref()) {
            Ok(outputs) => Response::LayerList(outputs),
            Err(err) => Response::Error(err),
        },
        LayerRequest::Promote { handle } => match layer_surface_for_handle(st, handle) {
            Ok(surface) => {
                let node_id = promote_layer_surface_to_node(st, &surface);
                Response::LayerPromoted(LayerPromoteResponse {
                    node_id: node_id.as_u64(),
                })
            }
            Err(err) => Response::Error(err),
        },
        LayerRequest::Demote { handle } => match layer_surface_for_handle(st, handle) {
            Ok(surface) => match demote_layer_surface_node(st, &surface) {
                Some(_) => Response::Ok,
                None => Response::Error(ApiError::NotFound(format!(
                    "layer surface {handle} is not promoted"
                ))),
            },
            Err(err) => Response::Error(err),
        },
    }
}

/// Resuelve el handle estable de una layer a su wl_surface raíz.
fn layer_surface_for_handle(
    st: &mut Halley,
    handle: u64,
) -> Result<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface, ApiError> {
    st.platform
        .wlr_layer_shell_state
        .layer_surfaces()
        .filter(|surface| surface.alive())
        .find(|surface| {
            st.model
                .monitor_state
                .layer_surface_handles
                .get(&surface.wl_surface().id())
                .copied()
                == Some(handle)
        })
        .map(|surface| surface.wl_surface().clone())
        .ok_or_else(|| ApiError::NotFound(format!("no layer surface with handle {handle}")))
}

fn list_layers(st: &Halley, output: Option<&str>) -> Result<LayerListResponse, ApiError> {
    let outputs: Vec<String> = match output {
        Some(name) => vec![validate_output(st, name)?.to_string()],
        None => sorted_outputs(st),
    };
    let groups = outputs
        .into_iter()
        .map(|output| LayerOutputGroup {
            layers: layer_surface_summaries_for_monitor(st, output.as_str())
                .into_iter()
                .map(|summary| layer_info(summary, output.as_str()))
                .collect(),
            output,
        })
        .collect();
    Ok(LayerListResponse { outputs: groups })
}

fn layer_info(summary: LayerSurfaceSummary, output: &str) -> LayerInfo {
    LayerInfo {
        id: summary.handle,
        namespace: summary.namespace,
        output: Some(output.to_string()),
        layer: match summary.layer {
            Layer::Background => LayerShellKind::Background,
            Layer::Bottom => LayerShellKind::Bottom,
            Layer::Top => LayerShellKind::Top,
            Layer::Overlay => LayerShellKind::Overlay,
        },
        anchor: anchor_edges(summary.anchor),
        exclusive_zone: match summary.exclusive_zone {
            ExclusiveZone::Exclusive(value) => Some(value as i32),
            ExclusiveZone::Neutral | ExclusiveZone::DontCare => None,
        },
        keyboard_interactivity: match summary.keyboard_interactivity {
            KeyboardInteractivity::None => LayerKeyboardInteractivity::None,
            KeyboardInteractivity::OnDemand => LayerKeyboardInteractivity::OnDemand,
            KeyboardInteractivity::Exclusive => LayerKeyboardInteractivity::Exclusive,
        },
        keyboard_focus: summary.keyboard_focus,
        committed: summary.committed,
        promoted_node: summary.promoted_node.map(|id| id.as_u64()),
        pos_x: summary.origin.x,
        pos_y: summary.origin.y,
        width: summary.size.w,
        height: summary.size.h,
    }
}

fn anchor_edges(anchor: Anchor) -> Vec<LayerAnchorEdge> {
    let mut edges = Vec::new();
    if anchor.contains(Anchor::TOP) {
        edges.push(LayerAnchorEdge::Top);
    }
    if anchor.contains(Anchor::BOTTOM) {
        edges.push(LayerAnchorEdge::Bottom);
    }
    if anchor.contains(Anchor::LEFT) {
        edges.push(LayerAnchorEdge::Left);
    }
    if anchor.contains(Anchor::RIGHT) {
        edges.push(LayerAnchorEdge::Right);
    }
    edges
}
