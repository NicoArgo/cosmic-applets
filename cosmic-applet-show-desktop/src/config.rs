// SPDX-License-Identifier: GPL-3.0-only

//! The one setting: does "show the desktop" act on the screen it was asked
//! from, or on every screen?
//!
//! Stored where cosmic-config keeps it, in the shape cosmic-config writes, so
//! COSMIC Settings → Displays can show and flip it with its own machinery:
//!
//! ```text
//! ~/.config/cosmic/com.popflow.ShowDesktop/v1/per_output   →   true | false
//! ```
//!
//! `true` (the default, also when the file is missing or unreadable) means only
//! the windows on the screen where it was triggered; `false` means every
//! screen, the behavior from before there was a choice.
//!
//! Plain std on purpose: this file is shared, through `#[path]`, with
//! `cosmic-show-desktop-corner`, which deliberately carries no libcosmic.

#![allow(dead_code)] // each of the two crates uses a different half

use std::{
    io,
    path::{Path, PathBuf},
};

pub const CONFIG_ID: &str = "com.popflow.ShowDesktop";
pub const CONFIG_VERSION: u64 = 1;
pub const PER_OUTPUT_KEY: &str = "per_output";
pub const PER_OUTPUT_DEFAULT: bool = true;

/// `$XDG_CONFIG_HOME/cosmic/com.popflow.ShowDesktop/v1/per_output`, falling
/// back to `~/.config` like cosmic-config does.
pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(path_in(&base))
}

/// The key's file under a given config home.
pub fn path_in(config_home: &Path) -> PathBuf {
    config_home
        .join("cosmic")
        .join(CONFIG_ID)
        .join(format!("v{CONFIG_VERSION}"))
        .join(PER_OUTPUT_KEY)
}

/// The value as RON, the way cosmic-config serializes a bool.
pub fn parse(contents: &str) -> Option<bool> {
    match contents.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

pub fn encode(per_output: bool) -> &'static str {
    if per_output { "true" } else { "false" }
}

/// The current setting; the default when there is none or it can't be read.
pub fn per_output() -> bool {
    path().map_or(PER_OUTPUT_DEFAULT, |path| per_output_at(&path))
}

pub fn per_output_at(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|contents| parse(&contents))
        .unwrap_or(PER_OUTPUT_DEFAULT)
}

pub fn set_per_output(per_output: bool) -> io::Result<()> {
    let path = path().ok_or_else(|| io::Error::other("neither XDG_CONFIG_HOME nor HOME is set"))?;
    set_per_output_at(&path, per_output)
}

/// Write beside the key and rename over it, as cosmic-config does: whoever
/// watches the directory (COSMIC Settings, the corner) sees the old value or
/// the new one, never an empty file mid-write. The temporary starts with a dot
/// so a watcher listing keys doesn't mistake it for one.
pub fn set_per_output_at(path: &Path, per_output: bool) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::other("config path has no parent"))?;
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{PER_OUTPUT_KEY}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, encode(per_output))?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_cosmic_config_writes() {
        assert_eq!(parse("true"), Some(true));
        assert_eq!(parse("false\n"), Some(false));
        assert_eq!(parse("  true  "), Some(true));
        assert_eq!(parse(""), None);
        assert_eq!(parse("yes"), None);
    }

    #[test]
    fn lives_under_the_shared_contract_path() {
        assert_eq!(
            path_in(Path::new("/x")),
            PathBuf::from("/x/cosmic/com.popflow.ShowDesktop/v1/per_output")
        );
    }

    #[test]
    fn missing_or_garbled_means_per_screen() {
        let dir = std::env::temp_dir().join(format!(
            "pop-flow-show-desktop-config-test-{}-{}",
            std::process::id(),
            line!()
        ));
        let path = path_in(&dir);
        assert!(per_output_at(&path), "no file: the default");

        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "maybe").unwrap();
        assert!(per_output_at(&path), "unreadable value: the default");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_write_reads_back_and_leaves_no_temporary() {
        let dir = std::env::temp_dir().join(format!(
            "pop-flow-show-desktop-config-test-{}-{}",
            std::process::id(),
            line!()
        ));
        let path = path_in(&dir);

        set_per_output_at(&path, false).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "false");
        assert!(!per_output_at(&path));
        set_per_output_at(&path, true).unwrap();
        assert!(per_output_at(&path));

        let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(leftovers, vec![std::ffi::OsString::from(PER_OUTPUT_KEY)]);

        std::fs::remove_dir_all(&dir).ok();
    }
}
