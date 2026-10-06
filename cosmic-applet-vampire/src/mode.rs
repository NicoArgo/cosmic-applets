// SPDX-License-Identifier: GPL-3.0-only

//! Vampire mode: the computer sleeps only when you tell it to.
//!
//! Two things put a laptop to sleep without being asked, and the mode turns
//! off both:
//!
//! - **Closing the lid.** logind handles the lid switch; a `handle-lid-switch`
//!   block inhibitor makes it leave the lid alone. The inhibitor is held by a
//!   systemd *user* unit, so it survives the panel restarting, and enabling the
//!   unit keeps the mode across reboots.
//! - **Idle suspend.** cosmic-idle runs `systemctl suspend` after the timeouts
//!   in `com.system76.CosmicIdle`. Those run as you, and logind does not hold
//!   your own inhibitors against you — so the timeouts themselves are set to
//!   `None` while the mode is on, and put back as they were when it ends.
//!
//! Suspending on purpose (the power menu, `systemctl suspend`) is untouched:
//! it's the same "your own inhibitor" rule that makes idle suspend need
//! handling above. That is the point of the mode.
//!
//! ## For a while
//!
//! "Awake for 1 h" is the mode plus a deadline: a wall-clock Unix time in
//! `~/.local/state/pop-flow/vampire-until`. A second user unit,
//! [`WATCH_UNIT`], runs this binary's watcher alongside the inhibitor and
//! turns the mode off once the deadline passes (see [`crate::watch`]). Why a
//! file and a watcher rather than a transient `systemd-run --on-active=` timer:
//!
//! - **Reboots and logouts.** A transient timer dies with the user manager,
//!   while the mode is *enabled* and comes back on its own — the computer
//!   would wake up in permanent vampire mode, the opposite of what was asked.
//!   The file stays; the watcher starts with the mode, finds the deadline
//!   past, and ends it.
//! - **Time asleep.** `--on-active` counts on the monotonic clock, which stops
//!   while suspended; "awake for 1 h" means until a time on the clock, even if
//!   the computer was put to sleep by hand in between.
//! - **One place to read.** The panel and `--status` show the time left
//!   straight from the file; nothing has to be asked of systemd.
//!
//! The watcher is its own unit, bound to the inhibitor's, rather than a
//! wrapper around `systemd-inhibit`: a bug in it can stop the reminders or the
//! countdown, never release the lid.

use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const UNIT: &str = "pop-flow-vampire.service";
pub const WATCH_UNIT: &str = "pop-flow-vampire-watch.service";
/// Where install.sh puts this binary; the watcher unit runs it from there.
const INSTALLED_BIN: &str = "/usr/local/bin/cosmic-applet-vampire";
const IDLE_KEYS: [&str; 2] = ["suspend_on_ac_time", "suspend_on_battery_time"];
/// Marks a key that had no file of its own before the mode — restoring it
/// means deleting ours, so COSMIC's built-in default applies again.
const ABSENT: &str = ".absent";

pub fn unit_text() -> String {
    r#"[Unit]
Description=POP Flow — modo vampiro: fechar a tampa não suspende
PartOf=graphical-session.target
After=graphical-session.target
Wants=pop-flow-vampire-watch.service

[Service]
ExecStart=/usr/bin/systemd-inhibit --what=handle-lid-switch --who="POP Flow" --why="Modo vampiro: só dorme quando você manda" --mode=block /usr/bin/sleep infinity
Restart=on-failure
RestartSec=2

[Install]
WantedBy=graphical-session.target
"#
    .to_string()
}

/// The watcher: ends a temporary mode on time and says when the lid closes
/// (see [`crate::watch`]). Bound to the inhibitor's unit
/// (pulled in by its `Wants=`, stopped with it), and never enabled itself.
pub fn watch_unit_text() -> String {
    format!(
        r#"[Unit]
Description=POP Flow — modo vampiro: prazo do modo temporário e aviso de tampa fechada
BindsTo={UNIT}
After={UNIT}

[Service]
ExecStart={INSTALLED_BIN} --watch
Restart=on-failure
RestartSec=5
"#
    )
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"))
}

fn state_home() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local/state"))
}

fn unit_dir() -> PathBuf {
    config_home().join("systemd/user")
}

pub fn deadline_path() -> PathBuf {
    state_home().join("pop-flow/vampire-until")
}

pub fn idle_dir() -> PathBuf {
    config_home().join("cosmic/com.system76.CosmicIdle/v1")
}

pub fn backup_dir() -> PathBuf {
    state_home().join("pop-flow/vampire-idle")
}

/// Whether the mode is on. Read from the unit's enable link rather than by
/// asking systemd, because the panel asks every few seconds and this is a
/// `stat` instead of a process. On and enabled always move together here.
pub fn is_on() -> bool {
    config_home()
        .join("systemd/user/graphical-session.target.wants")
        .join(UNIT)
        .exists()
}

