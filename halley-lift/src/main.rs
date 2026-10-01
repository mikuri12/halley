#![allow(
    clippy::result_large_err,
    clippy::too_many_arguments,
    clippy::type_complexity
)]

mod config;
mod mode;
mod model;
mod providers;
mod ui;

use std::{
    fs, io,
    os::unix::{fs::PermissionsExt, net::UnixListener, net::UnixStream},
    path::PathBuf,
    time::{Duration, Instant},
};

use calloop::channel::{Event as ChannelEvent, channel};
use calloop::{EventLoop, Interest, LoopHandle, Mode, PostAction, generic::Generic};
use calloop_wayland_source::WaylandSource;
use config::{LiftConfig, bootstrap_default_config, default_config_path};
use mode::{LiftMode, ModeInputState, effective_mode_query, parse_initial_mode};
use model::{ClusterDraft, LiftAction, LiftResult, LiftResultKind};
use providers::{ProviderIndex, SearchContext, activate_result, materialize_cluster_draft};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers},
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
    },
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use ui::{
    FontRenderer, IconCache, View, contains, draw_palette, panel_height, panel_rect,
    result_index_at, surface_height,
};
use wayland_client::{
    Connection, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
};

const NAMESPACE: &str = "halley-lift";

fn main() {
    if let Err(err) = run() {
        eprintln!("halley-lift: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    // Bootstrap the user config before any instance coordination so a second
    // invocation that merely toggles an existing instance closed still leaves
    // a documented config on disk on first run.
    bootstrap_default_config();

    let Some((_single_instance, instance_listener)) = acquire_single_instance()? else {
        return Ok(());
    };

    let start = Instant::now();
    let config_path = default_config_path();
    let config = LiftConfig::load(config_path.as_path())?;
    perf_elapsed("config load", start);
    let initial_raw = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    let (initial_mode, initial_query) = parse_initial_mode(initial_raw.as_str());

    let font_family = config.ui.font.clone();
    let font_job = std::thread::Builder::new()
        .name("halley-lift-font".into())
        .spawn(move || FontRenderer::new(&font_family))
        .map_err(|err| format!("font worker: {err}"))?;
    let index_config = config.clone();
    let index_job = std::thread::Builder::new()
        .name("halley-lift-providers".into())
        .spawn(move || ProviderIndex::load(&index_config))
        .map_err(|err| format!("provider worker: {err}"))?;

    let start = Instant::now();
    let conn = Connection::connect_to_env().map_err(|err| format!("wayland connect: {err}"))?;
    let (globals, event_queue) =
        registry_queue_init(&conn).map_err(|err| format!("registry init: {err}"))?;
    let qh = event_queue.handle();

    let compositor =
        CompositorState::bind(&globals, &qh).map_err(|err| format!("bind compositor: {err}"))?;
    let layer_shell =
        LayerShell::bind(&globals, &qh).map_err(|err| format!("bind layer shell: {err}"))?;
    let shm = Shm::bind(&globals, &qh).map_err(|err| format!("bind shm: {err}"))?;
    perf_elapsed("wayland init", start);

    let font = font_job
        .join()
        .map_err(|_| "font worker panicked".to_string())??;
    let mut index = index_job
        .join()
        .map_err(|_| "provider worker panicked".to_string())?;

    let surface = compositor.create_surface(&qh);
    let layer =
        layer_shell.create_layer_surface(&qh, surface, Layer::Overlay, Some(NAMESPACE), None);
    let width = config.width.max(420);
    let height = panel_height(&config) as u32;
    let (anchor, margins) = layer_position(&config);
    layer.set_anchor(anchor);
    // A launcher is a modal grab: while it is open, keyboard input must go to it
    // and nowhere else, so we always request exclusive interactivity. The
    // compositor grabs the keyboard and deactivates toplevels in response (see
    // apply_layer_surface_focus in halley-wl).
    layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
    layer.set_size(width, height);
    layer.set_margin(margins.0, margins.1, margins.2, margins.3);
    layer.commit();
    let pool = SlotPool::new((width * height * 4) as usize, &shm)
        .map_err(|err| format!("slot pool: {err}"))?;

    let start = Instant::now();
    let mut icon_cache = IconCache::new(&config);
    perf_elapsed("icon cache init", start);

    let mut event_loop: EventLoop<'static, LiftApp> =
        EventLoop::try_new().map_err(|err| format!("event loop: {err}"))?;
    let loop_handle = event_loop.handle();
    let (icon_wake_tx, icon_wake_rx) = channel();
    icon_cache.set_waker(icon_wake_tx);
    let (live_wake_tx, live_wake_rx) = channel();
    index.set_live_waker(live_wake_tx);
    WaylandSource::new(conn.clone(), event_queue)
        .insert(loop_handle.clone())
        .map_err(|err| format!("wayland source: {err}"))?;
    loop_handle
        .insert_source(icon_wake_rx, |event, _, app: &mut LiftApp| {
            if matches!(event, ChannelEvent::Msg(())) && app.icon_cache.poll_decodes() {
                app.mark_redraw();
            }
        })
        .map_err(|err| format!("icon result source: {err}"))?;
    loop_handle
        .insert_source(live_wake_rx, |event, _, app: &mut LiftApp| {
            if matches!(event, ChannelEvent::Msg(())) {
                app.poll_live_refresh();
            }
        })
        .map_err(|err| format!("API event source: {err}"))?;

    // A second invocation connects to our single-instance socket; treat that as a request
    // to dismiss this instance (toggle), so launching halley-lift again from anywhere
    // closes the open launcher instead of spawning a short-lived duplicate.
    loop_handle
        .insert_source(
            Generic::new(instance_listener, Interest::READ, Mode::Level),
            |_readiness, listener, app: &mut LiftApp| {
                loop {
                    match listener.accept() {
                        Ok(_) => app.exit = true,
                        Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                        Err(_) => break,
                    }
                }
                Ok(PostAction::Continue)
            },
        )
        .map_err(|err| format!("instance socket source: {err}"))?;

    let mut app = LiftApp {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        _compositor: compositor,
        _layer_shell: layer_shell,
        _shm: shm,
        pool,
        layer,
        qh: qh.clone(),
        loop_handle: loop_handle.clone(),
        keyboard: None,
        pointer: None,
        keyboard_focused: false,
        had_keyboard_focus: false,
        configured: false,
        prefetched_live: false,
        needs_redraw: false,
        frame_pending: false,
        cursor_visible: true,
        cursor_last_blink: Instant::now(),
        cursor_last_activity: Instant::now(),
        width,
        height,
        exit: false,
        config,
        font,
        index,
        icon_cache,
        input: ModeInputState {
            mode: initial_mode,
            query: initial_query,
        },
        results: Vec::new(),
        selected: 0,
        scroll_offset: 0,
        selection_authority: SelectionAuthority::Keyboard,
        draft: ClusterDraft::default(),
        modifiers: Modifiers::default(),
        status: None,
    };
    if app.input.mode != LiftMode::General || !app.input.query.trim().is_empty() {
        app.refresh_results();
    } else {
        perf(format_args!("skip hidden empty-startup search"));
    }

    while !app.exit {
        let timeout = app.dispatch_timeout();
        if let Err(err) = event_loop.dispatch(timeout, &mut app) {
            app.debug(format_args!("event dispatch error: {err}"));
            return Err(format!("event dispatch: {err}"));
        }
        app.poll_cursor_blink();
        app.flush_redraw();
    }
    Ok(())
}

struct SingleInstanceGuard {
    socket_path: PathBuf,
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.socket_path);
    }
}

