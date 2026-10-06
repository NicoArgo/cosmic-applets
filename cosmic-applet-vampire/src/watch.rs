// SPDX-License-Identifier: GPL-3.0-only

//! `--watch`: what runs in [`mode::WATCH_UNIT`] while the mode is on, panel
//! or no panel. Two jobs:
//!
//! - End a temporary mode when its deadline passes — the "why a file" is in
//!   [`mode`]'s docs.
//! - Say so when the lid closes: the computer stays awake with the lid shut,
//!   which is the point, but also how a laptop ends up hot in a backpack. One
//!   notification per close, none for a lid already shut when the mode
//!   starts (a docked laptop would get one every time).
//!
//! It looks every [`POLL`] rather than sleeping until something happens: the
//! deadline can move (another "awake for 3 h" while on), and a sleep across a
//! suspend wakes on the monotonic clock, late by however long the computer
//! slept. Once it polls for that, the lid is one more small file read on the
//! same tick — cheaper to keep than a D-Bus client and an async runtime for
//! logind's `LidClosed` signal.

use crate::{fl, mode};
use std::{
    process::{Command, Stdio},
    time::{Duration, SystemTime},
};

const POLL: Duration = Duration::from_secs(2);
const ACPI_LID: &str = "/proc/acpi/button/lid";
const ICON: &str = "com.popflow.CosmicAppletVampire";

pub fn run() -> Result<(), String> {
    let mut lid = LidReminder::default();
    loop {
        match mode::expire_if_due(SystemTime::now()) {
            // Stopping the mode stops this unit too; nothing left to do.
            Ok(true) => return Ok(()),
            Ok(false) => {}
            // Say so and try again next time round: the deadline is still
            // there, so the mode can't outlive it by more than a failure.
            Err(err) => eprintln!("ending vampire mode: {err}"),
        }
        if lid.observe(lid_closed()) {
            notify(&fl!("lid-closed"));
        }
        std::thread::sleep(POLL);
    }
}

/// Fires on the open → closed edge only.
#[derive(Default)]
pub struct LidReminder {
    /// The last state actually read; `None` until the first.
    last: Option<bool>,
}

impl LidReminder {
    /// Feed the lid state (`None` = couldn't tell); true when this reading
    /// is a fresh close, worth a notification. An unreadable moment is
    /// skipped rather than counted as open, so it can't make one close look
    /// like two.
    pub fn observe(&mut self, closed: Option<bool>) -> bool {
        let Some(closed) = closed else {
            return false;
        };
        let fire = closed && self.last == Some(false);
        self.last = Some(closed);
        fire
    }
}

/// The ACPI lid button, which every laptop with a lid we've seen has; logind
/// as the fallback (it also knows lids that are only input switches), at the
/// cost of a process per look — only on machines without the ACPI file.
fn lid_closed() -> Option<bool> {
    let acpi = std::fs::read_dir(ACPI_LID).ok().and_then(|dirs| {
        dirs.flatten()
            .find_map(|dir| std::fs::read_to_string(dir.path().join("state")).ok())
    });
    match acpi {
        Some(state) => parse_acpi_lid(&state),
        None => {
            let out = Command::new("busctl")
                .args([
                    "get-property",
                    "org.freedesktop.login1",
                    "/org/freedesktop/login1",
                    "org.freedesktop.login1.Manager",
                    "LidClosed",
                ])
                .stderr(Stdio::null())
                .output()
                .ok()?;
            parse_logind_lid(&String::from_utf8_lossy(&out.stdout))
        }
    }
}

/// `state:      closed` (or `open`).
pub fn parse_acpi_lid(text: &str) -> Option<bool> {
    match text.split_once(':')?.1.trim() {
        "closed" => Some(true),
        "open" => Some(false),
        _ => None,
    }
}

/// `busctl get-property` output: `b true`.
pub fn parse_logind_lid(text: &str) -> Option<bool> {
    match text.trim() {
        "b true" => Some(true),
        "b false" => Some(false),
        _ => None,
    }
}

/// notify-send when there is one; otherwise straight to the notification
/// daemon over the session bus, through busctl, which systemd always brings.
fn notify(summary: &str) {
    let sent = Command::new("notify-send")
        .args(["--app-name=POP Flow", &format!("--icon={ICON}"), summary])
        .status()
        .is_ok_and(|status| status.success());
    if sent {
        return;
    }
    let status = Command::new("busctl")
        .args([
            "--user",
            "call",
            // So the trailing -1 (no expiry preference) isn't read as a flag.
            "--",
            "org.freedesktop.Notifications",
            "/org/freedesktop/Notifications",
            "org.freedesktop.Notifications",
            "Notify",
            "susssasa{sv}i",
            "POP Flow",
            "0",
            ICON,
            summary,
            "",
            "0",
            "0",
            "-1",
        ])
        .stdout(Stdio::null())
        .status();
    if !status.is_ok_and(|status| status.success()) {
        eprintln!("lid closed, but no notification could be sent");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_notification_per_close() {
        let mut lid = LidReminder::default();
        let seen: Vec<bool> = [false, true, true, true, false, true]
            .into_iter()
            .map(|closed| lid.observe(Some(closed)))
            .collect();
        assert_eq!(seen, [false, true, false, false, false, true]);
    }

    #[test]
    fn a_lid_already_closed_at_start_is_not_news() {
        let mut lid = LidReminder::default();
        assert!(!lid.observe(Some(true)));
        assert!(!lid.observe(Some(true)));
        assert!(!lid.observe(Some(false)));
        assert!(lid.observe(Some(true)));
    }

    #[test]
    fn unreadable_moments_neither_fire_nor_reset() {
        let mut lid = LidReminder::default();
        lid.observe(Some(false));
        assert!(lid.observe(Some(true)));
        assert!(!lid.observe(None));
        assert!(!lid.observe(Some(true)));
    }

    #[test]
    fn reads_both_lid_sources() {
        assert_eq!(parse_acpi_lid("state:      closed\n"), Some(true));
        assert_eq!(parse_acpi_lid("state:      open\n"), Some(false));
        assert_eq!(parse_acpi_lid("garbage"), None);
        assert_eq!(parse_logind_lid("b true\n"), Some(true));
        assert_eq!(parse_logind_lid("b false\n"), Some(false));
        assert_eq!(parse_logind_lid(""), None);
    }
}
