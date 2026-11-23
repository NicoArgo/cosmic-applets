// SPDX-License-Identifier: GPL-3.0-only

//! `cosmic-applet-show-desktop --toggle`: one show-or-restore, then exit.
//!
//! This is what a keyboard shortcut or a touchpad gesture runs. It does exactly
//! what pressing the panel button does, and shares the remembered set with it
//! through [`crate::state_file`], so the two are interchangeable — put the
//! windows away with a gesture, bring them back with the button.
//!
//! Which windows depends on the setting ([`crate::config`]): with "this
//! screen", the output named by `--output` (a screen corner names its own), or
//! else the output of the focused window (Super+D); with "all screens", every
//! output.
//!
//! With `--wait` it connects and gathers the window list first, then acts when
//! a line arrives on stdin. The screen corner starts it that way as the
//! pointer arrives, so by the time the rest is over the list is already here:
//! COSMIC only sends it 45 to 145 ms after a client connects (measured
//! 23/11/2026), which used to be paid after every rest, on top of it.

use crate::{
    config, scope,
    show_desktop::Step,
    state_file,
    wayland_handler::wayland_handler,
    wayland_subscription::{WaylandRequest, WaylandUpdate},
};
use cosmic::cctk::sctk::reexports::calloop;
use cosmic::iced::futures::channel::mpsc::{self, TryRecvError};
use std::time::{Duration, Instant};

/// How long to wait for the compositor to describe the current windows.
///
/// Generous, because this runs at the tail of a gesture and a wrong answer is
/// worse than a slow one — acting on a half-populated list would minimize some
/// windows and forget the rest.
const LIST_TIMEOUT: Duration = Duration::from_secs(2);

/// The window list is complete once the compositor has been quiet this long.
///
/// The Wayland thread sends a fresh list after every window it learns about,
/// so the first list names only the first window. Acting on it would put one
/// window away and forget the rest; waiting for the stream to settle gets them
/// all.
///
/// The whole list arrives in one burst (five windows within 0.1 ms, measured
/// 23/11/2026), so a short quiet is enough; it was 150 ms, all of it waited
/// on every toggle.
const SETTLE: Duration = Duration::from_millis(30);

/// How long to let the requests reach the compositor before exiting. The
/// process owns the Wayland connection, so leaving too early would drop the
/// requests it just made.
const FLUSH_GRACE: Duration = Duration::from_millis(250);

pub fn run(output: Option<String>, dry_run: bool, wait: bool) -> Result<(), String> {
    let (update_tx, mut update_rx) = mpsc::channel::<WaylandUpdate>(4);
    let (calloop_tx, calloop_rx) = calloop::channel::channel::<WaylandRequest>();

    let handler = std::thread::spawn(move || wayland_handler(update_tx, calloop_rx));

    // `--wait`: the go-ahead is a line on stdin; stdin closing first means the
    // pointer left the corner and nothing should happen.
    let go_rx = wait.then(|| {
        let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            let mut line = String::new();
            if matches!(std::io::stdin().read_line(&mut line), Ok(n) if n > 0) {
                let _ = go_tx.send(());
            }
        });
        go_rx
    });
    let mut go = !wait;

    // Take lists until they stop coming. Polled rather than awaited, so the
    // deadline holds even if the compositor never says anything at all.
    let mut start = Instant::now();
    let mut latest: Option<(Vec<_>, Instant)> = None;
    let windows = loop {
        match update_rx.try_recv() {
            Ok(WaylandUpdate::Windows(windows)) => {
                latest = Some((windows, Instant::now()));
                continue;
            }
            Ok(WaylandUpdate::Init(_)) => continue,
            Ok(WaylandUpdate::Finished) | Err(TryRecvError::Closed) => {
                return Err("the wayland connection closed".into());
            }
            Err(TryRecvError::Empty) => {}
        }
        if !go && let Some(go_rx) = &go_rx {
            match go_rx.try_recv() {
                Ok(()) => {
                    go = true;
                    // The deadline counts from the go-ahead: the wait before
                    // it is the pointer resting, not the compositor stalling.
                    start = Instant::now();
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    drop(calloop_tx);
                    let _ = handler.join();
                    return Ok(());
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        if !go {
            std::thread::sleep(Duration::from_millis(2));
            continue;
        }
        if let Some((_, at)) = &latest
            && at.elapsed() >= SETTLE
        {
            break latest.take().map(|(windows, _)| windows).unwrap_or_default();
        }
        if start.elapsed() >= LIST_TIMEOUT {
            match latest.take() {
                Some((windows, _)) => break windows,
                None => return Err("timed out waiting for the compositor's window list".into()),
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    // Stop listening. The Wayland thread keeps reporting changes (including
    // the ones this toggle is about to cause); with nobody reading, a full
    // channel used to block it for good — before it ever ran the minimize
    // requests — and the process hung. A closed channel makes those sends fail
    // at once instead.
    drop(update_rx);

    let scoped = crate::to_windows(&windows);
    let scope = scope::resolve(
        config::per_output(),
        output.as_deref(),
        scope::focused_output(&scoped),
    );
    tracing::debug!("show-desktop toggle in {scope:?}");
    let mut state = state_file::load();
    let steps = state.toggle(&scope, &scoped);
    if dry_run {
        for window in &scoped {
            println!(
                "{}\tminimized={}\tfocused={}\toutputs={}",
                window.id,
                window.minimized,
                window.activated,
                window.outputs.join(",")
            );
        }
        println!("scope: {}", scope.key());
        for step in &steps {
            println!("would {step:?}");
        }
        drop(calloop_tx);
        let _ = handler.join();
        return Ok(());
    }
    state_file::save(&state);

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
