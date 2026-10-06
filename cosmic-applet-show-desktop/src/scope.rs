// SPDX-License-Identifier: GPL-3.0-only

//! "Show the desktop" on one screen, or on all of them.
//!
//! With two monitors, putting everything away because the pointer touched the
//! laptop's corner also empties the other screen — usually not what was meant.
//! So each press has a [`Scope`]: one output by name, or every output. And the
//! remembered set is kept per scope ([`HiddenSets`]), so bringing one screen
//! back never unminimizes what was put away on another.
//!
//! How the two kinds of scope meet:
//!
//! - **Output X**, pressed while something is put away for X — either by a
//!   press on X, or by an all-screens press while the window was on X — brings
//!   back exactly those. Otherwise it puts away what is on screen on X.
//! - **All**, pressed while anything is put away anywhere, brings everything
//!   back; otherwise it puts away every window on screen, on every output.
//!
//! The inner rule is still [`ShowDesktop`]'s: restore exactly what was put
//! away, skip what closed or was brought back by hand.

use std::collections::BTreeMap;

use crate::show_desktop::{ShowDesktop, Step, Window};

/// Which windows a press acts on.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub enum Scope {
    All,
    /// An output by its connector name (`eDP-1`, `HDMI-A-2`).
    Output(String),
}

impl Scope {
    /// How the scope is written in the state file.
    pub fn key(&self) -> String {
        match self {
            Scope::All => "all".to_string(),
            Scope::Output(name) => format!("output:{name}"),
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "all" => Some(Scope::All),
            _ => key
                .strip_prefix("output:")
                .filter(|name| !name.is_empty())
                .map(|name| Scope::Output(name.to_string())),
        }
    }
}

/// The scope of a press.
///
/// - `explicit`: an output named on the command line (`--output X`), or by the
///   panel button / corner for the screen it sits on. Wins when the setting is
///   per-screen.
/// - `focused`: the output of the window that has focus — what Super+D means by
///   "this screen", since a keyboard shortcut has no screen of its own.
///
/// With the setting on "all screens", or with no screen known, it is all.
pub fn resolve(per_output: bool, explicit: Option<&str>, focused: Option<&str>) -> Scope {
    if !per_output {
        return Scope::All;
    }
    explicit
        .filter(|name| !name.is_empty())
        .or(focused.filter(|name| !name.is_empty()))
        .map_or(Scope::All, |name| Scope::Output(name.to_string()))
}

/// A window with the outputs it shows on, the one showing most of it first.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScopedWindow {
    pub id: String,
    pub minimized: bool,
    /// Focused: decides the screen for a press that comes with none.
    pub activated: bool,
    pub outputs: Vec<String>,
}

/// The output the focused window is mostly on.
pub fn focused_output(windows: &[ScopedWindow]) -> Option<&str> {
    windows
        .iter()
        .find(|window| window.activated && !window.minimized)
        .and_then(|window| window.outputs.first())
        .map(String::as_str)
}

/// How much of a window (x, y, w, h in output-local logical coordinates) an
/// output of the given logical size shows — for ordering a window's outputs.
pub fn visible_area(window: (i32, i32, i32, i32), output: (i32, i32)) -> i64 {
    let (x, y, w, h) = window;
    let left = x.max(0) as i64;
    let top = y.max(0) as i64;
    let right = (x as i64 + w as i64).min(output.0 as i64);
    let bottom = (y as i64 + h as i64).min(output.1 as i64);
    (right - left).max(0) * (bottom - top).max(0)
}

/// Remembered windows, per scope.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HiddenSets {
    sets: BTreeMap<Scope, Vec<String>>,
}

impl HiddenSets {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_sets(sets: BTreeMap<Scope, Vec<String>>) -> Self {
        let mut this = Self { sets };
        this.sets.retain(|_, ids| !ids.is_empty());
        this
    }

    pub fn sets(&self) -> &BTreeMap<Scope, Vec<String>> {
        &self.sets
    }