/// Acquires single-instance ownership. On success returns the guard (which unlinks the
/// socket on drop) plus the bound, nonblocking listener — the caller wires it into the
/// event loop so a second invocation toggles this instance closed. Returns `Ok(None)` when
/// another instance is already running and answering.
fn acquire_single_instance() -> Result<Option<(SingleInstanceGuard, UnixListener)>, String> {
    let socket_path = lift_socket_path()?;
    for _ in 0..2 {
        match UnixListener::bind(&socket_path) {
            Ok(listener) => {
                listener
                    .set_nonblocking(true)
                    .map_err(|err| format!("set instance socket nonblocking: {err}"))?;
                return Ok(Some((SingleInstanceGuard { socket_path }, listener)));
            }
            Err(err) if err.kind() == io::ErrorKind::AddrInUse => {
                if UnixStream::connect(&socket_path).is_ok() {
                    return Ok(None);
                }
                match fs::remove_file(&socket_path) {
                    Ok(()) => continue,
                    Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                    Err(err) => {
                        return Err(format!(
                            "remove stale instance socket {}: {err}",
                            socket_path.display()
                        ));
                    }
                }
            }
            Err(err) => {
                return Err(format!(
                    "bind instance socket {}: {err}",
                    socket_path.display()
                ));
            }
        }
    }

    Err(format!(
        "bind instance socket {} after stale cleanup",
        socket_path.display()
    ))
}

