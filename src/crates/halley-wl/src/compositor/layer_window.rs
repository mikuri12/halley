//! Promoción de superficies layer-shell a nodos del Field ("layer windows").
//!
//! Una superficie promovida deja de participar en la colocación/zona exclusiva
//! del layer-shell (se filtra en `layer_shell_placements_for_monitor`) y pasa a
//! ser un nodo normal: render por `collect_active_surfaces`, foco por
//! `wl_surface_for_node`, input de puntero por el hit-test del Field, y
//! tile/stack dentro de clusters como cualquier ventana.
//!
//! El nodo vive mientras el mapa de la superficie exista: cuando el cliente
//! destruye la layer (p. ej. el OSD de volumen que se cierra solo),
//! `remove_layer_surface_impl` despawnea el nodo.

use std::time::Instant;

use halley_core::field::{NodeId, Vec2};
use smithay::desktop::utils::bbox_from_surface_tree;
use smithay::reexports::wayland_server::{Resource, protocol::wl_surface::WlSurface};
use smithay::utils::{Logical, Rectangle};
use smithay::wayland::compositor::{RectangleKind, SurfaceAttributes, with_states};

use crate::compositor::monitor::layer_shell::{
    layer_surface_monitor_name, layer_shell_placements_for_monitor,
};
use crate::compositor::root::Halley;

/// Promueve la raíz de una superficie layer-shell a nodo. Idempotente: si ya
/// está promovida devuelve su nodo. La posición inicial del nodo es la
/// posición visual actual de la layer (en coords de mundo del monitor al que
/// está asignada), de modo que el "pop-out" ocurre en el sitio.
pub(crate) fn promote_layer_surface_to_node(st: &mut Halley, surface: &WlSurface) -> NodeId {
    let key = surface.id();
    if let Some(existing) = st.model.surface_to_node.get(&key).copied() {
        return existing;
    }

    let monitor = layer_surface_monitor_name(st, surface);
    let output_size = crate::compositor::monitor::layer_shell::layer_output_size_for_monitor(
        st,
        monitor.as_str(),
    );
    let placement = layer_shell_placements_for_monitor(st, monitor.as_str())
        .into_iter()
        .find(|placement| placement.wl_surface.id() == key);
    let (origin, size) = match placement {
        Some(placement) => (placement.origin, placement.size),
        // Sin colocación (p. ej. sin commit aún): centrado en el monitor.
        None => (
            (0, 0).into(),
            (output_size.w.min(480).max(64), output_size.h.min(360).max(64)).into(),
        ),
    };

    let mut center_screen = (
        origin.x as f32 + size.w as f32 / 2.0,
        origin.y as f32 + size.h as f32 / 2.0,
    );
    let mut intrinsic = Vec2 {
        x: size.w.max(64) as f32,
        y: size.h.max(64) as f32,
    };

    // Contenido real si la layer recorta su input region (sombra dentro del
    // buffer): el nodo y su textura abrazan el contenido, no el padding.
    if let Some(content) =
        layer_surface_content_region(surface, bbox_from_surface_tree(surface, (0, 0)))
    {
        center_screen.0 = (origin.x + content.loc.x + content.size.w / 2) as f32;
        center_screen.1 = (origin.y + content.loc.y + content.size.h / 2) as f32;
        intrinsic.x = content.size.w.max(64) as f32;
        intrinsic.y = content.size.h.max(64) as f32;
    }

    let pos = monitor_screen_center_to_world(
        st,
        monitor.as_str(),
        (output_size.w.max(1), output_size.h.max(1)),
        center_screen,
    );

    let namespace = st
        .model
        .monitor_state
        .layer_surface_namespace
        .get(&key)
        .cloned();
    let label = namespace
        .as_deref()
        .and_then(compact_namespace_label)
        .unwrap_or_else(|| "Layer Window".to_string());

    let now = Instant::now();
    let node_id = st.model.field.spawn_surface(label, pos, intrinsic);
    st.model.surface_to_node.insert(key, node_id);
    st.model
        .node_layer_surfaces
        .insert(node_id, surface.clone());
    st.assign_node_to_monitor(node_id, monitor.as_str());
    if let Some(namespace) = namespace {
        st.model.node_app_ids.insert(node_id, namespace);
    }
    let _ = st
        .model
        .field
        .set_state(node_id, halley_core::field::NodeState::Active);
    let _ = st
        .model
        .field
        .set_decay_level(node_id, halley_core::decay::DecayLevel::Hot);
    st.ui.render_state.cache.zoom_nominal_size.insert(node_id, intrinsic);
    st.model.workspace_state.last_active_size.insert(node_id, intrinsic);
    if st.runtime.tuning.animations_enabled() {
        st.ui
            .render_state
            .animator
            .observe_field(&st.model.field, now);
    }

    // La layer deja de reservar zona exclusiva / colocarse: reflow del área
    // usable y foco al nodo recién creado.
    crate::compositor::monitor::layer_shell::refresh_monitor_usable_viewports(st);
    st.set_interaction_focus(Some(node_id), 30_000, now);
    st.request_maintenance();
    node_id
}