    pub fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }

    /// Whether the next press in this scope brings windows back.
    pub fn is_showing_desktop(&self, scope: &Scope, windows: &[ScopedWindow]) -> bool {
        !self.claimed(scope, windows).is_empty()
    }

    /// One press, in one scope. `windows` is every window on the active
    /// workspaces of every output — not only this scope's, so the windows of
    /// other scopes are recognized as still existing.
    pub fn toggle(&mut self, scope: &Scope, windows: &[ScopedWindow]) -> Vec<Step<String>> {
        let every: Vec<Window<String>> = windows.iter().map(plain).collect();
        let claimed = self.claimed(scope, windows);

        if !claimed.is_empty() {
            let mut ids: Vec<String> = Vec::new();
            for (_, id) in &claimed {
                if !ids.contains(id) {
                    ids.push(id.clone());
                }
            }
            for (owner, id) in &claimed {
                if let Some(set) = self.sets.get_mut(owner) {
                    set.retain(|kept| kept != id);
                }
            }
            self.sets.retain(|_, ids| !ids.is_empty());
            return ShowDesktop::from_hidden(ids).toggle(&every);
        }

        let outputs_known = windows.iter().any(|window| !window.outputs.is_empty());
        let in_scope: Vec<Window<String>> = windows
            .iter()
            .filter(|window| in_scope(scope, window, outputs_known))
            .map(plain)
            .collect();
        let mut state = ShowDesktop::new();
        let steps = state.toggle(&in_scope);
        if state.is_showing_desktop() {
            self.sets.insert(scope.clone(), state.hidden().to_vec());
        }
        steps
    }

    /// Forget windows that no longer exist (closed, or on a workspace that is
    /// no longer active), and scopes left with nothing.
    pub fn retain_existing(&mut self, windows: &[ScopedWindow]) {
        for ids in self.sets.values_mut() {
            ids.retain(|id| windows.iter().any(|window| &window.id == id));
        }
        self.sets.retain(|_, ids| !ids.is_empty());
    }

    /// What a press in `scope` would bring back, with the scope that holds
    /// each one.
    fn claimed(&self, scope: &Scope, windows: &[ScopedWindow]) -> Vec<(Scope, String)> {
        let entries = self
            .sets
            .iter()
            .flat_map(|(owner, ids)| ids.iter().map(move |id| (owner, id)));
        match scope {
            Scope::All => entries
                .map(|(owner, id)| (owner.clone(), id.clone()))
                .collect(),
            Scope::Output(name) => entries
                .filter(|(owner, id)| match owner {
                    Scope::Output(other) => other == name,
                    // Put away from every screen: this screen takes back the
                    // ones that are on it.
                    Scope::All => windows
                        .iter()
                        .any(|window| &&window.id == id && window.outputs.contains(name)),
                })
                .map(|(owner, id)| (owner.clone(), id.clone()))
                .collect(),
        }
    }
}

fn plain(window: &ScopedWindow) -> Window<String> {
    Window {
        id: window.id.clone(),
        minimized: window.minimized,
    }
}