fn lift_socket_path() -> Result<PathBuf, String> {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .ok_or_else(|| "XDG_RUNTIME_DIR is not set".to_string())?;
    let dir = PathBuf::from(runtime_dir).join("halley");
    fs::create_dir_all(&dir).map_err(|err| format!("create {}: {err}", dir.display()))?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
        .map_err(|err| format!("chmod {}: {err}", dir.display()))?;
    Ok(dir.join("halley-lift.sock"))
}

fn perf(args: std::fmt::Arguments<'_>) {
    if std::env::var_os("HALLEY_LIFT_PERF").is_some() {
        eprintln!("halley-lift perf: {args}");
    }
}

fn perf_elapsed(label: &str, start: Instant) {
    perf(format_args!("{label}: {:.2?}", start.elapsed()));
}

fn layer_position(config: &LiftConfig) -> (Anchor, (i32, i32, i32, i32)) {
    let pad = config.ui.padding;
    let top = config.ui.top_margin + config.position.offset_y;
    let bottom = config.ui.top_margin - config.position.offset_y;
    let left = pad + config.position.offset_x;
    let right = pad - config.position.offset_x;
    match config.position.anchor.to_ascii_lowercase().as_str() {
        "top" => (
            Anchor::TOP,
            (top, -config.position.offset_x, 0, config.position.offset_x),
        ),
        "top-left" => (Anchor::TOP | Anchor::LEFT, (top, 0, 0, left)),
        "top-right" => (Anchor::TOP | Anchor::RIGHT, (top, right, 0, 0)),
        "bottom" => (
            Anchor::BOTTOM,
            (
                0,
                -config.position.offset_x,
                bottom,
                config.position.offset_x,
            ),
        ),
        "bottom-left" => (Anchor::BOTTOM | Anchor::LEFT, (0, 0, bottom, left)),
        "bottom-right" => (Anchor::BOTTOM | Anchor::RIGHT, (0, right, bottom, 0)),
        // Default ("center"): horizontally centered, but pin the top edge so the
        // search bar stays fixed and results grow downward (Spotlight/Flow style)
        // rather than re-centering the whole surface as it grows.
        _ => (
            Anchor::TOP,
            (top, -config.position.offset_x, 0, config.position.offset_x),
        ),
    }
}

fn sane_dimension(configured: u32, fallback: u32, max: u32) -> u32 {
    if configured == 0 {
        fallback.clamp(1, max)
    } else {
        configured.clamp(1, max)
    }
}

struct LiftApp {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    _compositor: CompositorState,
    _layer_shell: LayerShell,
    _shm: Shm,
    pool: SlotPool,
    layer: LayerSurface,
    qh: QueueHandle<LiftApp>,
    loop_handle: LoopHandle<'static, LiftApp>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    keyboard_focused: bool,
    had_keyboard_focus: bool,
    configured: bool,
    prefetched_live: bool,
    needs_redraw: bool,
    frame_pending: bool,
    cursor_visible: bool,
    cursor_last_blink: Instant,
    cursor_last_activity: Instant,
    width: u32,
    height: u32,
    exit: bool,
    config: LiftConfig,
    font: FontRenderer,
    index: ProviderIndex,
    icon_cache: IconCache,
    input: ModeInputState,
    results: Vec<LiftResult>,
    selected: usize,
    scroll_offset: usize,
    selection_authority: SelectionAuthority,
    draft: ClusterDraft,
    modifiers: Modifiers,
    status: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SelectionAuthority {
    #[default]
    Keyboard,
    Pointer,
}

impl SelectionAuthority {
    fn keyboard_activity(&mut self) {
        *self = Self::Keyboard;
    }

    fn pointer_activity(&mut self) {
        *self = Self::Pointer;
    }

