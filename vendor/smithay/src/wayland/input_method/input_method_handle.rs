use std::{
    fmt,
    sync::{Arc, Mutex},
};

use tracing::warn;
use wayland_protocols_misc::zwp_input_method_v2::server::{
    zwp_input_method_keyboard_grab_v2::ZwpInputMethodKeyboardGrabV2,
    zwp_input_method_v2::{self, ZwpInputMethodV2},
    zwp_input_popup_surface_v2::ZwpInputPopupSurfaceV2,
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, Resource, protocol::wl_keyboard::KeymapFormat,
};
use wayland_server::{backend::ClientId, protocol::wl_surface::WlSurface};

use crate::{
    input::{SeatHandler, keyboard::KeyboardHandle},
    utils::{Logical, Rectangle, SERIAL_COUNTER, alive_tracker::AliveTracker},
    wayland::{compositor, seat::WaylandFocus, text_input::TextInputHandle},
};

use super::{
    INPUT_POPUP_SURFACE_ROLE, InputMethodHandler, InputMethodKeyboardUserData,
    InputMethodManagerState, InputMethodPopupSurfaceUserData,
    input_method_keyboard_grab::InputMethodKeyboardGrab,
    input_method_popup_surface::{PopupHandle, PopupParent, PopupSurface},
};

#[derive(Default, Debug)]
pub(crate) struct InputMethod {
    pub instance: Option<Instance>,
    pub popup_handle: PopupHandle,
    pub keyboard_grab: InputMethodKeyboardGrab,
    pending: PendingText,
    suspended: bool,
}

#[derive(Default, Debug)]
struct PendingText {
    commit: Option<String>,
    preedit: Option<(String, i32, i32)>,
    delete: Option<(u32, u32)>,
}

#[derive(Debug)]
pub(crate) struct Instance {
    pub object: ZwpInputMethodV2,
    pub serial: u32,
}

impl Instance {
    /// Send the done incrementing the serial.
    pub(crate) fn done(&mut self) {
        self.object.done();
        self.serial = self.serial.wrapping_add(1);
    }
}

/// Handle to an input method instance
#[derive(Default, Debug, Clone)]
pub struct InputMethodHandle {
    pub(crate) inner: Arc<Mutex<InputMethod>>,
}

impl InputMethodHandle {
    pub(super) fn add_instance(&self, instance: &ZwpInputMethodV2) -> bool {
        let mut inner = self.inner.lock().unwrap();
        if inner.instance.is_some() {
            instance.unavailable();
            false
        } else {
            inner.instance = Some(Instance {
                object: instance.clone(),
                serial: 0,
            });
            true
        }
    }