fn in_scope(scope: &Scope, window: &ScopedWindow, outputs_known: bool) -> bool {
    match scope {
        Scope::All => true,
        // A compositor that reports no outputs at all would otherwise make
        // every per-screen press a no-op; acting on everything is the lesser
        // surprise.
        Scope::Output(name) => !outputs_known || window.outputs.contains(name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAPTOP: &str = "eDP-1";
    const HDMI: &str = "HDMI-A-2";

    fn win(id: &str, minimized: bool, outputs: &[&str]) -> ScopedWindow {
        ScopedWindow {
            id: id.to_string(),
            minimized,
            activated: false,
            outputs: outputs.iter().map(|o| o.to_string()).collect(),
        }
    }

    fn out(name: &str) -> Scope {
        Scope::Output(name.to_string())
    }

    fn minimize(ids: &[&str]) -> Vec<Step<String>> {
        ids.iter()
            .map(|id| Step::Minimize(id.to_string()))
            .collect()
    }

    fn unminimize(ids: &[&str]) -> Vec<Step<String>> {
        ids.iter()
            .map(|id| Step::Unminimize(id.to_string()))
            .collect()
    }

    #[test]
    fn scope_resolution() {
        // Setting off: always every screen, whatever was asked.
        assert_eq!(resolve(false, Some(HDMI), Some(LAPTOP)), Scope::All);
        // Setting on: the screen asked from wins over the focused one.
        assert_eq!(resolve(true, Some(HDMI), Some(LAPTOP)), out(HDMI));
        // Super+D: no screen of its own, so the focused window's.
        assert_eq!(resolve(true, None, Some(LAPTOP)), out(LAPTOP));
        // Nothing focused: all.
        assert_eq!(resolve(true, None, None), Scope::All);
        assert_eq!(resolve(true, Some(""), None), Scope::All);
    }

    #[test]
    fn scope_keys_round_trip() {
        for scope in [Scope::All, out(HDMI)] {
            assert_eq!(Scope::from_key(&scope.key()), Some(scope));
        }
        assert_eq!(Scope::from_key("output:"), None);
        assert_eq!(Scope::from_key("bogus"), None);
    }

    #[test]
    fn the_focused_output_is_the_focused_windows_main_one() {
        let mut focused = win("b", false, &[HDMI, LAPTOP]);
        focused.activated = true;
        let windows = vec![win("a", false, &[LAPTOP]), focused];
        assert_eq!(focused_output(&windows), Some(HDMI));
        assert_eq!(focused_output(&[win("a", false, &[LAPTOP])]), None);
    }

    #[test]
    fn visible_area_clips_to_the_output() {
        assert_eq!(visible_area((0, 0, 100, 50), (1920, 1080)), 5000);
        assert_eq!(visible_area((1900, 0, 100, 50), (1920, 1080)), 20 * 50);
        assert_eq!(visible_area((-50, -10, 100, 50), (1920, 1080)), 50 * 40);
        assert_eq!(visible_area((2000, 0, 100, 50), (1920, 1080)), 0);
    }

    #[test]
    fn a_press_on_one_screen_leaves_the_other_alone() {
        let mut sets = HiddenSets::new();
        let open = vec![
            win("a", false, &[LAPTOP]),
            win("b", false, &[HDMI]),
            win("c", false, &[LAPTOP]),
        ];
        assert_eq!(sets.toggle(&out(LAPTOP), &open), minimize(&["a", "c"]));
        assert!(sets.is_showing_desktop(&out(LAPTOP), &open));
        assert!(!sets.is_showing_desktop(&out(HDMI), &open));
    }

    #[test]
    fn restoring_one_screen_never_unminimizes_another() {
        let mut sets = HiddenSets::new();
        let open = vec![win("a", false, &[LAPTOP]), win("b", false, &[HDMI])];
        sets.toggle(&out(LAPTOP), &open);
        let now = vec![win("a", true, &[LAPTOP]), win("b", false, &[HDMI])];
        sets.toggle(&out(HDMI), &now);

        let now = vec![win("a", true, &[LAPTOP]), win("b", true, &[HDMI])];
        assert_eq!(sets.toggle(&out(HDMI), &now), unminimize(&["b"]));
        // The laptop's are still put away, and its next press brings them.
        assert!(sets.is_showing_desktop(&out(LAPTOP), &now));
        assert_eq!(sets.toggle(&out(LAPTOP), &now), unminimize(&["a"]));
        assert!(sets.is_empty());
    }

    #[test]
    fn all_screens_brings_back_everything_put_away_anywhere() {
        let mut sets = HiddenSets::new();
        let open = vec![win("a", false, &[LAPTOP]), win("b", false, &[HDMI])];
        sets.toggle(&out(LAPTOP), &open);
        assert!(sets.is_showing_desktop(&Scope::All, &open));

        let now = vec![win("a", true, &[LAPTOP]), win("b", false, &[HDMI])];
        assert_eq!(sets.toggle(&Scope::All, &now), unminimize(&["a"]));
        assert!(sets.is_empty());

        // And with nothing put away, all screens puts away everything.
        let open = vec![win("a", false, &[LAPTOP]), win("b", false, &[HDMI])];
        assert_eq!(sets.toggle(&Scope::All, &open), minimize(&["a", "b"]));
    }

    #[test]
    fn a_screen_takes_back_its_share_of_an_all_screens_press() {
        // Put away with the setting on "all", then switched to per-screen:
        // a press on the laptop brings back the laptop's, not the TV's.
        let mut sets = HiddenSets::new();
        let open = vec![win("a", false, &[LAPTOP]), win("b", false, &[HDMI])];
        sets.toggle(&Scope::All, &open);

        let now = vec![win("a", true, &[LAPTOP]), win("b", true, &[HDMI])];
        assert_eq!(sets.toggle(&out(LAPTOP), &now), unminimize(&["a"]));
        assert!(sets.is_showing_desktop(&out(HDMI), &now));
        assert_eq!(sets.toggle(&out(HDMI), &now), unminimize(&["b"]));
        assert!(sets.is_empty());
    }

    #[test]
    fn a_window_spanning_both_screens_belongs_to_each() {
        let mut sets = HiddenSets::new();
        let open = vec![win("wide", false, &[LAPTOP, HDMI])];
        assert_eq!(sets.toggle(&out(HDMI), &open), minimize(&["wide"]));
    }

    #[test]
    fn already_minimized_windows_are_left_where_they_are() {
        let mut sets = HiddenSets::new();
        let open = vec![win("a", true, &[LAPTOP]), win("b", false, &[LAPTOP])];
        assert_eq!(sets.toggle(&out(LAPTOP), &open), minimize(&["b"]));
        let now = vec![win("a", true, &[LAPTOP]), win("b", true, &[LAPTOP])];
        assert_eq!(sets.toggle(&out(LAPTOP), &now), unminimize(&["b"]));
    }

    #[test]
    fn closing_what_was_put_away_ends_the_showing_state_for_that_screen_only() {
        let mut sets = HiddenSets::new();
        let open = vec![win("a", false, &[LAPTOP]), win("b", false, &[HDMI])];
        sets.toggle(&out(LAPTOP), &open);
        let now = vec![win("a", true, &[LAPTOP]), win("b", false, &[HDMI])];
        sets.toggle(&out(HDMI), &now);

        // "a" closed.
        let now = vec![win("b", true, &[HDMI])];
        sets.retain_existing(&now);
        assert!(!sets.is_showing_desktop(&out(LAPTOP), &now));
        assert!(sets.is_showing_desktop(&out(HDMI), &now));
    }

    #[test]
    fn without_any_output_information_a_screen_press_acts_on_everything() {
        let mut sets = HiddenSets::new();
        let open = vec![win("a", false, &[]), win("b", false, &[])];
        assert_eq!(sets.toggle(&out(LAPTOP), &open), minimize(&["a", "b"]));
    }
}