    fn enter_can_hover(self) -> bool {
        self == Self::Pointer
    }
}

fn wheel_direction(value120: i32, discrete: i32, absolute: f64) -> isize {
    if value120 != 0 {
        value120.signum() as isize
    } else if discrete != 0 {
        discrete.signum() as isize
    } else if absolute > 0.0 {
        1
    } else if absolute < 0.0 {
        -1
    } else {
        0
    }
}

fn keep_selection_visible(
    selected: usize,
    scroll_offset: usize,
    result_count: usize,
    visible_results: usize,
) -> usize {
    if result_count == 0 {
        return 0;
    }

    let selected = selected.min(result_count - 1);
    let visible = visible_results.max(1);
    let max_offset = result_count.saturating_sub(visible);
    let mut offset = scroll_offset.min(max_offset);
    if selected < offset {
        offset = selected;
    } else if selected >= offset.saturating_add(visible) {
        offset = selected + 1 - visible;
    }
    offset.min(max_offset)
}

/// Mouse-wheel navigation moves the highlight by one row. The viewport stays
/// fixed while that row remains visible, then follows the highlight at either
/// visible edge. Unlike keyboard navigation, wheel movement never wraps.
fn wheel_position(
    selected: usize,
    scroll_offset: usize,
    result_count: usize,
    visible_results: usize,
    direction: isize,
) -> (usize, usize) {
    if result_count == 0 {
        return (0, 0);
    }

    let selected = selected.min(result_count - 1);
    let next = if direction < 0 {
        selected.saturating_sub(1)
    } else if direction > 0 {
        selected.saturating_add(1).min(result_count - 1)
    } else {
        selected
    };
    let offset = keep_selection_visible(next, scroll_offset, result_count, visible_results);
    (next, offset)
}

impl LiftApp {
    fn refresh_results(&mut self) {
        let start = Instant::now();
        let (mode, query) = self.effective_search();
        self.ensure_live_snapshot();
        let ctx = SearchContext {
            mode,
            query: query.clone(),
            query_lower: query.trim().to_ascii_lowercase(),
            max_results: self.config.max_results,
            draft_count: self.draft.count(),
        };
        self.results = self.index.search(&ctx);
        if self.selected >= self.results.len() {
            self.selected = self.results.len().saturating_sub(1);
        }
        self.scroll_offset = keep_selection_visible(
            self.selected,
            self.scroll_offset,
            self.results.len(),
            self.config.visible_results,
        );
        perf(format_args!(
            "search mode={:?} query_len={} results={} elapsed={:.2?}",
            mode,
            query.len(),
            self.results.len(),
            start.elapsed()
        ));
    }

    /// Refresh results after the user changed the query by typing. The highlight
    /// snaps back to the first entry; a stationary cursor emits no motion event, so
    /// it only moves again once the mouse is physically dragged over another row.
    fn refresh_results_typed(&mut self) {
        self.refresh_results();
        self.selected = 0;
        self.scroll_offset = 0;
        self.selection_authority.keyboard_activity();
    }

    fn effective_search(&self) -> (LiftMode, String) {
        effective_mode_query(self.input.mode, self.input.query.as_str())
    }

    fn ensure_live_snapshot(&mut self) {
        if !self.live_results_needed() || !self.index.needs_live_refresh() {
            return;
        }
        let start = Instant::now();
        self.index.start_live_refresh();
        perf(format_args!(
            "live ipc refresh started elapsed={:.2?}",
            start.elapsed()
        ));
    }

    fn live_results_needed(&self) -> bool {
        let (mode, _) = self.effective_search();
        matches!(mode, LiftMode::Nodes | LiftMode::Clusters)
    }

    fn draw(&mut self) -> Result<(), String> {
        if !self.configured {
            return Ok(());
        }
        let start = Instant::now();
        let stride = self.width as i32 * 4;
        self.debug(format_args!(
            "draw size={}x{} stride={}",
            self.width, self.height, stride
        ));
        let scroll_offset = self.scroll_offset;
        let (mode, _) = self.effective_search();
        let (buffer, canvas) = self
            .pool
            .create_buffer(
                self.width as i32,
                self.height as i32,
                stride,
                wl_shm::Format::Argb8888,
            )
            .map_err(|err| format!("create buffer: {err}"))?;
        draw_palette(
            canvas,
            self.width,
            self.height,
            &self.font,
            &mut self.icon_cache,
            View {
                config: &self.config,
                input: &self.input,
                mode,
                results: &self.results,
                selected: self.selected,
                scroll_offset,
                draft: &self.draft,
                status: self.status.as_deref(),
                cursor_visible: self.cursor_visible,
            },
        );
        self.layer
            .wl_surface()
            .damage_buffer(0, 0, self.width as i32, self.height as i32);
        buffer
            .attach_to(self.layer.wl_surface())
            .map_err(|err| format!("attach buffer: {err}"))?;
        self.layer
            .wl_surface()
            .frame(&self.qh, self.layer.wl_surface().clone());
        self.frame_pending = true;
        self.layer.commit();
        perf_elapsed("draw", start);
        Ok(())
    }

