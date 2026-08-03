// SPDX-License-Identifier: GPL-3.0-only

//! The Wayland thread: watches toplevels and workspaces, and minimizes or
//! restores on request.
//!
//! Everything Wayland-shaped is confined here. The applet above only ever sees
//! a list of [`WindowEntry`] and sends [`WaylandRequest`]s back, which is what
//! keeps the actual decision in `show_desktop.rs` testable without a compositor.

use crate::wayland_subscription::{WaylandRequest, WaylandUpdate, WindowEntry};

use cctk::{
    cosmic_protocols::toplevel_info::v1::client::zcosmic_toplevel_handle_v1,
    sctk::{
        self,
        reexports::{calloop, calloop_wayland_source::WaylandSource},
        registry::{ProvidesRegistryState, RegistryState},
        seat::{SeatHandler, SeatState},
    },
    toplevel_info::{ToplevelInfoHandler, ToplevelInfoState},
    toplevel_management::{ToplevelManagerHandler, ToplevelManagerState},
    wayland_client::{
        Connection, QueueHandle, WEnum,
        globals::registry_queue_init,
        protocol::wl_seat::WlSeat,
    },
    wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
    workspace::{WorkspaceHandler, WorkspaceState},
};
use cosmic::cctk::{
    self,
    cosmic_protocols::toplevel_management::v1::client::zcosmic_toplevel_manager_v1,
    wayland_protocols::ext::workspace::v1::client::ext_workspace_handle_v1,
};
use cosmic::iced::futures::{SinkExt, channel::mpsc, executor::block_on};

struct AppData {
    exit: bool,
    tx: mpsc::Sender<WaylandUpdate>,
    registry_state: RegistryState,
    toplevel_info_state: ToplevelInfoState,
    toplevel_manager_state: ToplevelManagerState,
    workspace_state: WorkspaceState,
    seat_state: SeatState,
    /// Last snapshot sent up, so an event storm that changes nothing the applet
    /// cares about does not turn into a stream of identical messages.
    last_sent: Vec<WindowEntry>,
}

impl AppData {
    /// Windows on the workspace the user is actually looking at.
    ///
    /// Filtering by workspace is not cosmetic: minimizing windows on the other
    /// workspaces would leave those empty when the user switched back, and the
    /// restore would be the only way to find them again.
    fn windows(&self) -> Vec<WindowEntry> {
        let active: Vec<_> = self
            .workspace_state
            .workspaces()
            .filter(|workspace| {
                workspace
                    .state
                    .contains(ext_workspace_handle_v1::State::Active)
            })
            .map(|workspace| workspace.handle.clone())
            .collect();

        self.toplevel_info_state
            .toplevels()
            .filter_map(|info| {
                // With no active workspace reported — an older compositor, or
                // before the first event — fall back to every window rather
                // than to none, so the button still does something sensible.
                let on_active =
                    active.is_empty() || info.workspace.iter().any(|w| active.contains(w));
                if !on_active {
                    return None;
                }
                Some(WindowEntry {
                    handle: info.foreign_toplevel.clone(),
                    identifier: info.identifier.clone(),
                    minimized: info
                        .state
                        .contains(&zcosmic_toplevel_handle_v1::State::Minimized),
                })
            })
            .collect()
    }

    fn send_windows(&mut self) {
        let windows = self.windows();
        if windows == self.last_sent {
            return;
        }
        self.last_sent = windows.clone();
        if let Err(err) = block_on(self.tx.send(WaylandUpdate::Windows(windows))) {
            tracing::error!("failed to send window list to the applet: {err:?}");
        }
    }

    fn cosmic_toplevel(
        &self,
        handle: &ExtForeignToplevelHandleV1,
    ) -> Option<zcosmic_toplevel_handle_v1::ZcosmicToplevelHandleV1> {
        self.toplevel_info_state
            .info(handle)
            .and_then(|info| info.cosmic_toplevel.clone())
    }
}

