//! Real Wayland request/event regression tests for Halley's patched Smithay.
use smithay::{
    delegate_compositor, delegate_input_method_manager, delegate_seat, delegate_text_input_manager,
    input::{Seat, SeatHandler, SeatState, pointer::CursorImageStatus},
    reexports::wayland_server::{self as server, Display, protocol::wl_surface::WlSurface},
    utils::{Logical, Rectangle, SERIAL_COUNTER},
    wayland::{
        compositor::{CompositorClientState, CompositorHandler, CompositorState},
        input_method::{
            InputMethodHandler, InputMethodManagerState, InputMethodSeat, PopupSurface,
        },
        text_input::TextInputManagerState,
    },
};
use std::{
    collections::HashMap,
    os::unix::net::UnixStream,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop,
    protocol::{wl_compositor, wl_keyboard, wl_registry, wl_seat, wl_surface},
};
use wayland_protocols::wp::text_input::zv3::client::{
    zwp_text_input_manager_v3 as tim, zwp_text_input_v3 as ti,
};
use wayland_protocols_misc::zwp_input_method_v2::client::{
    zwp_input_method_keyboard_grab_v2 as ime_keyboard, zwp_input_method_manager_v2 as imm,
    zwp_input_method_v2 as im, zwp_input_popup_surface_v2 as popup,
};

use smithay::reexports::wayland_protocols::ext::session_lock::v1::server::{
    ext_session_lock_manager_v1::ExtSessionLockManagerV1,
    ext_session_lock_surface_v1::ExtSessionLockSurfaceV1,
    ext_session_lock_v1::{ExtSessionLockV1, Request as LockRequest},
};
use smithay::reexports::wayland_server::Resource;
use smithay::wayland::session_lock::{
    ExtLockSurfaceUserData, LockSurface, SessionLockHandler, SessionLockManagerGlobalData,
    SessionLockManagerState, SessionLockState, SessionLocker,
};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1 as lock_manager, ext_session_lock_surface_v1 as lock_surface,
    ext_session_lock_v1 as lock,
};

struct Server {
    popups: Arc<std::sync::Mutex<Vec<PopupSurface>>>,
    compositor: CompositorState,
    lock_state: SessionLockManagerState,
    locked: bool,
    rejected_locks: std::collections::HashSet<server::backend::ObjectId>,
    seats: SeatState<Self>,
    seat: Seat<Self>,
}
impl SessionLockHandler for Server {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.lock_state
    }
    fn lock(&mut self, locker: SessionLocker) {
        if self.locked {
            self.rejected_locks.insert(locker.ext_session_lock().id());
        } else {
            self.locked = true;
            locker.lock();
        }
    }
    fn unlock(&mut self) {
        self.locked = false;
    }
    fn new_surface(&mut self, surface: LockSurface, _: server::protocol::wl_output::WlOutput) {
        surface.with_pending_state(|state| state.size = Some((100, 100).into()));
    }
}
smithay::reexports::wayland_server::delegate_global_dispatch!(Server: [ExtSessionLockManagerV1: SessionLockManagerGlobalData] => SessionLockManagerState);
smithay::reexports::wayland_server::delegate_dispatch!(Server: [ExtSessionLockManagerV1: ()] => SessionLockManagerState);
smithay::reexports::wayland_server::delegate_dispatch!(Server: [ExtSessionLockSurfaceV1: ExtLockSurfaceUserData] => SessionLockManagerState);
impl server::Dispatch<ExtSessionLockV1, SessionLockState> for Server {
    fn request(
        state: &mut Self,
        client: &server::Client,
        lock: &ExtSessionLockV1,
        request: LockRequest,
        data: &SessionLockState,
        display: &server::DisplayHandle,
        init: &mut server::DataInit<'_, Self>,
    ) {
        if state.rejected_locks.contains(&lock.id()) {
            SessionLockManagerState::rejected_request(lock, request, init);
        } else {
            <SessionLockManagerState as server::Dispatch<
                ExtSessionLockV1,
                SessionLockState,
                Self,
            >>::request(state, client, lock, request, data, display, init);
        }
    }
}
impl smithay::wayland::output::OutputHandler for Server {}
smithay::delegate_output!(Server);