    fn redraw(&mut self) {
        if let Err(err) = self.draw() {
            self.status = Some(err);
        }
    }

    fn mark_redraw(&mut self) {
        self.needs_redraw = true;
    }

    fn flush_redraw(&mut self) {
        if self.exit || !self.configured || !self.needs_redraw || self.frame_pending {
            return;
        }
        let desired_height = self.desired_surface_height();
        let desired_width = self.config.width.max(420);
        if desired_width != self.width || desired_height != self.height {
            self.debug(format_args!(
                "request resize {}x{} -> {}x{}",
                self.width, self.height, desired_width, desired_height
            ));
            self.layer.set_size(desired_width, desired_height);
            self.layer.commit();
            return;
        }
        self.needs_redraw = false;
        self.redraw();
        self.prefetch_live_after_first_draw();
    }

    fn prefetch_live_after_first_draw(&mut self) {
        if self.prefetched_live || !self.index.needs_live_refresh() {
            return;
        }
        self.prefetched_live = true;
        let start = Instant::now();
        self.index.start_live_refresh();
        perf(format_args!(
            "live ipc prefetch started elapsed={:.2?}",
            start.elapsed()
        ));
    }

    fn poll_live_refresh(&mut self) {
        if let Some((nodes, clusters)) = self.index.finish_live_refresh_if_ready() {
            perf(format_args!(
                "live ipc prefetch ready nodes={} clusters={}",
                nodes, clusters
            ));
            let (mode, query) = self.effective_search();
            if matches!(mode, LiftMode::Nodes | LiftMode::Clusters)
                || (mode == LiftMode::General && !query.trim().is_empty())
            {
                self.refresh_results();
                self.mark_redraw();
            }
        }
    }

    fn desired_surface_height(&self) -> u32 {
        surface_height(self.current_view()).max(1) as u32
    }

    fn move_selection(&mut self, delta: isize) {
        if self.results.is_empty() {
            return;
        }
        let len = self.results.len() as isize;
        self.selected = ((self.selected as isize + delta).rem_euclid(len)) as usize;
        self.keep_selection_visible();
    }

    fn set_selection(&mut self, index: usize) {
        if index < self.results.len() {
            self.selected = index;
            self.keep_selection_visible();
        }
    }

    fn move_page(&mut self, delta: isize) {
        self.move_selection(delta * self.config.visible_results.max(1) as isize);
    }

    fn jump_to_edge(&mut self, end: bool) {
        if self.results.is_empty() {
            return;
        }
        self.selected = if end { self.results.len() - 1 } else { 0 };
        self.keep_selection_visible();
    }

    fn keep_selection_visible(&mut self) {
        self.scroll_offset = keep_selection_visible(
            self.selected,
            self.scroll_offset,
            self.results.len(),
            self.config.visible_results,
        );
    }

    fn scroll_selection(&mut self, direction: isize) {
        (self.selected, self.scroll_offset) = wheel_position(
            self.selected,
            self.scroll_offset,
            self.results.len(),
            self.config.visible_results,
            direction,
        );
    }

    fn selected_result(&self) -> Option<&LiftResult> {
        self.results.get(self.selected)
    }

    fn current_view(&self) -> View<'_> {
        View {
            config: &self.config,
            input: &self.input,
            mode: self.effective_search().0,
            results: &self.results,
            selected: self.selected,
            scroll_offset: self.scroll_offset,
            draft: &self.draft,
            status: self.status.as_deref(),
            cursor_visible: self.cursor_visible,
        }
    }

    fn toggle_selected(&mut self) {
        let Some(result) = self.selected_result().cloned() else {
            return;
        };
        if matches!(result.kind, LiftResultKind::App | LiftResultKind::Node) {
            self.draft.toggle_result(&result);
            self.status = None;
            self.refresh_results();
        }
    }

    fn activate_selected(&mut self) {
        let Some(result) = self.selected_result().cloned() else {
            return;
        };
        if matches!(result.action, LiftAction::CreateCluster) {
            self.materialize_draft();
            return;
        }
        let (mode, _) = self.effective_search();
        if mode == LiftMode::Clusters
            && matches!(result.kind, LiftResultKind::App | LiftResultKind::Node)
        {
            self.toggle_selected();
            return;
        }
        match activate_result(&self.index, &result) {
            Ok(()) => self.exit("activate"),
            Err(err) => self.status = Some(err),
        }
    }

