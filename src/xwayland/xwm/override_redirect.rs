use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OverrideRedirectOwnerSource {
    Transient,
    WindowGroup,
    FullscreenGrabProxy,
}

pub(super) struct OverrideRedirectOwner {
    window: Window,
    source: OverrideRedirectOwnerSource,
}

pub(super) struct OverrideRedirectResolution {
    owner: Option<OverrideRedirectOwner>,
    output: Option<Output>,
}

pub(super) fn point_rect_distance_squared(
    point: Point<i32, Logical>,
    geometry: Rectangle<i32, Logical>,
) -> i64 {
    let right = geometry.loc.x.saturating_add(geometry.size.w);
    let bottom = geometry.loc.y.saturating_add(geometry.size.h);
    let dx = if point.x < geometry.loc.x {
        i64::from(geometry.loc.x - point.x)
    } else if point.x > right {
        i64::from(point.x - right)
    } else {
        0
    };
    let dy = if point.y < geometry.loc.y {
        i64::from(geometry.loc.y - point.y)
    } else if point.y > bottom {
        i64::from(point.y - bottom)
    } else {
        0
    };
    dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy))
}

#[derive(Clone, Copy)]
pub(super) struct OverrideRedirectIdentity<'a> {
    pub(super) pid: Option<u32>,
    pub(super) class: &'a str,
    pub(super) instance: &'a str,
}

pub(super) fn override_redirect_owner_rank(
    popup: OverrideRedirectIdentity<'_>,
    candidate: OverrideRedirectIdentity<'_>,
    center: Point<i32, Logical>,
    candidate_geometry: Rectangle<i32, Logical>,
    stack_index: usize,
) -> (bool, bool, bool, Reverse<i64>, usize) {
    (
        popup.pid.is_some() && popup.pid == candidate.pid,
        !popup.class.is_empty()
            && popup.class == candidate.class
            && popup.instance == candidate.instance,
        candidate_geometry.contains(center),
        Reverse(point_rect_distance_squared(center, candidate_geometry)),
        stack_index,
    )
}

pub(super) fn mapped_managed_x11_window<D: SessionDriver>(
    session: &Session<D>,
    xid: u32,
) -> Option<Window> {
    session.wayland.space.elements().find_map(|window| {
        window
            .x11_surface()
            .filter(|candidate| candidate.window_id() == xid && !candidate.is_override_redirect())
            .map(|_| window.clone())
    })
}

pub(super) fn transient_override_redirect_owner<D: SessionDriver>(
    session: &Session<D>,
    surface: &X11Surface,
) -> Option<Window> {
    let mut xid = surface.is_transient_for()?;
    let mut visited = Vec::new();
    for _ in 0..16 {
        if visited.contains(&xid) {
            return None;
        }
        visited.push(xid);
        if let Some(owner) = mapped_managed_x11_window(session, xid) {
            return Some(owner);
        }
        let candidate = session.wayland.space.elements().find_map(|window| {
            window
                .x11_surface()
                .filter(|candidate| candidate.window_id() == xid)
                .cloned()
        })?;
        xid = candidate.is_transient_for()?;
    }
    None
}

pub(super) fn window_group_override_redirect_owner<D: SessionDriver>(
    session: &Session<D>,
    surface: &X11Surface,
    geometry: Rectangle<i32, Logical>,
) -> Option<Window> {
    let group = surface.hints()?.window_group?;
    let center = Point::from((
        geometry.loc.x.saturating_add(geometry.size.w / 2),
        geometry.loc.y.saturating_add(geometry.size.h / 2),
    ));
    let popup_pid = surface.pid();
    let popup_class = surface.class();
    let popup_instance = surface.instance();

    session
        .wayland
        .space
        .elements()
        .enumerate()
        .filter_map(|(stack_index, window)| {
            let candidate = window.x11_surface()?;
            if candidate.is_override_redirect() || candidate.hints()?.window_group != Some(group) {
                return None;
            }
            // The popup geometry and the candidate's X11 geometry are both in
            // root-desktop coordinates.  Comparing either one to `Space`
            // would mix screen and Field coordinates whenever the camera is
            // panned or zoomed.
            let candidate_geometry = candidate.geometry();
            Some((
                override_redirect_owner_rank(
                    OverrideRedirectIdentity {
                        pid: popup_pid,
                        class: &popup_class,
                        instance: &popup_instance,
                    },
                    OverrideRedirectIdentity {
                        pid: candidate.pid(),
                        class: &candidate.class(),
                        instance: &candidate.instance(),
                    },
                    center,
                    candidate_geometry,
                    stack_index,
                ),
                window.clone(),
            ))
        })
        .max_by_key(|(key, _)| *key)
        .map(|(_, window)| window)
}

