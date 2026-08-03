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
    if let Err(err) = std::fs::write(&path, encode(identifiers)) {
        tracing::warn!("could not write {}: {err}", path.display());
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
    fn stray_whitespace_does_not_invent_entries() {
        // A half-written file, or one someone poked at by hand, must not turn
        // into identifiers that match no window.
        assert_eq!(parse("  toplevel-1  \n\n  \n"), vec!["toplevel-1".to_string()]);
    }
}