/// Despromueve el nodo de una layer: destruye el nodo (silenciosamente, sin
/// animación de cierre) y devuelve la superficie a la colocación layer-shell.
pub(crate) fn demote_layer_surface_node(st: &mut Halley, surface: &WlSurface) -> Option<NodeId> {
    let key = surface.id();
    let node_id = st.model.surface_to_node.get(&key).copied()?;
    let monitor = layer_surface_monitor_name(st, surface);

    // Silenciar la animación de cierre: la superficie sigue viva y vuelve a
    // renderizarse como layer en el siguiente frame.
    st.model
        .workspace_state
        .pending_silent_close_until_ms
        .insert(node_id, st.now_ms(Instant::now()).saturating_add(50));
    crate::compositor::ctx::surface_lifecycle_ctx(st).drop_surface(surface);

    // Reconciliar la colocación: descartar el tamaño configurado previo y
    // dejar que el cliente se reconfigure al tamaño de su anclaje.
    st.model
        .monitor_state
        .layer_surface_last_configured_size
        .remove(&key);
    crate::compositor::monitor::layer_shell::configure_layer_shell_surfaces_for_monitor(
        st,
        monitor.as_str(),
    );
    crate::compositor::monitor::layer_shell::refresh_monitor_usable_viewports(st);
    st.request_maintenance();
    Some(node_id)
}

/// Coordenadas de pantalla lógicas del monitor → mundo, para el centro del
/// nodo promovido. Para el monitor actual usa la cámara viva
/// (`screen_to_world`); para otros monitores la cámara asentada del
/// `MonitorSpace` (la misma normalización de `screen_to_world`).
fn monitor_screen_center_to_world(
    st: &Halley,
    monitor: &str,
    output_size: (i32, i32),
    screen: (f32, f32),
) -> Vec2 {
    if monitor == st.model.monitor_state.current_monitor {
        return crate::spatial::screen_to_world(st, output_size.0, output_size.1, screen.0, screen.1);
    }
    let Some(space) = st.model.monitor_state.monitors.get(monitor) else {
        return st.model.viewport.center;
    };
    let w = output_size.0.max(1) as f32;
    let h = output_size.1.max(1) as f32;
    let view = space.viewport.size;
    Vec2 {
        x: space.viewport.center.x + ((screen.0 / w) - 0.5) * view.x.max(1.0),
        y: space.viewport.center.y + ((screen.1 / h) - 0.5) * view.y.max(1.0),
    }
}

/// Identidad de nodo para una layer promovida: etiqueta y app_id derivados del
/// namespace. La usan tanto el promote como `refresh_node_identity_for_surface`
/// (los commits de una layer no tienen datos de rol xdg, y sin esto la
/// etiqueta se pisaría con el fallback genérico).
pub(crate) fn layer_surface_identity(
    st: &Halley,
    root_key: &smithay::reexports::wayland_server::backend::ObjectId,
) -> Option<(Option<String>, Option<String>)> {
    let namespace = st
        .model
        .monitor_state
        .layer_surface_namespace
        .get(root_key)?
        .clone();
    let label = compact_namespace_label(namespace.as_str());
    Some((label, Some(namespace)))
}

/// ¿Es esta superficie (o la raíz de su árbol) una layer promovida a nodo?
pub(crate) fn is_promoted_layer_surface(st: &Halley, surface: &WlSurface) -> bool {
    if st.model.surface_to_node.contains_key(&surface.id()) {
        return true;
    }
    let mut current = surface.clone();
    while let Some(parent) = smithay::wayland::compositor::get_parent(&current) {
        current = parent;
    }
    st.model.surface_to_node.contains_key(&current.id())
}

/// Heurística de "shield": capa Top/Overlay que cubre (casi) toda la salida.
/// Los shells usan estas capas fullscreen invisibles (click-shields, backdrops)
/// para cerrar sus popups al click afuera; no son widgets.
fn layer_resembles_shield(
    layer: smithay::wayland::shell::wlr_layer::Layer,
    size: (i32, i32),
    ws_w: i32,
    ws_h: i32,
) -> bool {
    if !matches!(
        layer,
        smithay::wayland::shell::wlr_layer::Layer::Top
            | smithay::wayland::shell::wlr_layer::Layer::Overlay
    ) {
        return false;
    }
    let area = (size.0.max(0) as i64) * (size.1.max(0) as i64);
    let output = (ws_w.max(1) as i64) * (ws_h.max(1) as i64);
    area >= output * 85 / 100
}

