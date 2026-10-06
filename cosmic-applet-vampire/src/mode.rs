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

use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

pub const UNIT: &str = "pop-flow-vampire.service";
const IDLE_KEYS: [&str; 2] = ["suspend_on_ac_time", "suspend_on_battery_time"];
/// Marks a key that had no file of its own before the mode — restoring it
/// means deleting ours, so COSMIC's built-in default applies again.
const ABSENT: &str = ".absent";

pub fn unit_text() -> String {
    r#"[Unit]
Description=POP Flow — modo vampiro: fechar a tampa não suspende
PartOf=graphical-session.target
After=graphical-session.target

[Service]
ExecStart=/usr/bin/systemd-inhibit --what=handle-lid-switch --who="POP Flow" --why="Modo vampiro: só dorme quando você manda" --mode=block /usr/bin/sleep infinity
Restart=on-failure
RestartSec=2

[Install]
WantedBy=graphical-session.target
"#
    .to_string()
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

fn unit_path() -> PathBuf {
    config_home().join("systemd/user").join(UNIT)
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

pub fn set(on: bool) -> Result<(), String> {
    if on {
        let path = unit_path();
        if fs::read_to_string(&path).ok().as_deref() != Some(unit_text().as_str()) {
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
            }
            fs::write(&path, unit_text()).map_err(|err| format!("{}: {err}", path.display()))?;
            systemctl(&["daemon-reload"])?;
        }
        systemctl(&["enable", "--now", UNIT])?;
        hold_idle(&idle_dir(), &backup_dir()).map_err(|err| format!("idle settings: {err}"))
    } else {
        // Idle first: if systemd fails below, the computer can at least sleep
        // on idle again, which is the safer half to get back.
        release_idle(&idle_dir(), &backup_dir()).map_err(|err| format!("idle settings: {err}"))?;
        systemctl(&["disable", "--now", UNIT])
    }
}

/// Flip the mode; returns the new one.
pub fn toggle() -> Result<bool, String> {
    let on = !is_on();
    set(on)?;
    Ok(on)
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
    fn releasing_without_a_backup_changes_nothing() {
        let root = scratch("no-backup");
        let (idle, backup) = (root.join("idle"), root.join("backup"));
        fs::create_dir_all(&idle).unwrap();
        fs::write(idle.join("suspend_on_ac_time"), "Some(5)").unwrap();

        release_idle(&idle, &backup).unwrap();

        assert_eq!(fs::read_to_string(idle.join("suspend_on_ac_time")).unwrap(), "Some(5)");
    }
}