    /// Whether there's an active instance of input-method.
    pub(crate) fn has_instance(&self) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.instance.is_some() && !inner.suspended
    }

    /// Suspend input-method access during secure compositor input (such as a lock).
    /// Existing protocol objects survive, but their grab is inactive and text-input
    /// state is hidden until resumed. New grab objects can be created while suspended
    /// without becoming active; this lets an IME survive a lock/unlock cycle.
    pub fn set_suspended<D: SeatHandler + 'static>(&self, state: &mut D, suspended: bool) {
        let object = {
            let inner = self.inner.lock().unwrap();
            if inner.suspended == suspended {
                return;
            }
            inner
                .instance
                .as_ref()
                .map(|instance| instance.object.clone())
        };
        if suspended {
            self.deactivate_input_method(state);
        }
        self.inner.lock().unwrap().suspended = suspended;
        let Some(object) = object else {
            return;
        };
        let data = object.data::<InputMethodUserData<D>>().unwrap();
        if suspended {
            data.text_input_handle.leave();
            let owns_grab = data
                .keyboard_handle
                .with_grab(|_, grab| grab.downcast_ref::<InputMethodKeyboardGrab>().is_some())
                .unwrap_or(false);
            if owns_grab {
                data.keyboard_handle.unset_grab(state);
            }
        } else {
            data.text_input_handle.enter();
            if self.keyboard_grabbed() {
                let grab = self.inner.lock().unwrap().keyboard_grab.clone();
                data.keyboard_handle
                    .set_grab(state, grab, SERIAL_COUNTER.next_serial());
            }
        }
    }

    /// Whether input-method access is suspended for secure compositor input.
    pub fn is_suspended(&self) -> bool {
        self.inner.lock().unwrap().suspended
    }

    /// Callback function to access the input method object
    pub(crate) fn with_instance<F>(&self, f: F)
    where
        F: FnOnce(&mut Instance),
    {
        let mut inner = self.inner.lock().unwrap();
        if inner.suspended {
            return;
        }
        if let Some(instance) = inner.instance.as_mut() {
            f(instance);
        }
    }

    /// Callback function to access the input method.
    pub(crate) fn with_input_method<F>(&self, mut f: F)
    where
        F: FnMut(&mut InputMethod),
    {
        let mut inner = self.inner.lock().unwrap();
        f(&mut inner);
    }

    /// Indicates that an input method has grabbed a keyboard
    pub fn keyboard_grabbed(&self) -> bool {
        let inner = self.inner.lock().unwrap();
        let keyboard = inner.keyboard_grab.inner.lock().unwrap();
        keyboard.grab.is_some()
    }

    pub(crate) fn set_text_input_rectangle<D: SeatHandler + 'static>(
        &self,
        state: &mut D,
        rect: Rectangle<i32, Logical>,
    ) {
        let mut inner = self.inner.lock().unwrap();
        inner.popup_handle.rectangle = rect;

        inner.popup_handle.surfaces.retain(PopupSurface::alive);
        for mut popup_surface in inner.popup_handle.surfaces.clone() {
            popup_surface.set_text_input_rectangle(rect.loc.x, rect.loc.y, rect.size.w, rect.size.h);
            if let Some(instance) = &inner.instance {
                let data = instance.object.data::<InputMethodUserData<D>>().unwrap();
                (data.popup_repositioned)(state, popup_surface);
            }
        }
    }

    /// Activate input method on the given surface.
    pub(crate) fn activate_input_method<D: SeatHandler + 'static>(
        &self,
        state: &mut D,
        surface: &WlSurface,
    ) {
        self.with_input_method(|im| {
            im.pending = PendingText::default();
            im.popup_handle.rectangle = Rectangle::default();
            if let Some(instance) = im.instance.as_ref() {
                instance.object.activate();
                im.popup_handle.surfaces.retain(PopupSurface::alive);
                for popup in &mut im.popup_handle.surfaces {
                    let data = instance.object.data::<InputMethodUserData<D>>().unwrap();
                    let location = (data.popup_geometry_callback)(state, surface);
                    // Remove old popup.
                    (data.dismiss_popup)(state, popup.clone());
                    popup.set_text_input_rectangle(0, 0, 0, 0);

                    // Add a new one with updated parent.
                    let parent = PopupParent {
                        surface: surface.clone(),
                        location,
                    };
                    popup.set_parent(Some(parent));
                    (data.new_popup)(state, popup.clone());
                }
            }
        });
    }

    /// Deactivate the active input method.
    ///
    /// The `done` is always send when deactivating IME.
    pub(crate) fn deactivate_input_method<D: SeatHandler + 'static>(&self, state: &mut D) {
        self.with_input_method(|im| {
            im.pending = PendingText::default();
            if let Some(instance) = im.instance.as_mut() {
                instance.object.deactivate();
                instance.done();
                im.popup_handle.surfaces.retain(PopupSurface::alive);
                for popup in &mut im.popup_handle.surfaces {
                    let data = instance.object.data::<InputMethodUserData<D>>().unwrap();
                    if popup.get_parent().is_some() {
                        (data.dismiss_popup)(state, popup.clone());
                    }
                    popup.set_parent(None);
                }
            }
        });
    }
}

