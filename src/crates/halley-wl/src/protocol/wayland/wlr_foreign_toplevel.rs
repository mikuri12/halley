use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use halley_core::field::NodeId;
use smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1::server::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};
use smithay::reexports::wayland_server::{
    backend::{ClientId, GlobalId, ObjectId},
    protocol::wl_surface::WlSurface,
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, Weak,
};
use smithay::utils::SERIAL_COUNTER;

use crate::compositor::root::Halley;

#[derive(Debug)]
struct WlrForeignToplevelHandleInner {
    title: String,
    app_id: String,
    activated: bool,
    instances: Vec<Weak<ZwlrForeignToplevelHandleV1>>,
    closed: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct WlrForeignToplevelHandle {
    pub(crate) surface: WlSurface,
    inner: Arc<Mutex<WlrForeignToplevelHandleInner>>,
}

impl WlrForeignToplevelHandle {
    pub(crate) fn new(surface: WlSurface, title: String, app_id: String, activated: bool) -> Self {
        Self {
            surface,
            inner: Arc::new(Mutex::new(WlrForeignToplevelHandleInner {
                title,
                app_id,
                activated,
                instances: Vec::new(),
                closed: false,
            })),
        }
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.inner.lock().unwrap().closed
    }

    pub(crate) fn send_title(&self, title: &str) {
        let mut inner = self.inner.lock().unwrap();
        if inner.title == title {
            return;
        }
        inner.title = title.to_string();
        for instance in &inner.instances {
            if let Ok(handle) = instance.upgrade() {
                handle.title(title.to_string());
            }
        }
    }

    pub(crate) fn send_app_id(&self, app_id: &str) {
        let mut inner = self.inner.lock().unwrap();
        if inner.app_id == app_id {
            return;
        }
        inner.app_id = app_id.to_string();
        for instance in &inner.instances {
            if let Ok(handle) = instance.upgrade() {
                handle.app_id(app_id.to_string());
            }
        }
    }

    pub(crate) fn send_activated(&self, activated: bool) {
        let mut inner = self.inner.lock().unwrap();
        if inner.activated == activated {
            return;
        }
        inner.activated = activated;
        let state_bytes = if activated {
            (2u32).to_ne_bytes().to_vec()
        } else {
            Vec::new()
        };
        for instance in &inner.instances {
            if let Ok(handle) = instance.upgrade() {
                handle.state(state_bytes.clone());
                handle.done();
            }
        }
    }

    pub(crate) fn send_done(&self) {
        let inner = self.inner.lock().unwrap();
        for instance in &inner.instances {
            if let Ok(handle) = instance.upgrade() {
                handle.done();
            }
        }
    }

    pub(crate) fn send_closed(&self) {
        let mut inner = self.inner.lock().unwrap();
        if inner.closed {
            return;
        }
        inner.closed = true;
        for instance in inner.instances.drain(..) {
            if let Ok(handle) = instance.upgrade() {
                handle.closed();
            }
        }
    }

    fn init_new_instance(&self, handle: &ZwlrForeignToplevelHandleV1) {
        let inner = self.inner.lock().unwrap();
        let state_bytes = if inner.activated {
            (2u32).to_ne_bytes().to_vec()
        } else {
            Vec::new()
        };
        handle.title(inner.title.clone());
        handle.app_id(inner.app_id.clone());
        handle.state(state_bytes);
        handle.done();
        drop(inner);
        self.inner.lock().unwrap().instances.push(handle.downgrade());
    }

    fn remove_instance(&self, handle: &ZwlrForeignToplevelHandleV1) {
        let mut inner = self.inner.lock().unwrap();
        inner.instances.retain(|i| i != handle);
    }
}

#[derive(Debug)]
pub(crate) struct WlrForeignToplevelState {
    _global: GlobalId,
    managers: Vec<ZwlrForeignToplevelManagerV1>,
    toplevels: HashMap<ObjectId, WlrForeignToplevelHandle>,
    dh: DisplayHandle,
}

impl WlrForeignToplevelState {
    pub(crate) fn new(dh: &DisplayHandle) -> Self {
        let global = dh.create_global::<Halley, ZwlrForeignToplevelManagerV1, _>(3, ());
        Self {
            _global: global,
            managers: Vec::new(),
            toplevels: HashMap::new(),
            dh: dh.clone(),
        }
    }

