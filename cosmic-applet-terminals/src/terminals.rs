// SPDX-License-Identifier: GPL-3.0-only

//! Which dots the panel shows, and in which color — data in, data out, so none
//! of it needs a compositor to test.
//!
//! The window list comes from the compositor (app id + title, no pid, no
//! folder). The colors come from the terminal: each cosmic-term window keeps a
//! file in `$XDG_RUNTIME_DIR/pop-flow/terminals/<pid>` saying which title it
//! set and which folder identity its active tab wears (`src/presence.rs` in
//! the cosmic-term fork). The two meet on the title.

use std::{
    fs,
    path::{Path, PathBuf},
};

pub const TERM_APP_ID: &str = "com.system76.CosmicTerm";

/// Up to this many dots; past it, a "+N".
pub const MAX_DOTS: usize = 8;

/// A window, as far as the applet cares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window<H> {
    pub handle: H,
    pub app_id: String,
    pub title: String,
    pub minimized: bool,
    pub active: bool,
}

/// What one terminal window says about itself.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Presence {
    pub pid: u32,
    pub title: String,
    pub name: Option<String>,
    pub accent: Option<[u8; 3]>,
}

/// One dot on the panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dot<H> {
    pub handle: H,
    /// `None`: the folder has no rule, and the dot is neutral.
    pub accent: Option<[u8; 3]>,
    pub name: String,
    pub active: bool,
}

pub fn presence_dir() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    Some(Path::new(&runtime).join("pop-flow/terminals"))
}

pub fn parse_presence(pid: u32, text: &str) -> Presence {
    let mut presence = Presence {
        pid,
        ..Presence::default()
    };
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "title" => presence.title = value.to_string(),
            "name" if !value.trim().is_empty() => presence.name = Some(value.trim().to_string()),
            "accent" => presence.accent = parse_hex(value),
            _ => {}
        }
    }
    presence
}

fn parse_hex(value: &str) -> Option<[u8; 3]> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// Whether `pid` is still a cosmic-term. A terminal that crashed never removes
/// its file, and its pid may later belong to something else.
fn is_terminal(pid: u32) -> bool {
    fs::read_to_string(format!("/proc/{pid}/comm")).is_ok_and(|comm| comm.trim() == "cosmic-term")
}

/// Every live terminal's presence, oldest process first. Files of terminals
/// that died without cleaning up are removed on the way.
pub fn read_presences(dir: &Path) -> Vec<Presence> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut presences: Vec<Presence> = entries
        .flatten()
        .filter_map(|entry| {
            let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
            if !is_terminal(pid) {
                let _ = fs::remove_file(entry.path());
                return None;
            }
            let text = fs::read_to_string(entry.path()).ok()?;
            Some(parse_presence(pid, &text))
        })
        .collect();
    presences.sort_by_key(|p| p.pid);
    presences
}

/// Keeps each terminal window in the slot it got when it opened: windows that
/// closed leave, new ones join at the end, and focusing one never moves it —
/// the same rule as the Alt+Tab grid.
pub fn keep_order<H: PartialEq + Clone>(order: &mut Vec<H>, windows: &[Window<H>]) {
    let terminals = || windows.iter().filter(|w| w.app_id == TERM_APP_ID);
    order.retain(|h| terminals().any(|w| &w.handle == h));
    for w in terminals() {
        if !order.contains(&w.handle) {
            order.push(w.handle.clone());
        }
    }
}

/// The name to show for a window with no folder rule: its title minus the
/// trailing " — <app name>" the terminal adds.
fn fallback_name(title: &str) -> String {
    title
        .rsplit_once(" — ")
        .map_or(title, |(head, _)| head)
        .to_string()
}