#[derive(Default)]
struct ClientData(CompositorClientState);
impl server::backend::ClientData for ClientData {
    fn initialized(&self, _: server::backend::ClientId) {}
    fn disconnected(&self, _: server::backend::ClientId, _: server::backend::DisconnectReason) {}
}
impl CompositorHandler for Server {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor
    }
    fn client_compositor_state<'a>(&self, client: &'a server::Client) -> &'a CompositorClientState {
        &client.get_data::<ClientData>().unwrap().0
    }
    fn commit(&mut self, surface: &WlSurface) {
        // Test-only focus control, exercised over the same socket as requests.
        let keyboard = self.seat.get_keyboard().unwrap();
        keyboard.set_focus(self, Some(surface.clone()), SERIAL_COUNTER.next_serial());
    }
}
impl SeatHandler for Server {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;
    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seats
    }
    fn focus_changed(&mut self, _: &Seat<Self>, _: Option<&WlSurface>) {}
    fn cursor_image(&mut self, _: &Seat<Self>, _: CursorImageStatus) {}
}
impl InputMethodHandler for Server {
    fn new_popup(&mut self, popup: PopupSurface) {
        self.popups.lock().unwrap().push(popup);
    }
    fn dismiss_popup(&mut self, popup: PopupSurface) {
        self.popups.lock().unwrap().retain(|old| old != &popup);
    }
    fn popup_repositioned(&mut self, _: PopupSurface) {}
    fn parent_geometry(&self, _: &WlSurface) -> Rectangle<i32, Logical> {
        Rectangle::default()
    }
}
delegate_compositor!(Server);
delegate_seat!(Server);
delegate_text_input_manager!(Server);
delegate_input_method_manager!(Server);

#[derive(Default)]
struct Client {
    globals: HashMap<String, (u32, u32)>,
    lock_configures: usize,
    lock_finished: usize,
    ime_keys: usize,
    client_keys: usize,
    text: Vec<(u32, ti::Event)>,
    ime: Vec<(u32, im::Event)>,
    popup_rectangles: Vec<(i32, i32, i32, i32)>,
}
impl Dispatch<wl_registry::WlRegistry, ()> for Client {
    fn event(
        state: &mut Self,
        _: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            state.globals.insert(interface, (name, version));
        }
    }
}
impl Dispatch<ti::ZwpTextInputV3, ()> for Client {
    fn event(
        state: &mut Self,
        proxy: &ti::ZwpTextInputV3,
        event: ti::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.text.push((proxy.id().protocol_id(), event));
    }
}
impl Dispatch<im::ZwpInputMethodV2, ()> for Client {
    fn event(
        state: &mut Self,
        proxy: &im::ZwpInputMethodV2,
        event: im::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.ime.push((proxy.id().protocol_id(), event));
    }
}
impl Dispatch<popup::ZwpInputPopupSurfaceV2, ()> for Client {
    fn event(
        state: &mut Self,
        _: &popup::ZwpInputPopupSurfaceV2,
        event: popup::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let popup::Event::TextInputRectangle {
            x,
            y,
            width,
            height,
        } = event
        {
            state.popup_rectangles.push((x, y, width, height));
        }
    }
}
impl Dispatch<ime_keyboard::ZwpInputMethodKeyboardGrabV2, ()> for Client {
    fn event(
        state: &mut Self,
        _: &ime_keyboard::ZwpInputMethodKeyboardGrabV2,
        event: ime_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, ime_keyboard::Event::Key { .. }) {
            state.ime_keys += 1;
        }
    }
}
impl Dispatch<wl_keyboard::WlKeyboard, ()> for Client {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, wl_keyboard::Event::Key { .. }) {
            state.client_keys += 1;
        }
    }
}