const FULLSCREEN_GRAB_PROXY_EDGE_TOLERANCE: i64 = 48;

pub(super) fn fullscreen_grab_proxy_geometry_matches(
    proxy: Rectangle<i32, Logical>,
    candidate: Rectangle<i32, Logical>,
) -> bool {
    if proxy.size.w <= 0 || proxy.size.h <= 0 || candidate.size.w <= 0 || candidate.size.h <= 0 {
        return false;
    }
    let edges = |geometry: Rectangle<i32, Logical>| {
        [
            i64::from(geometry.loc.x),
            i64::from(geometry.loc.y),
            i64::from(geometry.loc.x) + i64::from(geometry.size.w),
            i64::from(geometry.loc.y) + i64::from(geometry.size.h),
        ]
    };
    edges(proxy)
        .into_iter()
        .zip(edges(candidate))
        .all(|(proxy, candidate)| {
            proxy.abs_diff(candidate) <= FULLSCREEN_GRAB_PROXY_EDGE_TOLERANCE as u64
        })
}

/// Proton can implement an X grab with a separate override-redirect X window
/// whose Wayland surface owns the pointer constraint. Some games do not set a
/// transient or window-group relationship on that window. Associate only a
/// screen-sized proxy with a fullscreen managed X11 window on the same output;
/// small menus and helpers cannot pass this fallback.
pub(super) fn fullscreen_grab_proxy_owner<D: SessionDriver>(
    session: &Session<D>,
    geometry: Rectangle<i32, Logical>,
) -> Option<Window> {
    let output = output_for_geometry(session, geometry)?;
    let focused = session.wayland.focused_window.as_ref();
    let matches = session
        .wayland
        .space
        .elements()
        .filter(|window| {
            let Some(surface) = window.x11_surface() else {
                return false;
            };
            !surface.is_override_redirect()
                && surface.is_fullscreen()
                && crate::wayland::window_output_name(window).as_deref()
                    == Some(output.name().as_str())
                && fullscreen_grab_proxy_geometry_matches(geometry, surface.geometry())
        })
        .cloned()
        .collect::<Vec<_>>();

    matches
        .iter()
        .find(|window| window.wl_surface().as_deref() == focused)
        .cloned()
        .or_else(|| (matches.len() == 1).then(|| matches[0].clone()))
}

pub(super) fn resolve_override_redirect_owner<D: SessionDriver>(
    session: &Session<D>,
    surface: &X11Surface,
    geometry: Rectangle<i32, Logical>,
) -> Option<OverrideRedirectOwner> {
    transient_override_redirect_owner(session, surface)
        .map(|window| OverrideRedirectOwner {
            window,
            source: OverrideRedirectOwnerSource::Transient,
        })
        .or_else(|| {
            window_group_override_redirect_owner(session, surface, geometry).map(|window| {
                OverrideRedirectOwner {
                    window,
                    source: OverrideRedirectOwnerSource::WindowGroup,
                }
            })
        })
        .or_else(|| {
            fullscreen_grab_proxy_owner(session, geometry).map(|window| OverrideRedirectOwner {
                window,
                source: OverrideRedirectOwnerSource::FullscreenGrabProxy,
            })
        })
}

pub(super) fn output_named(wayland: &crate::wayland::WaylandState, name: &str) -> Option<Output> {
    wayland
        .space
        .outputs()
        .find(|output| output.name() == name)
        .cloned()
}

pub(super) fn override_redirect_resolution<D: SessionDriver>(
    session: &Session<D>,
    surface: &X11Surface,
    geometry: Rectangle<i32, Logical>,
) -> OverrideRedirectResolution {
    let owner = resolve_override_redirect_owner(session, surface, geometry);
    let output = owner
        .as_ref()
        .and_then(|owner| crate::wayland::window_output_name(&owner.window))
        .and_then(|name| output_named(&session.wayland, &name))
        .or_else(|| output_for_geometry(session, geometry));
    OverrideRedirectResolution { owner, output }
}

