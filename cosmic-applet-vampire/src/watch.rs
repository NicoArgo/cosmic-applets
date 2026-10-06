// SPDX-License-Identifier: GPL-3.0-only

//! `--watch`: what runs in [`mode::WATCH_UNIT`] while the mode is on. It
//! ends a temporary mode when its deadline passes — the "why a file" is in
//! [`mode`]'s docs.
//!
//! It looks every [`POLL`] rather than sleeping until the deadline: the
//! deadline can move (another "awake for 3 h" while on), and a sleep across a
//! suspend wakes on the monotonic clock, late by however long the computer
//! slept. A file read every few seconds costs nothing.

use crate::mode;
use std::time::{Duration, SystemTime};

const POLL: Duration = Duration::from_secs(2);

pub fn run() -> Result<(), String> {
    loop {
        match mode::expire_if_due(SystemTime::now()) {
            // Stopping the mode stops this unit too; nothing left to do.
            Ok(true) => return Ok(()),
            Ok(false) => {}
            // Say so and try again next time round: the deadline is still
            // there, so the mode can't outlive it by more than a failure.
            Err(err) => eprintln!("ending vampire mode: {err}"),
        }
        std::thread::sleep(POLL);
    }
}
