// SPDX-License-Identifier: GPL-3.0-only

//! Where "what did I put away" is kept.
//!
//! The panel button is not the only way to show the desktop — a touchpad
//! gesture runs the same binary with `--toggle`, in a separate process that
//! shares nothing with the applet. So the remembered set cannot live in either
//! one's memory: whichever acts second would restore the wrong thing, or
//! nothing.
//!
//! It lives in a file under the runtime directory instead, which both read
//! before acting and write after. The runtime directory is per-session and
//! cleared on logout, which is exactly the lifetime this state should have —
//! window handles from a previous session mean nothing.

use std::{
    io,
    path::{Path, PathBuf},
};

const FILE_NAME: &str = "pop-flow-show-desktop";

/// Path of the state file for this session, if there is a runtime directory.
///
/// Without `XDG_RUNTIME_DIR` there is nowhere session-scoped to write, and
/// `/tmp` would survive logout and hand the next session a stale set. Better to
/// have no memory than a wrong one, so this returns `None` and callers fall back
/// to "nothing is put away".
pub fn path() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(|dir| Path::new(&dir).join(FILE_NAME))
}

/// Read the remembered set, or an empty one if it is missing or unreadable.
pub fn load() -> Vec<String> {
    let Some(path) = path() else {
        return Vec::new();
    };
    match std::fs::read_to_string(&path) {
        Ok(contents) => parse(&contents),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(err) => {
            tracing::warn!("could not read {}: {err}", path.display());
            Vec::new()
        }
    }
}

/// Replace the remembered set. An empty set removes the file rather than
/// leaving an empty one behind.
pub fn save(identifiers: &[String]) {
    let Some(path) = path() else {
        return;
    };
    if identifiers.is_empty() {
        if let Err(err) = std::fs::remove_file(&path)
            && err.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!("could not remove {}: {err}", path.display());
        }
        return;
    }
    // Written beside the file and renamed over it, because the whole point of
    // this file is that two processes share it: the panel button and the
    // `--toggle` of the gesture. `write` truncates in place, so a reader that
    // arrives mid-write would see half a set and put back half the windows.
    // A rename is atomic on the same filesystem — a reader sees the old set or
    // the new one, never a torn one.
    let tmp = path.with_extension("tmp");
    if let Err(err) = std::fs::write(&tmp, encode(identifiers)) {
        tracing::warn!("could not write {}: {err}", tmp.display());
        return;
    }
    if let Err(err) = std::fs::rename(&tmp, &path) {
        tracing::warn!("could not replace {}: {err}", path.display());
        // Leaving a stray .tmp behind would be worse than the failed write:
        // nothing ever reads it, and nothing else would clean it up.
        let _ = std::fs::remove_file(&tmp);
    }
}

/// One identifier per line. Toplevel identifiers are opaque strings from the
/// compositor and never contain newlines, so this needs no escaping — and it
/// stays readable when someone goes looking for why the button is confused.
fn encode(identifiers: &[String]) -> String {
    let mut out = identifiers.join("\n");
    out.push('\n');
    out
}

fn parse(contents: &str) -> Vec<String> {
    contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_written_set_reads_back_the_same() {
        let identifiers = vec!["toplevel-1".to_string(), "toplevel-2".to_string()];
        assert_eq!(parse(&encode(&identifiers)), identifiers);
    }

    #[test]
    fn a_missing_or_empty_file_means_nothing_is_put_away() {
        assert!(parse("").is_empty());
        assert!(parse("\n\n").is_empty());
    }

    #[test]
    fn a_round_trip_through_the_real_file_leaves_nothing_behind() {
        // The write goes through a temporary and a rename, so this checks both
        // halves: that the set survives, and that the temporary does not.
        let dir = std::env::temp_dir().join("pop-flow-show-desktop-test");
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: cargo runs tests on several threads, so what makes this sound
        // is that no other test in this crate reads XDG_RUNTIME_DIR — the
        // show_desktop ones are pure logic over an in-memory set. A test that
        // starts reading it has to take a path parameter instead.
        unsafe { std::env::set_var("XDG_RUNTIME_DIR", &dir) };

        let identifiers = vec!["toplevel-1".to_string(), "toplevel-2".to_string()];
        save(&identifiers);
        assert_eq!(load(), identifiers);
        assert!(
            !dir.join(format!("{FILE_NAME}.tmp")).exists(),
            "the temporary must be renamed away, not left for someone to find"
        );

        // And emptying it removes the file rather than leaving an empty one.
        save(&[]);
        assert!(load().is_empty());
        assert!(!path().unwrap().exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stray_whitespace_does_not_invent_entries() {
        // A half-written file, or one someone poked at by hand, must not turn
        // into identifiers that match no window.
        assert_eq!(parse("  toplevel-1  \n\n  \n"), vec!["toplevel-1".to_string()]);
    }
}