    fn materialize_draft(&mut self) {
        let (mode, query) = self.effective_search();
        if mode != LiftMode::Clusters {
            self.status = Some("Use cluster search before finalizing a draft".into());
            return;
        }
        if self.draft.count() == 0 {
            self.status = Some("Select apps or nodes with Space before finalizing".into());
            return;
        }
        match materialize_cluster_draft(&self.index, &self.draft, query.as_str()) {
            Ok(()) => self.exit("cluster-draft"),
            Err(err) => self.status = Some(err),
        }
    }

    fn exit(&mut self, reason: &str) {
        self.debug(format_args!("exit: {reason}"));
        self.exit = true;
    }

    fn debug(&self, args: std::fmt::Arguments<'_>) {
        if std::env::var_os("HALLEY_LIFT_DEBUG").is_some() {
            eprintln!("halley-lift: {args}");
        }
    }

    fn dispatch_timeout(&self) -> Option<Duration> {
        self.cursor_poll_interval()
    }

    fn cursor_poll_interval(&self) -> Option<Duration> {
        if !self.config.cursor.enabled || self.cursor_blink_stopped() {
            return None;
        }
        let blink_remaining = self
            .cursor_blink_interval()
            .saturating_sub(self.cursor_last_blink.elapsed());
        let Some(stop_remaining) = self.cursor_stop_remaining() else {
            return Some(blink_remaining);
        };
        Some(blink_remaining.min(stop_remaining))
    }

    fn cursor_blink_interval(&self) -> Duration {
        Duration::from_millis(self.config.cursor.blink_ms.max(1))
    }

    fn cursor_stop_interval(&self) -> Option<Duration> {
        (self.config.cursor.stop_blink_after_ms > 0)
            .then(|| Duration::from_millis(self.config.cursor.stop_blink_after_ms))
    }

    fn cursor_stop_remaining(&self) -> Option<Duration> {
        self.cursor_stop_interval()
            .map(|interval| interval.saturating_sub(self.cursor_last_activity.elapsed()))
    }

    fn cursor_blink_stopped(&self) -> bool {
        self.cursor_stop_interval()
            .is_some_and(|interval| self.cursor_last_activity.elapsed() >= interval)
    }

    fn reset_cursor_blink(&mut self) {
        self.cursor_visible = true;
        self.cursor_last_blink = Instant::now();
        self.cursor_last_activity = Instant::now();
    }

    fn poll_cursor_blink(&mut self) {
        if !self.config.cursor.enabled {
            if self.cursor_visible {
                self.cursor_visible = false;
                self.mark_redraw();
            }
            return;
        }
        if self.cursor_blink_stopped() {
            if !self.cursor_visible {
                self.cursor_visible = true;
                self.mark_redraw();
            }
            return;
        }
        if self.cursor_last_blink.elapsed() >= self.cursor_blink_interval() {
            self.cursor_visible = !self.cursor_visible;
            self.cursor_last_blink = Instant::now();
            self.mark_redraw();
        }
    }

    fn handle_text(&mut self, text: &str) {
        let (mode, query) = self.effective_search();
        if text == " "
            && mode == LiftMode::Clusters
            && !query.trim().is_empty()
            && self.selected_is_stageable()
        {
            self.toggle_selected();
            return;
        }
        self.input.insert_text(text);
        self.reset_cursor_blink();
        self.refresh_results_typed();
    }

    fn selected_is_stageable(&self) -> bool {
        self.selected_result()
            .is_some_and(|result| matches!(result.kind, LiftResultKind::App | LiftResultKind::Node))
    }