pub(crate) fn wayland_handler(
    tx: mpsc::Sender<WaylandUpdate>,
    rx: calloop::channel::Channel<WaylandRequest>,
) {
    let Ok(conn) = Connection::connect_to_env() else {
        tracing::error!("no wayland connection; the applet cannot show the desktop");
        return;
    };
    let Ok((globals, event_queue)) = registry_queue_init(&conn) else {
        tracing::error!("failed to initialize the wayland registry");
        return;
    };
    let qh = event_queue.handle();
    let wayland_source = WaylandSource::new(conn.clone(), event_queue);

    let Ok(mut event_loop) = calloop::EventLoop::<AppData>::try_new() else {
        tracing::error!("failed to create the event loop");
        return;
    };
    let handle = event_loop.handle();
    if wayland_source.insert(handle.clone()).is_err() {
        tracing::error!("failed to insert the wayland source");
        return;
    }

    if handle
        .insert_source(rx, |event, (), state| match event {
            calloop::channel::Event::Msg(req) => {
                let Some(seat) = state.seat_state.seats().next() else {
                    return;
                };
                let manager = &state.toplevel_manager_state.manager;
                match req {
                    WaylandRequest::Minimize(handle) => {
                        if let Some(toplevel) = state.cosmic_toplevel(&handle) {
                            manager.set_minimized(&toplevel);
                        }
                    }
                    WaylandRequest::Unminimize(handle) => {
                        if let Some(toplevel) = state.cosmic_toplevel(&handle) {
                            manager.unset_minimized(&toplevel);
                            // Bring it back to the front too: a window restored
                            // behind everything else looks like nothing
                            // happened.
                            manager.activate(&toplevel, &seat);
                        }
                    }
                }
            }
            calloop::channel::Event::Closed => {
                state.exit = true;
            }
        })
        .is_err()
    {
        tracing::error!("failed to insert the request channel");
        return;
    }

    let registry_state = RegistryState::new(&globals);
    let mut app_data = AppData {
        exit: false,
        tx,
        seat_state: SeatState::new(&globals, &qh),
        toplevel_info_state: ToplevelInfoState::new(&registry_state, &qh),
        toplevel_manager_state: ToplevelManagerState::new(&registry_state, &qh),
        workspace_state: WorkspaceState::new(&registry_state, &qh),
        registry_state,
        last_sent: Vec::new(),
    };

    loop {
        if app_data.exit {
            break;
        }
        if event_loop.dispatch(None, &mut app_data).is_err() {
            break;
        }
    }
}

impl ToplevelInfoHandler for AppData {
    fn toplevel_info_state(&mut self) -> &mut ToplevelInfoState {
        &mut self.toplevel_info_state
    }

    fn new_toplevel(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &ExtForeignToplevelHandleV1) {
        self.send_windows();
    }

    fn update_toplevel(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &ExtForeignToplevelHandleV1,
    ) {
        self.send_windows();
    }

    fn toplevel_closed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &ExtForeignToplevelHandleV1,
    ) {
        self.send_windows();
    }
}

impl WorkspaceHandler for AppData {
    fn workspace_state(&mut self) -> &mut WorkspaceState {
        &mut self.workspace_state
    }

    fn done(&mut self) {
        // Switching workspaces changes which windows are in scope, so the
        // applet needs a fresh list even though no toplevel moved.
        self.send_windows();
    }
}

impl ToplevelManagerHandler for AppData {
    fn toplevel_manager_state(&mut self) -> &mut ToplevelManagerState {
        &mut self.toplevel_manager_state
    }

    fn capabilities(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: Vec<WEnum<zcosmic_toplevel_manager_v1::ZcosmicToplelevelManagementCapabilitiesV1>>,
    ) {
    }
}

impl SeatHandler for AppData {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}

    fn new_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: WlSeat,
        _: sctk::seat::Capability,
    ) {
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: WlSeat,
        _: sctk::seat::Capability,
    ) {
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}
}

impl ProvidesRegistryState for AppData {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    sctk::registry_handlers!(SeatState);
}

sctk::delegate_seat!(AppData);
sctk::delegate_registry!(AppData);
cctk::delegate_toplevel_info!(AppData);
cctk::delegate_toplevel_manager!(AppData);
cctk::delegate_workspace!(AppData);
