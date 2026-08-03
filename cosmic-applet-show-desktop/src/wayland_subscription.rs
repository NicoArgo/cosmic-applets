// SPDX-License-Identifier: GPL-3.0-only

//! Plumbing between the applet and the Wayland thread.

use cctk::{
    sctk::reexports::calloop,
    wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
};
use cosmic::{
    cctk,
    iced::{self, Subscription, futures, stream},
};
use futures::SinkExt;

use crate::wayland_handler::wayland_handler;

pub fn wayland_subscription() -> iced::Subscription<WaylandUpdate> {
    Subscription::run_with(std::any::TypeId::of::<WaylandUpdate>(), |_| {
        stream::channel(
            1,
            move |mut output: futures::channel::mpsc::Sender<WaylandUpdate>| async move {
                let (calloop_tx, calloop_rx) = calloop::channel::channel();
                let runtime = tokio::runtime::Handle::current();

                let _ = std::thread::spawn(move || {
                    runtime.block_on(async move {
                        _ = output.send(WaylandUpdate::Init(calloop_tx)).await;
                        wayland_handler(output.clone(), calloop_rx);
                        tracing::error!("Wayland handler thread died");
                        _ = output.send(WaylandUpdate::Finished).await;
                    });
                });

                futures::future::pending().await
            },
        )
    })
}

/// A window on the active workspace, reduced to what this applet needs.
///
/// The applet never sees Wayland objects: the thread flattens each toplevel to
/// its handle and whether it is minimized, which is the whole input to the
/// decision in [`crate::show_desktop`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowEntry {
    pub handle: ExtForeignToplevelHandleV1,
    pub minimized: bool,
}

#[derive(Clone, Debug)]
pub enum WaylandUpdate {
    Init(calloop::channel::Sender<WaylandRequest>),
    Finished,
    /// The full set of windows on the active workspace, sent whenever it
    /// changes. A snapshot rather than a diff: the set is small, and the
    /// decision needs all of it anyway.
    Windows(Vec<WindowEntry>),
}

#[derive(Clone, Debug)]
pub enum WaylandRequest {
    Minimize(ExtForeignToplevelHandleV1),
    Unminimize(ExtForeignToplevelHandleV1),
}