    fn handle_key(&mut self, event: KeyEvent) {
        self.selection_authority.keyboard_activity();
        if self.modifiers.alt
            && self.config.alt_number_jump
            && let Some(offset) = alt_number_offset(event.keysym)
        {
            let index = self.scroll_offset + offset;
            if index < self.results.len() {
                self.selected = index;
                self.activate_selected();
            }
            return;
        }
        match event.keysym {
            Keysym::Escape => self.exit("escape"),
            Keysym::Up | Keysym::Left => self.move_selection(-1),
            Keysym::Down | Keysym::Right => self.move_selection(1),
            Keysym::Page_Up => self.move_page(-1),
            Keysym::Page_Down => self.move_page(1),
            Keysym::Home => self.jump_to_edge(false),
            Keysym::End => self.jump_to_edge(true),
            Keysym::Tab => {
                if self.input.query.trim().is_empty() {
                    self.input.query = "action ".into();
                } else {
                    self.input.query = format!("action {}", self.input.query.trim_start());
                }
                self.input.mode = LiftMode::General;
                self.reset_cursor_blink();
                self.refresh_results_typed();
            }
            Keysym::BackSpace => {
                self.input.backspace();
                self.reset_cursor_blink();
                self.refresh_results_typed();
            }
            Keysym::Return | Keysym::KP_Enter => {
                if self.modifiers.ctrl {
                    self.materialize_draft();
                } else {
                    self.activate_selected();
                }
            }
            // Handle Space by keysym rather than relying on `utf8`, which some keymaps
            // leave empty for the space key (so a trailing space would not register until
            // the next character arrived).
            Keysym::space | Keysym::KP_Space => {
                if !self.modifiers.ctrl && !self.modifiers.alt {
                    self.handle_text(" ");
                }
            }
            _ => {
                if !self.modifiers.ctrl
                    && !self.modifiers.alt
                    && let Some(text) = event.utf8.as_deref()
                    && !text.chars().any(char::is_control)
                {
                    self.handle_text(text);
                }
            }
        }
    }
}

fn alt_number_offset(keysym: Keysym) -> Option<usize> {
    match keysym {
        Keysym::_1 | Keysym::KP_1 => Some(0),
        Keysym::_2 | Keysym::KP_2 => Some(1),
        Keysym::_3 | Keysym::KP_3 => Some(2),
        Keysym::_4 | Keysym::KP_4 => Some(3),
        Keysym::_5 | Keysym::KP_5 => Some(4),
        Keysym::_6 | Keysym::KP_6 => Some(5),
        Keysym::_7 | Keysym::KP_7 => Some(6),
        Keysym::_8 | Keysym::KP_8 => Some(7),
        Keysym::_9 | Keysym::KP_9 => Some(8),
        Keysym::_0 | Keysym::KP_0 => Some(9),
        _ => None,
    }
}

impl CompositorHandler for LiftApp {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {
        self.frame_pending = false;
    }
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for LiftApp {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for LiftApp {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.exit("layer-closed");
    }
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        self.width = sane_dimension(configure.new_size.0, self.config.width.max(420), 4096);
        self.height = sane_dimension(
            configure.new_size.1,
            panel_height(&self.config) as u32,
            2160,
        );
        self.debug(format_args!(
            "configure size={}x{} -> {}x{}",
            configure.new_size.0, configure.new_size.1, self.width, self.height
        ));
        self.configured = true;
        self.reset_cursor_blink();
        self.mark_redraw();
    }
}

impl SeatHandler for LiftApp {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            let handle = self.loop_handle.clone();
            if let Ok(keyboard) = self.seat_state.get_keyboard_with_repeat(
                qh,
                &seat,
                None,
                handle,
                Box::new(|app: &mut LiftApp, _kbd, event| {
                    app.handle_key(event);
                    app.mark_redraw();
                }),
            ) {
                self.keyboard = Some(keyboard);
            }
        }
        if capability == Capability::Pointer
            && self.pointer.is_none()
            && let Ok(pointer) = self.seat_state.get_pointer(qh, &seat)
        {
            self.pointer = Some(pointer);
        }
    }
    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard
            && let Some(keyboard) = self.keyboard.take()
        {
            keyboard.release();
        }
        if capability == Capability::Pointer
            && let Some(pointer) = self.pointer.take()
        {
            pointer.release();
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl KeyboardHandler for LiftApp {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
        if self.layer.wl_surface() == surface {
            self.keyboard_focused = true;
            self.had_keyboard_focus = true;
            self.reset_cursor_blink();
        }
    }
    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        if self.layer.wl_surface() == surface {
            self.keyboard_focused = false;
            if self.config.close_on_focus_loss && self.had_keyboard_focus {
                self.exit("focus-loss");
            }
        }
    }
    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        self.handle_key(event);
        self.mark_redraw();
    }
    fn repeat_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        self.handle_key(event);
        self.mark_redraw();
    }
    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: KeyEvent,
    ) {
    }
    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        modifiers: Modifiers,
        _: RawModifiers,
        _: u32,
    ) {
        self.modifiers = modifiers;
    }
}