/// User data of ZwpInputMethodV2 object
pub struct InputMethodUserData<D: SeatHandler> {
    pub(super) handle: InputMethodHandle,
    pub(crate) text_input_handle: TextInputHandle,
    pub(crate) keyboard_handle: KeyboardHandle<D>,
    pub(crate) popup_geometry_callback: fn(&D, &WlSurface) -> Rectangle<i32, Logical>,
    pub(crate) new_popup: fn(&mut D, PopupSurface),
    pub(crate) popup_repositioned: fn(&mut D, PopupSurface),
    pub(crate) dismiss_popup: fn(&mut D, PopupSurface),
}

impl<D: SeatHandler> fmt::Debug for InputMethodUserData<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InputMethodUserData")
            .field("handle", &self.handle)
            .field("text_input_handle", &self.text_input_handle)
            .field("keyboard_handle", &self.keyboard_handle)
            .finish()
    }
}

impl<D> Dispatch<ZwpInputMethodV2, InputMethodUserData<D>, D> for InputMethodManagerState
where
    D: Dispatch<ZwpInputMethodV2, InputMethodUserData<D>>,
    D: Dispatch<ZwpInputPopupSurfaceV2, InputMethodPopupSurfaceUserData>,
    D: Dispatch<ZwpInputMethodKeyboardGrabV2, InputMethodKeyboardUserData<D>>,
    D: SeatHandler,
    D: InputMethodHandler,
    <D as SeatHandler>::KeyboardFocus: WaylandFocus,
    D: 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        seat: &ZwpInputMethodV2,
        request: zwp_input_method_v2::Request,
        data: &InputMethodUserData<D>,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        let is_current = data
            .handle
            .inner
            .lock()
            .unwrap()
            .instance
            .as_ref()
            .is_some_and(|instance| instance.object == *seat);
        if !is_current {
            match request {
                zwp_input_method_v2::Request::GetInputPopupSurface { id, .. } => {
                    data_init.init(
                        id,
                        InputMethodPopupSurfaceUserData {
                            alive_tracker: AliveTracker::default(),
                        },
                    );
                }
                zwp_input_method_v2::Request::GrabKeyboard { keyboard } => {
                    data_init.init(
                        keyboard,
                        InputMethodKeyboardUserData {
                            handle: InputMethodKeyboardGrab::default(),
                            keyboard_handle: data.keyboard_handle.clone(),
                        },
                    );
                }
                _ => {}
            }
            return;
        }
        if data.handle.is_suspended()
            && matches!(
                request,
                zwp_input_method_v2::Request::CommitString { .. }
                    | zwp_input_method_v2::Request::SetPreeditString { .. }
                    | zwp_input_method_v2::Request::DeleteSurroundingText { .. }
                    | zwp_input_method_v2::Request::Commit { .. }
            )
        {
            return;
        }
        match request {
            zwp_input_method_v2::Request::CommitString { text } => {
                data.handle.inner.lock().unwrap().pending.commit = Some(text);
            }
            zwp_input_method_v2::Request::SetPreeditString {
                text,
                cursor_begin,
                cursor_end,
            } => {
                data.handle.inner.lock().unwrap().pending.preedit =
                    Some((text, cursor_begin, cursor_end));
            }
            zwp_input_method_v2::Request::DeleteSurroundingText {
                before_length,
                after_length,
            } => {
                data.handle.inner.lock().unwrap().pending.delete =
                    Some((before_length, after_length));
            }
            zwp_input_method_v2::Request::Commit { serial: _ } => {
                // Even a stale IME serial must apply the edits normally. The text-input
                // done serial is the application's commit count, never a sentinel.
                let pending = std::mem::take(&mut data.handle.inner.lock().unwrap().pending);
                data.text_input_handle
                    .with_active_text_input(|ti, _surface| {
                        if let Some((before, after)) = pending.delete {
                            ti.delete_surrounding_text(before, after);
                        }
                        if let Some(text) = &pending.commit {
                            ti.commit_string(Some(text.clone()));
                        }
                        if let Some((text, begin, end)) = &pending.preedit {
                            ti.preedit_string(Some(text.clone()), *begin, *end);
                        }
                    });
                data.text_input_handle.done();
            }
            zwp_input_method_v2::Request::GetInputPopupSurface { id, surface } => {
                if compositor::give_role(&surface, INPUT_POPUP_SURFACE_ROLE).is_err()
                    && compositor::get_role(&surface) != Some(INPUT_POPUP_SURFACE_ROLE)
                {
                    // Protocol requires this raise an error, but doesn't define an error enum
                    seat.post_error(0u32, "Surface already has a role.");
                    return;
                }

                let mut parent = None;
                if !data.handle.is_suspended() {
                    data.text_input_handle.with_active_text_input(|_, surface| {
                        parent = Some(PopupParent {
                            surface: surface.clone(),
                            location: state.parent_geometry(surface),
                        });
                    });
                }
                let mut input_method = data.handle.inner.lock().unwrap();

                let instance = data_init.init(
                    id,
                    InputMethodPopupSurfaceUserData {
                        alive_tracker: AliveTracker::default(),
                    },
                );
                let popup_rect = Arc::new(Mutex::new(input_method.popup_handle.rectangle));
                let mut popup = PopupSurface::new(instance, surface, popup_rect, parent);
                let rect = input_method.popup_handle.rectangle;
                popup.set_text_input_rectangle(rect.loc.x, rect.loc.y, rect.size.w, rect.size.h);
                input_method.popup_handle.surfaces.retain(PopupSurface::alive);
                input_method.popup_handle.surfaces.push(popup.clone());
                if popup.get_parent().is_some() {
                    state.new_popup(popup);
                }
            }
            zwp_input_method_v2::Request::GrabKeyboard { keyboard } => {
                let input_method = data.handle.inner.lock().unwrap();
                if !input_method.suspended {
                    data.keyboard_handle.set_grab(
                        state,
                        input_method.keyboard_grab.clone(),
                        SERIAL_COUNTER.next_serial(),
                    );
                }
                let instance = data_init.init(
                    keyboard,
                    InputMethodKeyboardUserData {
                        handle: input_method.keyboard_grab.clone(),
                        keyboard_handle: data.keyboard_handle.clone(),
                    },
                );
                let mut keyboard = input_method.keyboard_grab.inner.lock().unwrap();
                keyboard.grab = Some(instance.clone());
                keyboard.text_input_handle = data.text_input_handle.clone();
                let guard = data.keyboard_handle.arc.internal.lock().unwrap();
                instance.repeat_info(guard.repeat_rate, guard.repeat_delay);
                let keymap_file = data.keyboard_handle.arc.keymap.lock().unwrap();
                let res = keymap_file.with_fd(false, |fd, size| {
                    instance.keymap(KeymapFormat::XkbV1, fd, size as u32);
                });

                if let Err(err) = res {
                    warn!(err = ?err, "Failed to send keymap to client");
                } else {
                    // Modifiers can be latched when taking the grab, thus we must send them to keep
                    // them in sync.
                    let mods = guard.mods_state.serialized;
                    instance.modifiers(
                        SERIAL_COUNTER.next_serial().into(),
                        mods.depressed,
                        mods.latched,
                        mods.locked,
                        mods.layout_effective,
                    );
                }
            }
            zwp_input_method_v2::Request::Destroy => {
                // Nothing to do
            }
            _ => unreachable!(),
        }
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        input_method: &ZwpInputMethodV2,
        data: &InputMethodUserData<D>,
    ) {
        let is_current = data
            .handle
            .inner
            .lock()
            .unwrap()
            .instance
            .as_ref()
            .is_some_and(|instance| instance.object == *input_method);
        if is_current {
            data.handle.deactivate_input_method(state);
            let grab = {
                let mut inner = data.handle.inner.lock().unwrap();
                inner.instance = None;
                for popup in inner.popup_handle.surfaces.drain(..) {
                    popup.surface_role
                        .data::<InputMethodPopupSurfaceUserData>()
                        .unwrap()
                        .alive_tracker
                        .destroy_notify();
                }
                inner.popup_handle.rectangle = Rectangle::default();
                // Old child objects must not share the next IME's grab state.
                std::mem::take(&mut inner.keyboard_grab)
            };
            grab.release(state, &data.keyboard_handle);
            data.text_input_handle.leave();
        }
    }
}