/// One dot per terminal window, in `order`. Each presence is used once, so two
/// windows with the same title still get one file each; which one gets which
/// is by pid, oldest first. A wrong pairing can only swap two colors, and the
/// only thing a dot does is act on its own window handle.
pub fn dots<H: PartialEq + Clone>(
    order: &[H],
    windows: &[Window<H>],
    presences: &[Presence],
) -> Vec<Dot<H>> {
    let mut used = vec![false; presences.len()];
    order
        .iter()
        .filter_map(|h| windows.iter().find(|w| &w.handle == h))
        .map(|w| {
            let presence = presences
                .iter()
                .enumerate()
                .find(|(i, p)| !used[*i] && p.title == w.title)
                .map(|(i, p)| {
                    used[i] = true;
                    p
                });
            Dot {
                handle: w.handle.clone(),
                accent: presence.and_then(|p| p.accent),
                name: presence
                    .and_then(|p| p.name.clone())
                    .unwrap_or_else(|| fallback_name(&w.title)),
                active: w.active,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term(handle: u32, title: &str) -> Window<u32> {
        Window {
            handle,
            app_id: TERM_APP_ID.into(),
            title: title.into(),
            minimized: false,
            active: false,
        }
    }

    fn presence(pid: u32, title: &str, accent: Option<[u8; 3]>) -> Presence {
        Presence {
            pid,
            title: title.into(),
            name: accent.map(|_| "FLOW".into()),
            accent,
        }
    }

    #[test]
    fn parses_what_the_terminal_writes() {
        let p = parse_presence(
            7,
            "title=POP FLOW — claude — COSMIC Terminal\nname=POP FLOW\naccent=#48B9C7\n",
        );
        assert_eq!(p.title, "POP FLOW — claude — COSMIC Terminal");
        assert_eq!(p.name.as_deref(), Some("POP FLOW"));
        assert_eq!(p.accent, Some([0x48, 0xb9, 0xc7]));

        // A folder without a rule: title only.
        let bare = parse_presence(8, "title=~ — COSMIC Terminal\n");
        assert_eq!(bare.name, None);
        assert_eq!(bare.accent, None);
    }

    #[test]
    fn a_bad_color_is_no_color() {
        assert_eq!(parse_presence(1, "accent=#12345\n").accent, None);
        assert_eq!(parse_presence(1, "accent=48b9c7\n").accent, None);
        assert_eq!(parse_presence(1, "accent=#zzzzzz\n").accent, None);
        assert_eq!(parse_presence(1, "accent=#ééé\n").accent, None);
    }

    #[test]
    fn only_terminals_get_dots_and_keep_their_slot() {
        let mut windows = vec![
            term(1, "a — T"),
            Window {
                app_id: "firefox".into(),
                ..term(2, "web")
            },
            term(3, "b — T"),
        ];
        let mut order = Vec::new();
        keep_order(&mut order, &windows);
        assert_eq!(order, [1, 3]);

        // Focus moves, the list comes back in another order: slots stay.
        windows.reverse();
        windows[0].active = true;
        keep_order(&mut order, &windows);
        assert_eq!(order, [1, 3]);

        // One closes, one opens: the new one goes to the end.
        windows.retain(|w| w.handle != 1);
        windows.push(term(4, "c — T"));
        keep_order(&mut order, &windows);
        assert_eq!(order, [3, 4]);
    }

    #[test]
    fn colors_come_from_the_matching_title() {
        let windows = [term(1, "a — T"), term(2, "b — T")];
        let presences = [presence(10, "b — T", Some([1, 2, 3]))];
        let dots = dots(&[1, 2], &windows, &presences);
        assert_eq!(dots[0].accent, None);
        assert_eq!(dots[0].name, "a");
        assert_eq!(dots[1].accent, Some([1, 2, 3]));
        assert_eq!(dots[1].name, "FLOW");
    }

    #[test]
    fn same_title_twice_uses_each_file_once() {
        let windows = [term(1, "x — T"), term(2, "x — T")];
        let presences = [
            presence(10, "x — T", Some([1, 1, 1])),
            presence(11, "x — T", Some([2, 2, 2])),
        ];
        let dots = dots(&[1, 2], &windows, &presences);
        assert_eq!(dots[0].accent, Some([1, 1, 1]));
        assert_eq!(dots[1].accent, Some([2, 2, 2]));
    }

    #[test]
    fn a_stock_terminal_still_gets_a_neutral_dot() {
        let dots = dots(&[1], &[term(1, "COSMIC Terminal")], &[]);
        assert_eq!(dots[0].accent, None);
        assert_eq!(dots[0].name, "COSMIC Terminal");
    }

    #[test]
    fn a_dead_terminals_file_is_dropped() {
        let dir = std::env::temp_dir().join(format!("terminals-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        // pid 0 is never a process we can read.
        let stale = dir.join("0");
        fs::write(&stale, "title=gone\n").unwrap();
        fs::write(dir.join(".1.tmp"), "title=half\n").unwrap();
        assert!(read_presences(&dir).is_empty());
        assert!(!stale.exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