impl PointerHandler for LiftApp {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            if &event.surface != self.layer.wl_surface() {
                continue;
            }
            match event.kind {
                PointerEventKind::Motion { .. } => {
                    self.selection_authority.pointer_activity();
                    if let Some(index) = result_index_at(
                        self.current_view(),
                        self.width,
                        self.height,
                        event.position.0,
                        event.position.1,
                    ) {
                        self.set_selection(index);
                    }
                }
                PointerEventKind::Enter { .. } => {
                    if self.selection_authority.enter_can_hover()
                        && let Some(index) = result_index_at(
                            self.current_view(),
                            self.width,
                            self.height,
                            event.position.0,
                            event.position.1,
                        )
                    {
                        self.set_selection(index);
                    }
                }
                PointerEventKind::Press { button, .. } => {
                    self.selection_authority.pointer_activity();
                    let panel = panel_rect(&self.config, self.width, self.height);
                    if !contains(panel, event.position.0, event.position.1) {
                        continue;
                    }
                    if button == 0x110
                        && let Some(index) = result_index_at(
                            self.current_view(),
                            self.width,
                            self.height,
                            event.position.0,
                            event.position.1,
                        )
                    {
                        self.set_selection(index);
                        self.activate_selected();
                    }
                }
                PointerEventKind::Axis { vertical, .. } => {
                    self.selection_authority.pointer_activity();
                    let direction =
                        wheel_direction(vertical.value120, vertical.discrete, vertical.absolute);
                    if direction != 0 {
                        self.scroll_selection(direction);
                    }
                }
                PointerEventKind::Leave { .. } | PointerEventKind::Release { .. } => {}
            }
        }
        self.mark_redraw();
    }
}

impl ShmHandler for LiftApp {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self._shm
    }
}

delegate_compositor!(LiftApp);
delegate_output!(LiftApp);
delegate_shm!(LiftApp);
delegate_layer!(LiftApp);
delegate_seat!(LiftApp);
delegate_keyboard!(LiftApp);
delegate_pointer!(LiftApp);
delegate_registry!(LiftApp);

impl ProvidesRegistryState for LiftApp {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers!(OutputState, SeatState);
}

#[cfg(test)]
mod selection_tests {
    use super::{SelectionAuthority, keep_selection_visible, wheel_direction, wheel_position};

    #[test]
    fn positive_wayland_axis_moves_down_and_negative_moves_up() {
        assert_eq!(wheel_direction(120, 0, 0.0), 1);
        assert_eq!(wheel_direction(-120, 0, 0.0), -1);
        assert_eq!(wheel_direction(0, 1, 0.0), 1);
        assert_eq!(wheel_direction(0, -1, 0.0), -1);
        assert_eq!(wheel_direction(0, 0, 8.0), 1);
        assert_eq!(wheel_direction(0, 0, -8.0), -1);
    }

    #[test]
    fn high_resolution_axis_direction_takes_precedence() {
        assert_eq!(wheel_direction(120, -1, -8.0), 1);
        assert_eq!(wheel_direction(-120, 1, 8.0), -1);
    }

    #[test]
    fn wheel_down_moves_one_row_then_scrolls_at_bottom() {
        assert_eq!(wheel_position(1, 0, 10, 4, 1), (2, 0));
        assert_eq!(wheel_position(2, 0, 10, 4, 1), (3, 0));
        assert_eq!(wheel_position(3, 0, 10, 4, 1), (4, 1));
    }

    #[test]
    fn wheel_up_moves_one_row_then_scrolls_at_top() {
        assert_eq!(wheel_position(5, 3, 10, 4, -1), (4, 3));
        assert_eq!(wheel_position(4, 3, 10, 4, -1), (3, 3));
        assert_eq!(wheel_position(3, 3, 10, 4, -1), (2, 2));
    }

    #[test]
    fn wheel_stops_at_list_ends_instead_of_wrapping() {
        assert_eq!(wheel_position(0, 0, 10, 4, -1), (0, 0));
        assert_eq!(wheel_position(9, 6, 10, 4, 1), (9, 6));
    }

    #[test]
    fn keyboard_visibility_uses_existing_viewport() {
        assert_eq!(keep_selection_visible(4, 3, 10, 4), 3);
        assert_eq!(keep_selection_visible(2, 3, 10, 4), 2);
        assert_eq!(keep_selection_visible(7, 3, 10, 4), 4);
    }

    #[test]
    fn keyboard_activity_blocks_stationary_pointer_enter() {
        let mut authority = SelectionAuthority::Pointer;
        authority.keyboard_activity();
        assert!(!authority.enter_can_hover());
    }

    #[test]
    fn real_pointer_motion_returns_hover_authority() {
        let mut authority = SelectionAuthority::Keyboard;
        authority.pointer_activity();
        assert!(authority.enter_can_hover());
    }
}