fn systemctl(args: &[&str]) -> Result<(), String> {
    let out = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .output()
        .map_err(|err| format!("systemctl: {err}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "systemctl --user {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// Write `text` to `path` unless it is already there; true when it changed.
fn write_if_changed(path: &Path, text: &str) -> Result<bool, String> {
    if fs::read_to_string(path).ok().as_deref() == Some(text) {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    }
    fs::write(path, text).map_err(|err| format!("{}: {err}", path.display()))?;
    Ok(true)
}

/// Units written, mode on, idle held — whatever the deadline says.
fn enable() -> Result<(), String> {
    let dir = unit_dir();
    let main = write_if_changed(&dir.join(UNIT), &unit_text())?;
    let watch = write_if_changed(&dir.join(WATCH_UNIT), &watch_unit_text())?;
    if main || watch {
        systemctl(&["daemon-reload"])?;
    }
    systemctl(&["enable", "--now", UNIT])?;
    // Explicitly too: an inhibitor unit already running from before the
    // watcher existed doesn't pull it in again.
    systemctl(&["start", WATCH_UNIT])?;
    hold_idle(&idle_dir(), &backup_dir()).map_err(|err| format!("idle settings: {err}"))
}

/// `wait: false` is for the watcher, which is stopped by this very call and
/// shouldn't sit waiting for its own end.
fn disable(wait: bool) -> Result<(), String> {
    // Idle first: if systemd fails below, the computer can at least sleep
    // on idle again, which is the safer half to get back.
    release_idle(&idle_dir(), &backup_dir()).map_err(|err| format!("idle settings: {err}"))?;
    if wait {
        systemctl(&["disable", "--now", UNIT])
    } else {
        systemctl(&["disable", "--now", "--no-block", UNIT])
    }
}

/// On for good, or off. Either way a pending deadline is dropped: on means
/// until told otherwise.
pub fn set(on: bool) -> Result<(), String> {
    clear_deadline()?;
    if on { enable() } else { disable(true) }
}

/// On for `minutes`, then back to sleep mode on its own. While already on
/// (for good or for a while) this replaces the deadline.
pub fn set_for(minutes: u32) -> Result<(), String> {
    if minutes == 0 {
        return Err("--for needs a number of minutes above 0".into());
    }
    // Deadline first: if enabling fails half-way, what's left must not be a
    // mode with no end.
    write_deadline(deadline_after(SystemTime::now(), minutes))?;
    enable().inspect_err(|_| {
        let _ = clear_deadline();
    })
}

/// Rewrite the units if this binary's version of them differs, when the mode
/// is on — for the installer, so an upgrade takes effect without touching
/// the mode or its deadline.
pub fn refresh() -> Result<bool, String> {
    let on = is_on();
    if on {
        enable()?;
    }
    Ok(on)
}

/// The watcher's job: end the mode if its deadline has passed. True when it
/// did. The deadline goes only after systemd took the request, so a failure
/// is tried again on the next look.
pub fn expire_if_due(now: SystemTime) -> Result<bool, String> {
    match deadline() {
        Some(at) if at <= now => {
            disable(false)?;
            clear_deadline()?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Flip the mode; returns the new one.
pub fn toggle() -> Result<bool, String> {
    let on = !is_on();
    set(on)?;
    Ok(on)
}

/// When a temporary mode ends; `None` when there is no deadline (off, or on
/// for good).
pub fn deadline() -> Option<SystemTime> {
    parse_deadline(&fs::read_to_string(deadline_path()).ok()?)
}

/// A deadline file holds whole seconds since the Unix epoch.
pub fn parse_deadline(text: &str) -> Option<SystemTime> {
    let secs: u64 = text.trim().parse().ok()?;
    UNIX_EPOCH.checked_add(Duration::from_secs(secs))
}

pub fn deadline_after(now: SystemTime, minutes: u32) -> SystemTime {
    now + Duration::from_secs(u64::from(minutes) * 60)
}

/// Time left before `deadline`; zero once it has passed.
pub fn remaining(deadline: SystemTime, now: SystemTime) -> Duration {
    deadline.duration_since(now).unwrap_or_default()
}

/// "1 h 20 min", "3 h", "45 min" — rounded up, so the last minute still
/// reads "1 min" and never "0 min" while the mode is on.
pub fn format_remaining(left: Duration) -> String {
    let minutes = left.as_secs().div_ceil(60).max(1);
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

fn write_deadline(at: SystemTime) -> Result<(), String> {
    let path = deadline_path();
    let secs = at.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    }
    // Through a temporary file: the watcher reads this every few seconds and
    // must never see it half-written.
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, format!("{secs}\n"))
        .and_then(|()| fs::rename(&tmp, &path))
        .map_err(|err| format!("{}: {err}", path.display()))
}

fn clear_deadline() -> Result<(), String> {
    let path = deadline_path();
    match fs::remove_file(&path) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => {
            Err(format!("{}: {err}", path.display()))
        }
        _ => Ok(()),
    }
}

/// Turn idle suspend off, remembering what it was. A second call while the
/// mode is already on keeps the first backup — that one holds the user's
/// real settings, the files now hold ours.
pub fn hold_idle(idle: &Path, backup: &Path) -> io::Result<()> {
    if !backup.exists() {
        fs::create_dir_all(backup)?;
        for key in IDLE_KEYS {
            match fs::read(idle.join(key)) {
                Ok(data) => fs::write(backup.join(key), data)?,
                Err(err) if err.kind() == io::ErrorKind::NotFound => {
                    fs::write(backup.join(format!("{key}{ABSENT}")), b"")?
                }
                Err(err) => return Err(err),
            }
        }
    }
    fs::create_dir_all(idle)?;
    for key in IDLE_KEYS {
        fs::write(idle.join(key), "None")?;
    }
    Ok(())
}

/// Put idle suspend back as it was before [`hold_idle`]. Without a backup
/// there is nothing of ours to undo, and the files are left alone.
pub fn release_idle(idle: &Path, backup: &Path) -> io::Result<()> {
    if !backup.exists() {
        return Ok(());
    }
    for key in IDLE_KEYS {
        let saved = backup.join(key);
        if saved.exists() {
            fs::write(idle.join(key), fs::read(&saved)?)?;
        } else if backup.join(format!("{key}{ABSENT}")).exists() {
            match fs::remove_file(idle.join(key)) {
                Err(err) if err.kind() != io::ErrorKind::NotFound => return Err(err),
                _ => {}
            }
        }
    }
    fs::remove_dir_all(backup)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vampire-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn idle_round_trip_restores_set_and_absent_keys() {
        let root = scratch("round-trip");
        let (idle, backup) = (root.join("idle"), root.join("backup"));
        fs::create_dir_all(&idle).unwrap();
        fs::write(idle.join("suspend_on_ac_time"), "Some(1800000)").unwrap();

        hold_idle(&idle, &backup).unwrap();
        for key in IDLE_KEYS {
            assert_eq!(fs::read_to_string(idle.join(key)).unwrap(), "None");
        }

        release_idle(&idle, &backup).unwrap();
        assert_eq!(
            fs::read_to_string(idle.join("suspend_on_ac_time")).unwrap(),
            "Some(1800000)"
        );
        // It had no file before; it has none after.
        assert!(!idle.join("suspend_on_battery_time").exists());
        assert!(!backup.exists());
    }

    #[test]
    fn holding_twice_keeps_the_first_backup() {
        let root = scratch("twice");
        let (idle, backup) = (root.join("idle"), root.join("backup"));
        fs::create_dir_all(&idle).unwrap();
        fs::write(idle.join("suspend_on_ac_time"), "Some(60000)").unwrap();

        hold_idle(&idle, &backup).unwrap();
        hold_idle(&idle, &backup).unwrap();
        release_idle(&idle, &backup).unwrap();

        assert_eq!(
            fs::read_to_string(idle.join("suspend_on_ac_time")).unwrap(),
            "Some(60000)"
        );
    }

    #[test]
    fn deadline_round_trips_through_its_file_format() {
        let now = UNIX_EPOCH + Duration::from_secs(1_790_000_000);
        let at = deadline_after(now, 60);
        let secs = at.duration_since(UNIX_EPOCH).unwrap().as_secs();
        assert_eq!(secs, 1_790_003_600);
        assert_eq!(parse_deadline(&format!("{secs}\n")), Some(at));
        assert_eq!(parse_deadline("soon"), None);
        assert_eq!(parse_deadline(""), None);
    }

    #[test]
    fn remaining_counts_down_to_zero_and_stays_there() {
        let now = UNIX_EPOCH + Duration::from_secs(1_000_000);
        let at = deadline_after(now, 180);
        assert_eq!(remaining(at, now), Duration::from_secs(3 * 3600));
        assert_eq!(remaining(at, at), Duration::ZERO);
        assert_eq!(remaining(at, at + Duration::from_secs(5)), Duration::ZERO);
    }

    #[test]
    fn remaining_reads_in_hours_and_whole_minutes() {
        let secs = Duration::from_secs;
        assert_eq!(format_remaining(secs(3 * 3600)), "3 h");
        assert_eq!(format_remaining(secs(3600 + 20 * 60)), "1 h 20 min");
        // Rounded up: 59 min 30 s still has an hour's last minute to go.
        assert_eq!(format_remaining(secs(59 * 60 + 30)), "1 h");
        assert_eq!(format_remaining(secs(45 * 60)), "45 min");
        assert_eq!(format_remaining(secs(10)), "1 min");
        assert_eq!(format_remaining(Duration::ZERO), "1 min");
    }

    #[test]
    fn releasing_without_a_backup_changes_nothing() {
        let root = scratch("no-backup");
        let (idle, backup) = (root.join("idle"), root.join("backup"));
        fs::create_dir_all(&idle).unwrap();
        fs::write(idle.join("suspend_on_ac_time"), "Some(5)").unwrap();

        release_idle(&idle, &backup).unwrap();

        assert_eq!(fs::read_to_string(idle.join("suspend_on_ac_time")).unwrap(), "Some(5)");
    }
}