impl Dispatch<lock::ExtSessionLockV1, ()> for Client {
    fn event(
        state: &mut Self,
        _: &lock::ExtSessionLockV1,
        event: lock::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, lock::Event::Finished) {
            state.lock_finished += 1;
        }
    }
}
impl Dispatch<lock_surface::ExtSessionLockSurfaceV1, ()> for Client {
    fn event(
        state: &mut Self,
        _: &lock_surface::ExtSessionLockSurfaceV1,
        event: lock_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, lock_surface::Event::Configure { .. }) {
            state.lock_configures += 1;
        }
    }
}
delegate_noop!(Client: ignore lock_manager::ExtSessionLockManagerV1);
delegate_noop!(Client: ignore wayland_client::protocol::wl_output::WlOutput);

enum Control {
    Suspend(bool),
    Key,
}
delegate_noop!(Client: ignore wl_compositor::WlCompositor);
delegate_noop!(Client: ignore wl_surface::WlSurface);
delegate_noop!(Client: ignore wl_seat::WlSeat);
delegate_noop!(Client: ignore tim::ZwpTextInputManagerV3);
delegate_noop!(Client: ignore imm::ZwpInputMethodManagerV2);

struct Fixture {
    popups: Arc<std::sync::Mutex<Vec<PopupSurface>>>,
    state: Client,
    queue: EventQueue<Client>,
    compositor: wl_compositor::WlCompositor,
    lock_manager: lock_manager::ExtSessionLockManagerV1,
    output: wayland_client::protocol::wl_output::WlOutput,
    seat: wl_seat::WlSeat,
    manager: tim::ZwpTextInputManagerV3,
    ime_manager: imm::ZwpInputMethodManagerV2,
    input: ti::ZwpTextInputV3,
    ime: im::ZwpInputMethodV2,
    surface: wl_surface::WlSurface,
    control: std::sync::mpsc::Sender<(Control, std::sync::mpsc::SyncSender<()>)>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Fixture {
    fn new() -> Self {
        let (client_socket, server_socket) = UnixStream::pair().unwrap();
        let mut display = Display::<Server>::new().unwrap();
        let mut dh = display.handle();
        let compositor = CompositorState::new::<Server>(&dh);
        let lock_state = SessionLockManagerState::new::<Server, _>(&dh, |_| true);
        let output = smithay::output::Output::new(
            "test".into(),
            smithay::output::PhysicalProperties {
                size: (100, 100).into(),
                subpixel: smithay::output::Subpixel::Unknown,
                make: "test".into(),
                model: "test".into(),
                serial_number: "test".into(),
            },
        );
        output.create_global::<Server>(&dh);
        let mut seats = SeatState::new();
        let mut seat = seats.new_wl_seat(&dh, "test");
        seat.add_keyboard(Default::default(), 200, 25).unwrap();
        let _text = TextInputManagerState::new::<Server>(&dh);
        let _ime = InputMethodManagerState::new::<Server, _>(&dh, |_| true);
        dh.insert_client(server_socket, Arc::new(ClientData::default()))
            .unwrap();
        let popups = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut server = Server {
            popups: popups.clone(),
            lock_state,
            locked: false,
            rejected_locks: Default::default(),
            compositor,
            seats,
            seat,
        };
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let (control, controls) =
            std::sync::mpsc::channel::<(Control, std::sync::mpsc::SyncSender<()>)>();
        let worker = thread::spawn(move || {
            let _output = output;
            while !stopped.load(Ordering::Relaxed) {
                display.dispatch_clients(&mut server).unwrap();
                while let Ok((command, done)) = controls.try_recv() {
                    match command {
                        Control::Suspend(value) => {
                            server
                                .seat
                                .input_method()
                                .clone()
                                .set_suspended(&mut server, value);
                        }
                        Control::Key => {
                            let keyboard = server.seat.get_keyboard().unwrap();
                            for state in [
                                smithay::backend::input::KeyState::Pressed,
                                smithay::backend::input::KeyState::Released,
                            ] {
                                keyboard.input::<(), _>(
                                    &mut server,
                                    38u32.into(),
                                    state,
                                    SERIAL_COUNTER.next_serial(),
                                    0,
                                    |_, _, _| smithay::input::keyboard::FilterResult::Forward,
                                );
                            }
                        }
                    }
                    display.flush_clients().unwrap();
                    let _ = done.send(());
                }
                display.flush_clients().unwrap();
                thread::sleep(Duration::from_millis(1));
            }
        });
        let connection = Connection::from_socket(client_socket).unwrap();
        let mut queue = connection.new_event_queue();
        let qh = queue.handle();
        let registry = connection.display().get_registry(&qh, ());
        let mut state = Client::default();
        queue.roundtrip(&mut state).unwrap();
        let bind = |name: &str| state.globals[name].0;
        let compositor: wl_compositor::WlCompositor =
            registry.bind(bind("wl_compositor"), 4, &qh, ());
        let seat = registry.bind(bind("wl_seat"), 7, &qh, ());
        let manager: tim::ZwpTextInputManagerV3 =
            registry.bind(bind("zwp_text_input_manager_v3"), 1, &qh, ());
        let ime_manager: imm::ZwpInputMethodManagerV2 =
            registry.bind(bind("zwp_input_method_manager_v2"), 1, &qh, ());
        let lock_manager = registry.bind(bind("ext_session_lock_manager_v1"), 1, &qh, ());
        let output = registry.bind(bind("wl_output"), 4, &qh, ());
        let input = manager.get_text_input(&seat, &qh, ());
        let ime = ime_manager.get_input_method(&seat, &qh, ());
        let surface = compositor.create_surface(&qh, ());
        surface.commit();
        queue.roundtrip(&mut state).unwrap();
        let mut fixture = Self {
            popups,
            state,
            queue,
            compositor,
            lock_manager,
            output,
            seat,
            manager,
            ime_manager,
            input,
            ime,
            surface,
            control,
            stop,
            worker: Some(worker),
        };
        fixture.clear();
        fixture
    }
    fn command(&mut self, command: Control) {
        self.sync();
        let (done, wait) = std::sync::mpsc::sync_channel(1);
        self.control.send((command, done)).unwrap();
        wait.recv_timeout(Duration::from_secs(5)).unwrap();
        self.sync();
    }
    fn sync(&mut self) {
        self.queue.roundtrip(&mut self.state).unwrap();
    }
    fn clear(&mut self) {
        self.state.text.clear();
        self.state.ime.clear();
    }
    fn enable(&mut self) {
        self.input.enable();
        self.input.commit();
        self.sync();
        self.clear();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

#[test]
fn new_text_input_does_not_reenter_existing_objects() {
    let mut f = Fixture::new();
    let second = f.manager.get_text_input(&f.seat, &f.queue.handle(), ());
    f.sync();
    assert_eq!(f.state.text.len(), 1, "{:?}", f.state.text);
    assert_eq!(f.state.text[0].0, second.id().protocol_id());
    assert!(matches!(f.state.text[0].1, ti::Event::Enter { .. }));
}

#[test]
fn destroying_active_object_allows_another_object_to_enable() {
    let mut f = Fixture::new();
    f.enable();
    let second = f.manager.get_text_input(&f.seat, &f.queue.handle(), ());
    f.input.destroy();
    second.enable();
    second.commit();
    f.sync();
    assert!(
        f.state
            .ime
            .iter()
            .any(|(_, e)| matches!(e, im::Event::Deactivate))
    );
    assert!(
        f.state
            .ime
            .iter()
            .any(|(_, e)| matches!(e, im::Event::Activate))
    );
    f.clear();
    f.ime.commit_string("replacement".into());
    f.ime.commit(0);
    f.sync();
    assert!(f.state.text.iter().any(|(id, e)| *id == second.id().protocol_id() && matches!(e, ti::Event::CommitString { text: Some(text) } if text == "replacement")));
}

#[test]
fn enable_resets_pending_state_and_change_cause_defaults_each_commit() {
    let mut f = Fixture::new();
    f.input.set_surrounding_text("stale".into(), 5, 5);
    f.input
        .set_content_type(ti::ContentHint::SensitiveData, ti::ContentPurpose::Password);
    f.input.set_text_change_cause(ti::ChangeCause::Other);
    f.input.enable();
    f.input.commit();
    f.sync();
    assert!(!f.state.ime.iter().any(|(_, e)| matches!(
        e,
        im::Event::SurroundingText { .. } | im::Event::ContentType { .. }
    )));
    assert!(f.state.ime.iter().any(|(_, e)| matches!(e, im::Event::TextChangeCause { cause } if *cause == wayland_client::WEnum::Value(ti::ChangeCause::InputMethod))));
    f.input.set_text_change_cause(ti::ChangeCause::Other);
    f.input.commit();
    f.sync();
    f.clear();
    f.input.set_surrounding_text("new".into(), 3, 3);
    f.input.commit();
    f.sync();
    assert!(f.state.ime.iter().any(|(_, e)| matches!(e, im::Event::TextChangeCause { cause } if *cause == wayland_client::WEnum::Value(ti::ChangeCause::InputMethod))));
}

#[test]
fn ime_edits_are_buffered_last_write_wins_and_done_uses_commit_count() {
    let mut f = Fixture::new();
    f.enable();
    f.ime.commit_string("old".into());
    f.ime.commit_string("é🦀".into());
    f.ime.set_preedit_string("候補".into(), 0, 6);
    f.ime.delete_surrounding_text(2, 0);
    f.sync();
    assert!(f.state.text.is_empty(), "edits escaped before IME commit");
    f.ime.commit(0);
    f.sync();
    assert_eq!(f.state.text.len(), 4, "{:?}", f.state.text);
    assert!(
        matches!(&f.state.text[1].1, ti::Event::CommitString { text: Some(text) } if text == "é🦀")
    );
    assert!(matches!(f.state.text[3].1, ti::Event::Done { serial: 1 }));
    f.clear();
    f.ime.commit(0);
    f.sync();
    assert_eq!(f.state.text.len(), 1);
    assert!(matches!(f.state.text[0].1, ti::Event::Done { serial: 1 }));
}

#[test]
fn focus_change_discards_uncommitted_app_and_ime_state() {
    let mut f = Fixture::new();
    f.enable();
    f.input.set_surrounding_text("old field".into(), 0, 0);
    f.ime.commit_string("old composition".into());
    let other = f.compositor.create_surface(&f.queue.handle(), ());
    other.commit();
    f.sync();
    f.clear();
    f.input.enable();
    f.input.commit();
    f.ime.commit(0);
    f.sync();
    assert!(
        !f.state
            .ime
            .iter()
            .any(|(_, e)| matches!(e, im::Event::SurroundingText { .. }))
    );
    assert!(
        !f.state
            .text
            .iter()
            .any(|(_, e)| matches!(e, ti::Event::CommitString { .. }))
    );
    f.surface.commit();
    f.sync();
}

#[test]
fn rejected_second_ime_cannot_replace_or_disconnect_the_first() {
    let mut f = Fixture::new();
    f.enable();
    let second = f
        .ime_manager
        .get_input_method(&f.seat, &f.queue.handle(), ());
    f.sync();
    assert_eq!(f.state.ime.len(), 1, "{:?}", f.state.ime);
    assert_eq!(f.state.ime[0].0, second.id().protocol_id());
    assert!(matches!(f.state.ime[0].1, im::Event::Unavailable));
    assert!(
        f.state.text.is_empty(),
        "existing text inputs were reentered"
    );
    second.commit_string("intruder".into());
    second.commit(0);
    second.destroy();
    f.sync();
    f.clear();
    f.ime.commit_string("original".into());
    f.ime.commit(0);
    f.sync();
    assert!(f.state.text.iter().any(
        |(_, e)| matches!(e, ti::Event::CommitString { text: Some(text) } if text == "original")
    ));
}

#[test]
fn focus_leave_clears_pending_enable_without_resetting_commit_counter() {
    let mut f = Fixture::new();
    f.input.enable(); // never committed on the old surface
    let other = f.compositor.create_surface(&f.queue.handle(), ());
    other.commit();
    f.sync();
    f.clear();
    f.input.commit();
    f.sync();
    assert!(
        !f.state
            .ime
            .iter()
            .any(|(_, e)| matches!(e, im::Event::Activate))
    );
    f.enable();
    f.ime.commit_string("new field".into());
    f.ime.commit(0);
    f.sync();
    assert!(
        f.state
            .text
            .iter()
            .any(|(_, e)| matches!(e, ti::Event::Done { serial: 2 }))
    );
}

#[test]
fn disabled_and_inactive_objects_do_not_receive_composition() {
    let mut f = Fixture::new();
    f.enable();
    let second = f.manager.get_text_input(&f.seat, &f.queue.handle(), ());
    second.enable();
    second.commit();
    f.sync();
    f.clear();
    f.ime.commit_string("active only".into());
    f.ime.commit(0);
    f.sync();
    assert!(
        f.state
            .text
            .iter()
            .all(|(id, _)| *id == f.input.id().protocol_id())
    );
    f.input.disable();
    f.input.commit();
    f.sync();
    f.clear();
    f.ime.commit_string("disabled".into());
    f.ime.commit(0);
    f.sync();
    assert!(f.state.text.is_empty());
    // Consecutive disable and enable requests are valid.
    f.input.disable();
    f.input.commit();
    f.input.enable();
    f.input.commit();
    f.input.enable();
    f.input.commit();
    f.sync();
    f.clear();
    f.ime.commit_string("enabled again".into());
    f.ime.commit(0);
    f.sync();
    assert!(
        f.state
            .text
            .iter()
            .any(|(_, e)| matches!(e, ti::Event::Done { serial: 5 }))
    );
}

#[test]
fn ime_reconnect_and_inactive_object_destruction_preserve_focus() {
    let mut f = Fixture::new();
    f.enable();
    let spare = f.manager.get_text_input(&f.seat, &f.queue.handle(), ());
    spare.destroy();
    f.sync();
    assert!(
        !f.state
            .ime
            .iter()
            .any(|(_, e)| matches!(e, im::Event::Deactivate))
    );
    f.ime.destroy();
    f.sync();
    assert!(
        f.state
            .text
            .iter()
            .any(|(_, e)| matches!(e, ti::Event::Leave { .. }))
    );
    f.clear();
    f.ime = f
        .ime_manager
        .get_input_method(&f.seat, &f.queue.handle(), ());
    f.sync();
    assert!(
        f.state
            .text
            .iter()
            .any(|(_, e)| matches!(e, ti::Event::Enter { .. }))
    );
    f.enable();
    f.ime.commit_string("reconnected".into());
    f.ime.commit(1);
    f.sync();
    assert!(
        f.state
            .text
            .iter()
            .any(|(_, e)| matches!(e, ti::Event::Done { serial: 2 }))
    );
}

#[test]
fn reenable_clears_the_previous_fields_popup_rectangle() {
    let mut f = Fixture::new();
    f.enable();
    let surface = f.compositor.create_surface(&f.queue.handle(), ());
    let _popup = f
        .ime
        .get_input_popup_surface(&surface, &f.queue.handle(), ());
    f.input.set_cursor_rectangle(50, 60, 2, 20);
    f.input.commit();
    f.sync();
    assert_eq!(f.state.popup_rectangles.last(), Some(&(50, 60, 2, 20)));
    f.input.enable();
    f.input.commit();
    f.sync();
    assert_eq!(f.state.popup_rectangles.last(), Some(&(0, 0, 0, 0)));
}

#[test]
fn secure_input_hides_keys_and_text_from_existing_ime_then_resumes() {
    let mut f = Fixture::new();
    let _client_keyboard = f.seat.get_keyboard(&f.queue.handle(), ());
    let _grab = f.ime.grab_keyboard(&f.queue.handle(), ());
    f.enable();
    f.command(Control::Key);
    assert_eq!(f.state.ime_keys, 2);
    assert_eq!(f.state.client_keys, 0);
    f.command(Control::Suspend(true));
    f.clear();
    f.input.enable();
    f.input.set_surrounding_text("secret".into(), 6, 6);
    f.input.commit();
    f.ime.commit_string("injected".into());
    f.ime.commit(0);
    f.command(Control::Key);
    assert_eq!(f.state.ime_keys, 2, "IME received secure keys");
    assert_eq!(
        f.state.client_keys, 2,
        "focused secure client did not receive keys"
    );
    assert!(f.state.ime.is_empty(), "IME received secure text state");
    assert!(
        f.state.text.is_empty(),
        "IME injected text during suspension"
    );
    f.command(Control::Suspend(false));
    f.command(Control::Key);
    assert_eq!(f.state.ime_keys, 4, "IME did not resume");
    assert_eq!(f.state.client_keys, 2);
}

#[test]
fn ime_cannot_reacquire_keyboard_during_secure_input() {
    let mut f = Fixture::new();
    let _client_keyboard = f.seat.get_keyboard(&f.queue.handle(), ());
    f.command(Control::Suspend(true));
    let _grab = f.ime.grab_keyboard(&f.queue.handle(), ());
    f.command(Control::Key);
    assert_eq!(f.state.ime_keys, 0);
    assert_eq!(f.state.client_keys, 2);
    f.command(Control::Suspend(false));
    f.command(Control::Key);
    assert_eq!(f.state.ime_keys, 2);
}

#[test]
fn ime_started_during_secure_input_waits_until_resume() {
    let mut f = Fixture::new();
    f.ime.destroy();
    f.sync();
    f.command(Control::Suspend(true));
    f.clear();
    f.ime = f
        .ime_manager
        .get_input_method(&f.seat, &f.queue.handle(), ());
    let _grab = f.ime.grab_keyboard(&f.queue.handle(), ());
    f.command(Control::Key);
    assert_eq!(f.state.ime_keys, 0);
    assert!(
        f.state.text.is_empty(),
        "new IME exposed focus while locked"
    );
    f.command(Control::Suspend(false));
    f.command(Control::Key);
    assert_eq!(f.state.ime_keys, 2);
    assert!(
        f.state
            .text
            .iter()
            .any(|(_, event)| matches!(event, ti::Event::Enter { .. }))
    );
}

#[test]
fn rejected_lock_surface_requests_remain_inert_without_crashing_or_reserving_outputs() {
    let mut f = Fixture::new();
    let owner = f.lock_manager.lock(&f.queue.handle(), ());
    let rejected = f.lock_manager.lock(&f.queue.handle(), ());
    let surface = f.compositor.create_surface(&f.queue.handle(), ());
    // Pipeline creation before receiving the second lock's finished event.
    let inert = rejected.get_lock_surface(&surface, &f.output, &f.queue.handle(), ());
    f.sync();
    assert_eq!(f.state.lock_finished, 1);
    assert_eq!(f.state.lock_configures, 0);
    rejected.destroy();
    inert.ack_configure(123); // inert objects must tolerate queued requests too
    inert.destroy();
    f.sync();
    let actual_surface = f.compositor.create_surface(&f.queue.handle(), ());
    let _actual = owner.get_lock_surface(&actual_surface, &f.output, &f.queue.handle(), ());
    f.sync();
    assert_eq!(
        f.state.lock_configures, 1,
        "rejected lock reserved the output"
    );
}

#[test]
fn destroying_ime_releases_keyboard_and_old_children_cannot_break_reconnection() {
    let mut f = Fixture::new();
    let _keyboard = f.seat.get_keyboard(&f.queue.handle(), ());
    let old_grab = f.ime.grab_keyboard(&f.queue.handle(), ());
    f.enable();
    f.command(Control::Key);
    assert_eq!(f.state.ime_keys, 2);
    f.ime.destroy();
    f.command(Control::Key);
    assert_eq!(f.state.ime_keys, 2, "destroyed IME retained the keyboard");
    assert_eq!(f.state.client_keys, 2);
    f.ime = f
        .ime_manager
        .get_input_method(&f.seat, &f.queue.handle(), ());
    let _new_grab = f.ime.grab_keyboard(&f.queue.handle(), ());
    old_grab.release();
    f.command(Control::Key);
    assert_eq!(
        f.state.ime_keys, 4,
        "old child released the replacement IME grab"
    );
    assert_eq!(f.state.client_keys, 2);
}

#[test]
fn releasing_superseded_keyboard_object_preserves_current_grab() {
    let mut f = Fixture::new();
    let _keyboard = f.seat.get_keyboard(&f.queue.handle(), ());
    let old_grab = f.ime.grab_keyboard(&f.queue.handle(), ());
    let new_grab = f.ime.grab_keyboard(&f.queue.handle(), ());
    old_grab.release();
    f.command(Control::Key);
    assert_eq!(f.state.ime_keys, 2);
    assert_eq!(f.state.client_keys, 0);
    new_grab.release();
    f.command(Control::Key);
    assert_eq!(f.state.ime_keys, 2);
    assert_eq!(f.state.client_keys, 2);
}

#[test]
fn ime_popups_follow_activation_and_all_receive_the_caret_rectangle() {
    let mut f = Fixture::new();
    let first_surface = f.compositor.create_surface(&f.queue.handle(), ());
    let _first = f
        .ime
        .get_input_popup_surface(&first_surface, &f.queue.handle(), ());
    f.sync();
    assert!(
        f.popups.lock().unwrap().is_empty(),
        "inactive IME popup became visible"
    );
    f.enable();
    assert_eq!(f.popups.lock().unwrap().len(), 1);
    f.input.set_cursor_rectangle(25, 40, 3, 18);
    f.input.commit();
    f.sync();
    f.state.popup_rectangles.clear();
    let second_surface = f.compositor.create_surface(&f.queue.handle(), ());
    let _second = f
        .ime
        .get_input_popup_surface(&second_surface, &f.queue.handle(), ());
    f.sync();
    assert_eq!(f.state.popup_rectangles, vec![(25, 40, 3, 18)]);
    assert_eq!(f.popups.lock().unwrap().len(), 2);
    f.state.popup_rectangles.clear();
    f.input.set_cursor_rectangle(30, 50, 2, 16);
    f.input.commit();
    f.sync();
    assert_eq!(f.state.popup_rectangles, vec![(30, 50, 2, 16); 2]);
    f.input.disable();
    f.input.commit();
    f.sync();
    assert!(f.popups.lock().unwrap().is_empty());
    f.enable();
    assert_eq!(f.popups.lock().unwrap().len(), 2);
    f.ime.destroy();
    f.sync();
    assert!(f.popups.lock().unwrap().is_empty());
    f.ime = f
        .ime_manager
        .get_input_method(&f.seat, &f.queue.handle(), ());
    f.sync();
    f.enable();
    assert!(
        f.popups.lock().unwrap().is_empty(),
        "replacement IME revived old popups"
    );
}

#[test]
fn destroyed_popup_is_not_reactivated() {
    let mut f = Fixture::new();
    let surface = f.compositor.create_surface(&f.queue.handle(), ());
    let popup = f
        .ime
        .get_input_popup_surface(&surface, &f.queue.handle(), ());
    popup.destroy();
    f.sync();
    f.enable();
    assert!(f.popups.lock().unwrap().is_empty());
}