pub(super) fn apply_override_redirect_resolution(
    window: &Window,
    resolution: &OverrideRedirectResolution,
) {
    if let Some(owner) = resolution.owner.as_ref() {
        if !crate::wayland::inherit_window_output(window, &owner.window)
            && let Some(output) = resolution.output.as_ref()
        {
            crate::wayland::set_window_output(window, output);
        }
        if let Some(surface) = owner.window.x11_surface() {
            crate::wayland::set_window_presentation_owner(window, Some(surface.window_id()));
        }
    } else {
        crate::wayland::set_window_presentation_owner(window, None);
        if let Some(output) = resolution.output.as_ref() {
            crate::wayland::set_window_output(window, output);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OverrideRedirectStackAction {
    Bottom,
    Above(u32),
    PreserveMissing(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OverrideRedirectMapAdmission {
    ImmediateFresh,
    ImmediateConfigured,
    AwaitConfigure,
}

pub(super) fn override_redirect_map_admission(
    first_map: bool,
    has_authoritative_placement: bool,
) -> OverrideRedirectMapAdmission {
    if has_authoritative_placement {
        OverrideRedirectMapAdmission::ImmediateConfigured
    } else if first_map {
        OverrideRedirectMapAdmission::ImmediateFresh
    } else {
        OverrideRedirectMapAdmission::AwaitConfigure
    }
}

pub(super) fn override_redirect_stack_action(
    above_sibling: Option<u32>,
    sibling_is_mapped: bool,
) -> OverrideRedirectStackAction {
    match above_sibling {
        None => OverrideRedirectStackAction::Bottom,
        Some(sibling) if sibling_is_mapped => OverrideRedirectStackAction::Above(sibling),
        Some(sibling) => OverrideRedirectStackAction::PreserveMissing(sibling),
    }
}

pub(super) fn restack_override_redirect<D: SessionDriver>(
    session: &mut Session<D>,
    window: &Window,
    above_sibling: Option<u32>,
) -> OverrideRedirectStackAction {
    // ConfigureNotify reports the X server's completed stack operation. This
    // only mirrors that order in the compositor scene; it never sends a WM
    // restack request back to an override-redirect client.
    let reference = above_sibling.and_then(|sibling| {
        session
            .wayland
            .space
            .elements()
            .find(|candidate| {
                candidate
                    .x11_surface()
                    .is_some_and(|surface| surface.window_id() == sibling)
            })
            .cloned()
    });
    let action = override_redirect_stack_action(above_sibling, reference.is_some());
    match (&action, reference) {
        (OverrideRedirectStackAction::Bottom, _) => {
            // X11 defines a missing above_sibling as the bottom of the sibling
            // stack, not the top.
            session.wayland.space.lower_element(window);
        }
        (OverrideRedirectStackAction::Above(_), Some(reference)) => {
            // The configured window is immediately on top of above_sibling.
            // The sibling may be a managed owner or another popup.
            session
                .wayland
                .space
                .raise_element_above(window, &reference, false);
        }
        (OverrideRedirectStackAction::PreserveMissing(_), _) => {
            // Smithay may report an X sibling which has no compositor element
            // (or was destroyed earlier in this dispatch). Relocation already
            // preserves the last known order, which is safer than hiding the
            // popup or spuriously raising it.
        }
        (OverrideRedirectStackAction::Above(_), None) => {
            unreachable!("mapped sibling action requires a reference")
        }
    }
    action
}

pub(super) fn override_redirect_output_name(window: &Window) -> Option<String> {
    crate::wayland::window_output_name(window)
}

pub(super) fn override_redirect_source_location<D: SessionDriver>(
    session: &Session<D>,
    window: &Window,
    resolution: &OverrideRedirectResolution,
    root_screen: Point<i32, Logical>,
) -> Point<i32, Logical> {
    let now = crate::frame_clock::monotonic_now();
    let source_root = resolution
        .owner
        .as_ref()
        .and_then(|owner| {
            super::presentation::source_point_from_owner_x_root(
                session,
                &owner.window,
                root_screen,
                now,
            )
        })
        .unwrap_or(root_screen);
    source_root + window.geometry().loc
}

pub(super) fn describe_override_redirect_map(
    surface: &X11Surface,
    geometry: Rectangle<i32, Logical>,
    output: Option<&str>,
    resolution: &OverrideRedirectResolution,
) {
    let owner = resolution
        .owner
        .as_ref()
        .and_then(|owner| owner.window.x11_surface())
        .map(|owner| owner.window_id());
    let owner_source = resolution.owner.as_ref().map(|owner| owner.source);
    eventline::debug!(
        "xwayland: mapped override-redirect xid={} owner={owner:?} owner_source={owner_source:?} output={output:?} geometry={geometry:?}",
        surface.window_id(),
    );
}

pub(super) fn describe_override_redirect_configure(
    surface: &X11Surface,
    geometry: Rectangle<i32, Logical>,
    output: Option<&str>,
    resolution: &OverrideRedirectResolution,
    above_sibling: Option<u32>,
    action: OverrideRedirectStackAction,
) {
    let owner = resolution
        .owner
        .as_ref()
        .and_then(|owner| owner.window.x11_surface())
        .map(|owner| owner.window_id());
    let owner_source = resolution.owner.as_ref().map(|owner| owner.source);
    eventline::debug!(
        "xwayland: configured override-redirect xid={} owner={owner:?} owner_source={owner_source:?} output={output:?} geometry={geometry:?} above_sibling={above_sibling:?} stack={action:?}",
        surface.window_id(),
    );
}

pub(super) fn map_override_redirect<D: SessionDriver>(
    session: &mut Session<D>,
    surface: X11Surface,
    geometry: Rectangle<i32, Logical>,
    above: Option<u32>,
    mirror_configure_stack: bool,
) {
    if window_for_surface(&session.wayland, &session.nodes, &surface).is_some() {
        return;
    }
    let resolution = override_redirect_resolution(session, &surface, geometry);
    let window = Window::new_x11_window(surface.clone());
    apply_override_redirect_resolution(&window, &resolution);
    let source_location =
        override_redirect_source_location(session, &window, &resolution, geometry.loc);
    // ICCCM override-redirect windows are client-managed. Mapping one must
    // not activate it or transfer WM focus away from its managed owner.
    session
        .wayland
        .space
        .map_element(window.clone(), source_location, false);
    if mirror_configure_stack {
        restack_override_redirect(session, &window, above);
    }
    let output = override_redirect_output_name(&window);
    describe_override_redirect_map(&surface, geometry, output.as_deref(), &resolution);
    crate::session::pointer::refresh_desktop_client_focus(
        session,
        session.start_time.elapsed().as_millis() as u32,
    );
    session.request_redraw();
}

pub(super) fn cancel_pending_override_redirect<D: SessionDriver>(
    session: &mut Session<D>,
    xid: u32,
) -> Option<PendingOverrideRedirect> {
    let pending = session.xwayland.pending_override_redirects.remove(&xid)?;
    session.xwayland.loop_handle.remove(pending.timer);
    Some(pending)
}

pub(super) fn defer_override_redirect_remap<D: SessionDriver>(
    session: &mut Session<D>,
    surface: X11Surface,
    geometry: Rectangle<i32, Logical>,
) {
    const REMAP_GRACE: Duration = Duration::from_millis(8);

    let xid = surface.window_id();
    let loop_handle = session.xwayland.loop_handle.clone();
    let timer = loop_handle.insert_source(Timer::from_duration(REMAP_GRACE), move |_, _, session| {
        if let Some(pending) = session.xwayland.pending_override_redirects.remove(&xid) {
            eventline::debug!(
                "xwayland: override-redirect xid={xid} remapped without a new configure; using retained geometry"
            );
            map_override_redirect(session, pending.surface, pending.geometry, None, false);
        }
        TimeoutAction::Drop
    });
    match timer {
        Ok(timer) => {
            session.xwayland.pending_override_redirects.insert(
                xid,
                PendingOverrideRedirect {
                    surface,
                    geometry,
                    timer,
                },
            );
            eventline::debug!(
                "xwayland: deferring reused override-redirect xid={xid} for authoritative placement"
            );
        }
        Err(err) => {
            eventline::warn!("xwayland: failed to defer override-redirect xid={xid} remap: {err}");
            map_override_redirect(session, surface, geometry, None, false);
        }
    }
}

pub(super) fn refresh_override_redirect_owners<D: SessionDriver>(session: &mut Session<D>) {
    let windows = session
        .wayland
        .space
        .elements()
        .filter_map(|window| {
            let surface = window.x11_surface()?.clone();
            surface.is_override_redirect().then(|| {
                let geometry = surface.geometry();
                (window.clone(), surface, geometry)
            })
        })
        .collect::<Vec<_>>();
    let mut changed = false;
    for (window, surface, geometry) in windows {
        let previous_owner = crate::wayland::window_presentation_owner(&window);
        let previous_output = crate::wayland::window_output_name(&window);
        let resolution = override_redirect_resolution(session, &surface, geometry);
        apply_override_redirect_resolution(&window, &resolution);
        let source_location =
            override_redirect_source_location(session, &window, &resolution, geometry.loc);
        let previous_location = session.wayland.space.element_location(&window);
        if previous_location != Some(source_location) {
            session
                .wayland
                .space
                .relocate_element(&window, source_location);
        }
        let owner = crate::wayland::window_presentation_owner(&window);
        let output = crate::wayland::window_output_name(&window);
        if previous_owner != owner
            || previous_output != output
            || previous_location != Some(source_location)
        {
            eventline::debug!(
                "xwayland: refreshed override-redirect xid={} owner={previous_owner:?}->{owner:?} output={previous_output:?}->{output:?} source={previous_location:?}->{source_location:?}",
                surface.window_id()
            );
            changed = true;
        }
    }
    if changed {
        session.request_redraw();
    }
}
