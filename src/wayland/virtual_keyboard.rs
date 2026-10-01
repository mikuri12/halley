//! Synthetic keyboard input is unavailable while the session is locked.
use crate::session::{Session, SessionDriver};
use smithay::reexports::wayland_protocols_misc::zwp_virtual_keyboard_v1::server::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::{Request, ZwpVirtualKeyboardV1},
};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, delegate_dispatch, delegate_global_dispatch,
};
use smithay::wayland::virtual_keyboard::{
    VirtualKeyboardManagerGlobalData, VirtualKeyboardManagerState, VirtualKeyboardUserData,
};

delegate_global_dispatch!(@<D: SessionDriver> Session<D>:
    [ZwpVirtualKeyboardManagerV1: VirtualKeyboardManagerGlobalData] => VirtualKeyboardManagerState);
delegate_dispatch!(@<D: SessionDriver> Session<D>:
    [ZwpVirtualKeyboardManagerV1: ()] => VirtualKeyboardManagerState);

impl<D: SessionDriver> Dispatch<ZwpVirtualKeyboardV1, VirtualKeyboardUserData<Self>>
    for Session<D>
{
    fn request(
        state: &mut Self,
        client: &Client,
        resource: &ZwpVirtualKeyboardV1,
        request: Request,
        data: &VirtualKeyboardUserData<Self>,
        display: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        // None of these requests creates an object. Dropping a keymap request
        // also closes its received descriptor; Destroy still gets normal cleanup.
        if state.session_lock.active() {
            return;
        }
        <VirtualKeyboardManagerState as Dispatch<
            ZwpVirtualKeyboardV1,
            VirtualKeyboardUserData<Self>,
            Self,
        >>::request(state, client, resource, request, data, display, data_init);
    }
}
