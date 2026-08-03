// SPDX-License-Identifier: GPL-3.0-only

//! `cosmic-applet-show-desktop --toggle`: one show-or-restore, then exit.
//!
//! This is what a keyboard shortcut or a touchpad gesture runs. It does exactly
//! what pressing the panel button does, and shares the remembered set with it
//! through [`crate::state_file`], so the two are interchangeable — put the
//! windows away with a gesture, bring them back with the button.

use crate::{
    show_desktop::{ShowDesktop, Step},
    state_file,
    wayland_handler::wayland_handler,
    wayland_subscription::{WaylandRequest, WaylandUpdate},
};
use cosmic::cctk::sctk::reexports::calloop;
use cosmic::iced::futures::{
    channel::mpsc,
    executor::block_on,
    stream::StreamExt,
};
use std::time::Duration;

/// How long to wait for the compositor to describe the current windows.
///
/// Generous, because this runs at the tail of a gesture and a wrong answer is
/// worse than a slow one — acting on a half-populated list would minimize some
/// windows and forget the rest.
const LIST_TIMEOUT: Duration = Duration::from_secs(2);

/// How long to let the requests reach the compositor before exiting. The
/// process owns the Wayland connection, so leaving too early would drop the
/// requests it just made.
const FLUSH_GRACE: Duration = Duration::from_millis(250);

pub fn run() -> Result<(), String> {
    let (update_tx, mut update_rx) = mpsc::channel::<WaylandUpdate>(4);
    let (calloop_tx, calloop_rx) = calloop::channel::channel::<WaylandRequest>();

    let handler = std::thread::spawn(move || wayland_handler(update_tx, calloop_rx));

    // The handler announces itself, then sends the window list once the
    // compositor has described everything.
    let deadline = std::time::Instant::now() + LIST_TIMEOUT;
    let windows = loop {
        if std::time::Instant::now() >= deadline {
            return Err("timed out waiting for the compositor's window list".into());
        }
        match block_on(update_rx.next()) {
            Some(WaylandUpdate::Windows(windows)) => break windows,
            Some(WaylandUpdate::Init(_)) => continue,
            Some(WaylandUpdate::Finished) | None => {
                return Err("the wayland connection closed".into());
            }
        }
    };

    let mut state = ShowDesktop::from_hidden(state_file::load());
    let steps = state.toggle(&crate::to_windows(&windows));
    state_file::save(state.hidden());

    for step in steps {
        let (identifier, minimize) = match step {
            Step::Minimize(id) => (id, true),
            Step::Unminimize(id) => (id, false),
        };
        let Some(entry) = windows.iter().find(|entry| entry.identifier == identifier) else {
            continue;
        };
        let request = if minimize {
            WaylandRequest::Minimize(entry.handle.clone())
        } else {
            WaylandRequest::Unminimize(entry.handle.clone())
        };
        if calloop_tx.send(request).is_err() {
            return Err("the wayland thread went away mid-toggle".into());
        }
    }

    std::thread::sleep(FLUSH_GRACE);
    // Dropping the sender tells the handler to stop, which ends its event loop.
    drop(calloop_tx);
    let _ = handler.join();
    Ok(())
}