/// ¿Parece esta superficie un shield/backdrop (capa Top/Overlay que cubre casi
/// toda la salida de su monitor)? Resuelve la colocación layer-shell de la
/// raíz del árbol de la superficie.
pub(crate) fn layer_resembles_fullscreen_shield(st: &Halley, surface: &WlSurface) -> bool {
    let root =
        crate::compositor::monitor::layer_shell::layer_surface_root_for_surface(st, surface)
            .unwrap_or_else(|| surface.clone());
    let monitor = crate::compositor::monitor::layer_shell::layer_surface_monitor_name(st, &root);
    let output_size =
        crate::compositor::monitor::layer_shell::layer_output_size_for_monitor(st, monitor.as_str());
    let (ws_w, ws_h) = (output_size.w, output_size.h);
    crate::compositor::monitor::layer_shell::layer_shell_placements_for_monitor(
        st,
        monitor.as_str(),
    )
    .into_iter()
    .any(|placement| {
        placement.wl_surface.id() == root.id()
            && layer_resembles_shield(placement.layer, (placement.size.w, placement.size.h), ws_w, ws_h)
    })
}

/// ¿Debe esta colocación layer-shell hacerse click-through porque una layer
/// de su mismo cliente está promovida a nodo? Los shells (Noctalia) cubren la
/// salida con una capa invisible ("click-shield") para cerrar sus popups al
/// click afuera; mientras el popup vive como ventana del Field, esa capa no
/// debe comerse el input — cerraría la ventana al click y bloquearía el
/// canvas. Solo aplica a capas Top/Overlay que cubren (casi) toda la salida.
pub(crate) fn layer_shielded_by_promotion(
    st: &Halley,
    surface: &WlSurface,
    layer: smithay::wayland::shell::wlr_layer::Layer,
    size: (i32, i32),
    ws_w: i32,
    ws_h: i32,
) -> bool {
    if !layer_resembles_shield(layer, size, ws_w, ws_h) {
        return false;
    }
    let Some(client) = surface.client() else {
        return false;
    };
    st.model.node_layer_surfaces.values().any(|promoted| {
        promoted
            .client()
            .is_some_and(|promoted_client| promoted_client.id() == client.id())
    })
}

/// Región de contenido de una layer: el bounding box de su input region (en
/// coords de superficie). Shells como Noctalia dibujan la sombra del panel
/// DENTRO del buffer y recortan el input region al contenido visible; sin
/// esto, el nodo abrazaría el buffer completo y entre su borde y el contenido
/// quedaría un anillo transparente del grosor de la sombra. Devuelve None si
/// no hay input region o si este cubre (casi) todo el buffer.
pub(crate) fn layer_surface_content_region(
    surface: &WlSurface,
    surface_bbox: Rectangle<i32, Logical>,
) -> Option<Rectangle<i32, Logical>> {
    let region = with_states(surface, |states| {
        states
            .cached_state
            .get::<SurfaceAttributes>()
            .current()
            .input_region
            .clone()
    })?;
    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;
    for (kind, rect) in &region.rects {
        if !matches!(kind, RectangleKind::Add) {
            continue;
        }
        min_x = min_x.min(rect.loc.x);
        min_y = min_y.min(rect.loc.y);
        max_x = max_x.max(rect.loc.x + rect.size.w);
        max_y = max_y.max(rect.loc.y + rect.size.h);
    }
    if min_x > max_x || min_y > max_y {
        return None;
    }
    let shrunk =
        surface_bbox.size.w - (max_x - min_x) >= 2 || surface_bbox.size.h - (max_y - min_y) >= 2;
    if !shrunk {
        return None;
    }
    let region_bbox = Rectangle {
        loc: (min_x, min_y).into(),
        size: (max_x - min_x, max_y - min_y).into(),
    };
    region_bbox.intersection(surface_bbox)
}

fn compact_namespace_label(namespace: &str) -> Option<String> {
    let tail = namespace.rsplit('.').next().unwrap_or(namespace);
    if tail.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(tail.len());
    let mut upper_next = true;
    for ch in tail.chars() {
        if matches!(ch, '-' | '_' | '.') {
            if !out.ends_with(' ') {
                out.push(' ');
            }
            upper_next = true;
            continue;
        }
        if upper_next {
            out.extend(ch.to_uppercase());
            upper_next = false;
        } else {
            out.push(ch);
        }
    }
    Some(out.trim().to_string()).filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_label_title_cases_kebab_namespace() {
        assert_eq!(
            compact_namespace_label("noctalia-osd").as_deref(),
            Some("Noctalia Osd")
        );
        assert_eq!(
            compact_namespace_label("noctalia-desktop-widget-clock").as_deref(),
            Some("Noctalia Desktop Widget Clock")
        );
        assert_eq!(compact_namespace_label("").as_deref(), None);
    }
}
