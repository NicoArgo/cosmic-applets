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
//!
//! Since "show the desktop" can act on one screen, the file holds one set per
//! [`Scope`] — see [`crate::scope`] for how they interact.

use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};

use crate::scope::{HiddenSets, Scope};

const FILE_NAME: &str = "pop-flow-show-desktop";

/// First line of the per-scope format. A file without it is the old single
/// set, from before each screen had its own — read as one "all screens" set,
/// which is exactly what it was.
const HEADER: &str = "# pop-flow-show-desktop v2";

/// Path of the state file for this session, if there is a runtime directory.
///
/// Without `XDG_RUNTIME_DIR` there is nowhere session-scoped to write, and
/// `/tmp` would survive logout and hand the next session a stale set. Better to
/// have no memory than a wrong one, so this returns `None` and callers fall back
/// to "nothing is put away".
pub fn path() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(|dir| Path::new(&dir).join(FILE_NAME))
}

/// Read the remembered sets, or empty ones if the file is missing or unreadable.
pub fn load() -> HiddenSets {
    let Some(path) = path() else {
        return HiddenSets::new();
    };
    match std::fs::read_to_string(&path) {
        Ok(contents) => parse(&contents),
        Err(err) if err.kind() == io::ErrorKind::NotFound => HiddenSets::new(),
        Err(err) => {
            tracing::warn!("could not read {}: {err}", path.display());
            HiddenSets::new()
        }
    }
}

/// Replace the remembered sets. Nothing remembered removes the file rather than
/// leaving an empty one behind.
pub fn save(sets: &HiddenSets) {
    let Some(path) = path() else {
        return;
    };
    if sets.is_empty() {
        if let Err(err) = std::fs::remove_file(&path)
            && err.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!("could not remove {}: {err}", path.display());
        }
        return;
    }
    // Written beside the file and renamed over it, because the whole point of
    // this file is that several processes share it: the panel buttons, the
    // corners and the `--toggle` of a shortcut. `write` truncates in place, so
    // a reader that arrives mid-write would see half a set and put back half
    // the windows. A rename is atomic on the same filesystem — a reader sees
    // the old sets or the new ones, never torn ones.
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    if let Err(err) = std::fs::write(&tmp, encode(sets)) {
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

/// A header, then one `scope<TAB>identifier` per line. Toplevel identifiers
/// are opaque strings from the compositor and output names are connector
/// names; neither carries tabs or newlines, so this needs no escaping — and it
/// stays readable when someone goes looking for why the button is confused.
fn encode(sets: &HiddenSets) -> String {
    let mut out = String::from(HEADER);
    out.push('\n');
    for (scope, ids) in sets.sets() {
        let key = scope.key();
        for id in ids {
            out.push_str(&key);
            out.push('\t');
            out.push_str(id);
            out.push('\n');
        }
    }
    out
}

fn parse(contents: &str) -> HiddenSets {
    let mut lines = contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .peekable();
    let mut sets: BTreeMap<Scope, Vec<String>> = BTreeMap::new();
    let mut add = |scope: Scope, id: &str| {
        let ids = sets.entry(scope).or_default();
        if !ids.iter().any(|kept| kept == id) {
            ids.push(id.to_string());
        }
    };

    if lines.peek() == Some(&HEADER) {
        lines.next();
        for line in lines {
            // A line that doesn't parse is dropped, not guessed at: a wrong
            // guess would restore a window into the wrong screen's trip back.
            if let Some((key, id)) = line.split_once('\t')
                && let Some(scope) = Scope::from_key(key.trim())
                && !id.trim().is_empty()
            {
                add(scope, id.trim());
            }
        }
    } else {
        // The old format: one identifier per line, put away from every screen.
        for id in lines {
            add(Scope::All, id);
        }
    }
    HiddenSets::from_sets(sets)
}
#[cfg(test)]
mod tests {
    use super::*;

    fn sets(entries: &[(Scope, &[&str])]) -> HiddenSets {
        HiddenSets::from_sets(
            entries
                .iter()
                .map(|(scope, ids)| (scope.clone(), ids.iter().map(|id| id.to_string()).collect()))
                .collect(),
        )
    }

    #[test]
    fn written_sets_read_back_the_same() {
        let written = sets(&[
            (Scope::All, &["toplevel-1"]),
            (Scope::Output("eDP-1".into()), &["toplevel-2", "toplevel-3"]),
            (Scope::Output("HDMI-A-2".into()), &["toplevel-4"]),
        ]);
        assert_eq!(parse(&encode(&written)), written);
    }

    #[test]
    fn the_old_single_set_becomes_an_all_screens_set() {
        // A file written by the previous version, still in the runtime dir
        // after the update: what it put away came back with an all-screens
        // press, and that is still how it comes back.
        assert_eq!(
            parse("toplevel-1\ntoplevel-2\n"),
            sets(&[(Scope::All, &["toplevel-1", "toplevel-2"])])
        );
    }

    #[test]
    fn a_missing_or_empty_file_means_nothing_is_put_away() {
        assert!(parse("").is_empty());
        assert!(parse("\n\n").is_empty());
        assert!(parse(&format!("{HEADER}\n")).is_empty());
    }

    #[test]
    fn lines_that_do_not_parse_are_dropped() {
        let contents = format!(
            "{HEADER}\nall\ttoplevel-1\nnonsense\nweird\ttoplevel-2\noutput:\ttoplevel-3\noutput:eDP-1\t \n"
        );
        assert_eq!(parse(&contents), sets(&[(Scope::All, &["toplevel-1"])]));
    }

    #[test]
    fn a_round_trip_through_the_real_file_leaves_nothing_behind() {
        // The write goes through a temporary and a rename, so this checks both
        // halves: that the sets survive, and that the temporary does not.
        let dir = std::env::temp_dir().join("pop-flow-show-desktop-test");
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: cargo runs tests on several threads, so what makes this sound
        // is that no other test in this crate reads XDG_RUNTIME_DIR — the
        // others are pure logic over in-memory sets, and the config tests take
        // their path as a parameter. A test that starts reading it has to take
        // a path parameter instead.
        unsafe { std::env::set_var("XDG_RUNTIME_DIR", &dir) };

        let written = sets(&[
            (Scope::All, &["toplevel-1"]),
            (Scope::Output("eDP-1".into()), &["toplevel-2"]),
        ]);
        save(&written);
        assert_eq!(load(), written);
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(
            leftovers,
            vec![std::ffi::OsString::from(FILE_NAME)],
            "the temporary must be renamed away, not left for someone to find"
        );

        // A file in the old format is migrated on the next save.
        std::fs::write(path().unwrap(), "toplevel-9\n").unwrap();
        let migrated = load();
        assert_eq!(migrated, sets(&[(Scope::All, &["toplevel-9"])]));
        save(&migrated);
        assert!(std::fs::read_to_string(path().unwrap()).unwrap().starts_with(HEADER));

        // And emptying it removes the file rather than leaving an empty one.
        save(&HiddenSets::new());
        assert!(load().is_empty());
        assert!(!path().unwrap().exists());

        std::fs::remove_dir_all(&dir).ok();
    }
}