    pub(crate) fn new_toplevel(
        &mut self,
        surface: WlSurface,
        title: impl Into<String>,
        app_id: impl Into<String>,
        activated: bool,
    ) -> WlrForeignToplevelHandle {
        let handle = WlrForeignToplevelHandle::new(surface.clone(), title.into(), app_id.into(), activated);
        let surface_id = surface.id();

        for manager in &self.managers {
            let Ok(client) = self.dh.get_client(manager.id()) else {
                continue;
            };
            let Ok(handle_res) = client.create_resource::<ZwlrForeignToplevelHandleV1, _, Halley>(
                &self.dh,
                manager.version(),
                handle.clone(),
            ) else {
                continue;
            };

            manager.toplevel(&handle_res);
            handle.init_new_instance(&handle_res);
        }

        self.toplevels.insert(surface_id, handle.clone());
        handle
    }

    pub(crate) fn remove_toplevel(&mut self, surface_id: &ObjectId) {
        if let Some(handle) = self.toplevels.remove(surface_id) {
            handle.send_closed();
        }
    }

    pub(crate) fn focus_changed(
        &mut self,
        focused_node: Option<NodeId>,
        surface_to_node: &HashMap<ObjectId, NodeId>,
    ) {
        for handle in self.toplevels.values() {
            let handle_node = surface_to_node.get(&handle.surface.id()).copied();
            let is_active = focused_node.is_some() && focused_node == handle_node;
            handle.send_activated(is_active);
        }
    }
}

impl GlobalDispatch<ZwlrForeignToplevelManagerV1, (), Halley> for Halley {
    fn bind(
        state: &mut Halley,
        dh: &DisplayHandle,
        client: &Client,
        resource: New<ZwlrForeignToplevelManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Halley>,
    ) {
        let manager = data_init.init(resource, ());
        let wlr_state = &mut state.platform.wlr_foreign_toplevel_state;

        for handle in wlr_state.toplevels.values() {
            if handle.is_closed() {
                continue;
            }
            if let Ok(handle_res) = client.create_resource::<ZwlrForeignToplevelHandleV1, _, Halley>(
                dh,
                manager.version(),
                handle.clone(),
            ) {
                manager.toplevel(&handle_res);
                handle.init_new_instance(&handle_res);
            }
        }

        wlr_state.managers.push(manager);
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, (), Halley> for Halley {
    fn request(
        state: &mut Halley,
        _client: &Client,
        manager: &ZwlrForeignToplevelManagerV1,
        request: zwlr_foreign_toplevel_manager_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Halley>,
    ) {
        match request {
            zwlr_foreign_toplevel_manager_v1::Request::Stop => {
                state
                    .platform
                    .wlr_foreign_toplevel_state
                    .managers
                    .retain(|m| m != manager);
                manager.finished();
            }
            _ => {}
        }
    }

    fn destroyed(
        state: &mut Halley,
        _client: ClientId,
        resource: &ZwlrForeignToplevelManagerV1,
        _data: &(),
    ) {
        state
            .platform
            .wlr_foreign_toplevel_state
            .managers
            .retain(|m| m != resource);
    }
}

impl Dispatch<ZwlrForeignToplevelHandleV1, WlrForeignToplevelHandle, Halley> for Halley {
    fn request(
        state: &mut Halley,
        _client: &Client,
        _handle_res: &ZwlrForeignToplevelHandleV1,
        request: zwlr_foreign_toplevel_handle_v1::Request,
        data: &WlrForeignToplevelHandle,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Halley>,
    ) {
        match request {
            zwlr_foreign_toplevel_handle_v1::Request::Activate { .. } => {
                crate::compositor::focus::system::set_keyboard_focus(
                    state,
                    Some(data.surface.clone()),
                    SERIAL_COUNTER.next_serial(),
                );
            }
            zwlr_foreign_toplevel_handle_v1::Request::Close => {
                let surf_id = data.surface.id();
                for top in state.platform.xdg_shell_state.toplevel_surfaces() {
                    if top.wl_surface().id() == surf_id {
                        top.send_close();
                        break;
                    }
                }
            }
            zwlr_foreign_toplevel_handle_v1::Request::Destroy => {}
            _ => {}
        }
    }

    fn destroyed(
        _state: &mut Halley,
        _client: ClientId,
        resource: &ZwlrForeignToplevelHandleV1,
        data: &WlrForeignToplevelHandle,
    ) {
        data.remove_instance(resource);
    }
}
