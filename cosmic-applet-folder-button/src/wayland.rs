// SPDX-License-Identifier: GPL-3.0-only

//! The Wayland thread: keeps the applet's window list current, and brings a
//! window forward on request. Adapted from the show-desktop applet, minus the
//! workspace filter — the folder's window may be on another workspace, and
//! bringing it forward from there is exactly the point.

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
    wayland_client::{Connection, QueueHandle, WEnum, globals::registry_queue_init, protocol::wl_seat::WlSeat},
    wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
};
use cosmic::{
    cctk::{self, cosmic_protocols::toplevel_management::v1::client::zcosmic_toplevel_manager_v1},
    iced::{
        self, Subscription,
        futures::{self, SinkExt, channel::mpsc, executor::block_on},
        stream,
    },
};

use crate::folder::Window;

pub type Handle = ExtForeignToplevelHandleV1;

#[derive(Clone, Debug)]
pub enum Update {
    Init(calloop::channel::Sender<Request>),
    Finished,
    /// Every window, whenever anything about one changes.
    Windows(Vec<Window<Handle>>),
}

#[derive(Clone, Debug)]
pub enum Request {
    /// Unminimize if needed, and give it focus (switching workspace if it
    /// lives on another one).
    Activate(Handle),
}

pub fn subscription() -> iced::Subscription<Update> {
    Subscription::run_with(std::any::TypeId::of::<Update>(), |_| {
        stream::channel(1, move |mut output: mpsc::Sender<Update>| async move {
            let (calloop_tx, calloop_rx) = calloop::channel::channel();
            let runtime = tokio::runtime::Handle::current();
            let _ = std::thread::spawn(move || {
                runtime.block_on(async move {
                    _ = output.send(Update::Init(calloop_tx)).await;
                    handler(output.clone(), calloop_rx);
                    tracing::error!("Wayland handler thread died");
                    _ = output.send(Update::Finished).await;
                });
            });
            futures::future::pending().await
        })
    })
}

struct AppData {
    exit: bool,
    tx: mpsc::Sender<Update>,
    registry_state: RegistryState,
    toplevel_info_state: ToplevelInfoState,
    toplevel_manager_state: ToplevelManagerState,
    seat_state: SeatState,
    last_sent: Vec<Window<Handle>>,
}

impl AppData {
    fn send_windows(&mut self) {
        let windows: Vec<_> = self
            .toplevel_info_state
            .toplevels()
            .map(|info| Window {
                handle: info.foreign_toplevel.clone(),
                app_id: info.app_id.clone(),
                title: info.title.clone(),
                minimized: info.state.contains(&zcosmic_toplevel_handle_v1::State::Minimized),
                active: info.state.contains(&zcosmic_toplevel_handle_v1::State::Activated),
            })
            .collect();
        if windows == self.last_sent {
            return;
        }
        self.last_sent = windows.clone();
        if let Err(err) = block_on(self.tx.send(Update::Windows(windows))) {
            tracing::error!("failed to send window list to the applet: {err:?}");
        }
    }
}

fn handler(tx: mpsc::Sender<Update>, rx: calloop::channel::Channel<Request>) {
    let Ok(conn) = Connection::connect_to_env() else {
        tracing::error!("no wayland connection");
        return;
    };
    let Ok((globals, event_queue)) = registry_queue_init(&conn) else {
        tracing::error!("failed to initialize the wayland registry");
        return;
    };
    let qh = event_queue.handle();
    let Ok(mut event_loop) = calloop::EventLoop::<AppData>::try_new() else {
        tracing::error!("failed to create the event loop");
        return;
    };
    let handle = event_loop.handle();
    if WaylandSource::new(conn.clone(), event_queue)
        .insert(handle.clone())
        .is_err()
    {
        tracing::error!("failed to insert the wayland source");
        return;
    }

    if handle
        .insert_source(rx, |event, (), state| match event {
            calloop::channel::Event::Msg(Request::Activate(handle)) => {
                let Some(seat) = state.seat_state.seats().next() else {
                    return;
                };
                let Some(toplevel) = state
                    .toplevel_info_state
                    .info(&handle)
                    .and_then(|info| info.cosmic_toplevel.clone())
                else {
                    return;
                };
                let manager = &state.toplevel_manager_state.manager;
                manager.unset_minimized(&toplevel);
                manager.activate(&toplevel, &seat);
            }
            calloop::channel::Event::Closed => state.exit = true,
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
        registry_state,
        last_sent: Vec::new(),
    };

    while !app_data.exit {
        if event_loop.dispatch(None, &mut app_data).is_err() {
            break;
        }
    }
}

impl ToplevelInfoHandler for AppData {
    fn toplevel_info_state(&mut self) -> &mut ToplevelInfoState {
        &mut self.toplevel_info_state
    }
    fn new_toplevel(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Handle) {
        self.send_windows();
    }
    fn update_toplevel(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Handle) {
        self.send_windows();
    }
    fn toplevel_closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Handle) {
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
    fn new_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat, _: sctk::seat::Capability) {}
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat, _: sctk::seat::Capability) {}
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
