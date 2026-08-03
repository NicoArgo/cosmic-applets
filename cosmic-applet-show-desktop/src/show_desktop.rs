// SPDX-License-Identifier: GPL-3.0-only

//! Deciding what "show the desktop" should do, with no Wayland in sight.
//!
//! The whole feature is one rule: **restore exactly what you put away**. A
//! window the user minimized themselves must still be minimized when the
//! desktop comes back, and a window that appeared while the desktop was showing
//! must not be swept up by the trip back. Everything here exists to keep that
//! true across windows closing, opening and being restored by hand in between.

/// A window, as far as this feature cares.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Window<Id> {
    pub id: Id,
    pub minimized: bool,
}

/// One request to make of the compositor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Step<Id> {
    Minimize(Id),
    Unminimize(Id),
}

/// Tracks which windows this applet put away, so it can bring back those and
/// only those.
#[derive(Clone, Debug)]
pub struct ShowDesktop<Id> {
    /// Windows minimized by the last press, in the order they were minimized.
    /// Empty means "the desktop is not being shown by us".
    hidden: Vec<Id>,
}

// Hand-written rather than derived: deriving would demand `Id: Default`, and a
// window handle has no meaningful default.
impl<Id> Default for ShowDesktop<Id> {
    fn default() -> Self {
        Self { hidden: Vec::new() }
    }
}

impl<Id: Clone + PartialEq> ShowDesktop<Id> {
    pub fn new() -> Self {
        Self { hidden: Vec::new() }
    }

    /// Resume from a remembered set — the button and the `--toggle` invocation
    /// are different processes and hand this back and forth through a file.
    pub fn from_hidden(hidden: Vec<Id>) -> Self {
        Self { hidden }
    }

    /// The remembered set, to be handed back to whoever stores it.
    pub fn hidden(&self) -> &[Id] {
        &self.hidden
    }

    /// Whether the next press will restore rather than minimize.
    pub fn is_showing_desktop(&self) -> bool {
        !self.hidden.is_empty()
    }

    /// One press of the button.
    pub fn toggle(&mut self, windows: &[Window<Id>]) -> Vec<Step<Id>> {
        if self.hidden.is_empty() {
            self.hide(windows)
        } else {
            self.restore(windows)
        }
    }

    fn hide(&mut self, windows: &[Window<Id>]) -> Vec<Step<Id>> {
        // Only what is actually on screen. Sweeping up already-minimized
        // windows would mean restoring them later to a state the user never
        // asked for.
        self.hidden = windows
            .iter()
            .filter(|window| !window.minimized)
            .map(|window| window.id.clone())
            .collect();

        self.hidden.iter().cloned().map(Step::Minimize).collect()
    }

    fn restore(&mut self, windows: &[Window<Id>]) -> Vec<Step<Id>> {
        let steps = self
            .hidden
            .iter()
            .filter(|id| {
                // Skip anything that closed meanwhile, and anything the user
                // already brought back by hand — that one is where they want it.
                windows
                    .iter()
                    .any(|window| &&window.id == id && window.minimized)
            })
            .cloned()
            .map(Step::Unminimize)
            .collect();

        self.hidden.clear();
        steps
    }

    /// Forget the remembered set without touching any window.
    ///
    /// For when the desktop stops being shown by something other than this
    /// button — the user restoring every window by hand, say. Without this the
    /// button would still claim to be in "restore" mode with nothing to restore.
    pub fn forget(&mut self) {
        self.hidden.clear();
    }

    /// Drop remembered windows that no longer exist.
    ///
    /// Called as toplevels come and go: if every window we put away has since
    /// been closed, there is nothing left to come back to, and the button
    /// should go back to offering to minimize.
    pub fn retain_existing(&mut self, windows: &[Window<Id>]) {
        self.hidden
            .retain(|id| windows.iter().any(|window| &window.id == id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn windows(list: &[(u32, bool)]) -> Vec<Window<u32>> {
        list.iter()
            .map(|&(id, minimized)| Window { id, minimized })
            .collect()
    }

    #[test]
    fn a_press_puts_away_what_is_on_screen_and_the_next_brings_it_back() {
        let mut state = ShowDesktop::new();
        let open = windows(&[(1, false), (2, false)]);

        assert_eq!(
            state.toggle(&open),
            vec![Step::Minimize(1), Step::Minimize(2)]
        );
        assert!(state.is_showing_desktop());

        let hidden = windows(&[(1, true), (2, true)]);
        assert_eq!(
            state.toggle(&hidden),
            vec![Step::Unminimize(1), Step::Unminimize(2)]
        );
        assert!(!state.is_showing_desktop());
    }

    #[test]
    fn a_window_the_user_minimized_stays_minimized() {
        // The rule the whole feature rests on. Window 1 was already away before
        // the button was pressed, so coming back must leave it away.
        let mut state = ShowDesktop::new();
        let open = windows(&[(1, true), (2, false)]);

        assert_eq!(state.toggle(&open), vec![Step::Minimize(2)]);

        let hidden = windows(&[(1, true), (2, true)]);
        assert_eq!(state.toggle(&hidden), vec![Step::Unminimize(2)]);
    }

    #[test]
    fn a_window_opened_while_the_desktop_showed_is_left_alone() {
        let mut state = ShowDesktop::new();
        state.toggle(&windows(&[(1, false)]));

        // 2 appeared after the desktop was shown; the trip back is not its
        // business.
        let now = windows(&[(1, true), (2, false)]);
        assert_eq!(state.toggle(&now), vec![Step::Unminimize(1)]);
    }

    #[test]
    fn a_window_closed_while_hidden_is_simply_skipped() {
        let mut state = ShowDesktop::new();
        state.toggle(&windows(&[(1, false), (2, false)]));

        let now = windows(&[(2, true)]);
        assert_eq!(state.toggle(&now), vec![Step::Unminimize(2)]);
        assert!(!state.is_showing_desktop());
    }

    #[test]
    fn a_window_restored_by_hand_is_not_restored_twice() {
        let mut state = ShowDesktop::new();
        state.toggle(&windows(&[(1, false), (2, false)]));

        // The user brought 1 back themselves.
        let now = windows(&[(1, false), (2, true)]);
        assert_eq!(state.toggle(&now), vec![Step::Unminimize(2)]);
    }

    #[test]
    fn pressing_with_nothing_on_screen_does_nothing() {
        let mut state = ShowDesktop::<u32>::new();
        assert_eq!(state.toggle(&[]), vec![]);
        assert!(!state.is_showing_desktop());

        // Everything already minimized by hand: there is nothing of ours to
        // restore, so the press is a no-op rather than a mass un-minimize the
        // user never asked for.
        let mut state = ShowDesktop::new();
        assert_eq!(state.toggle(&windows(&[(1, true)])), vec![]);
        assert!(!state.is_showing_desktop());
    }

    #[test]
    fn closing_every_hidden_window_ends_the_showing_state() {
        // Otherwise the button would sit in "restore" mode forever with nothing
        // left to restore, and the next press would do nothing.
        let mut state = ShowDesktop::new();
        state.toggle(&windows(&[(1, false)]));
        assert!(state.is_showing_desktop());

        state.retain_existing(&[]);
        assert!(!state.is_showing_desktop());

        assert_eq!(
            state.toggle(&windows(&[(2, false)])),
            vec![Step::Minimize(2)]
        );
    }

    #[test]
    fn retaining_keeps_windows_that_are_still_around() {
        let mut state = ShowDesktop::new();
        state.toggle(&windows(&[(1, false), (2, false)]));

        state.retain_existing(&windows(&[(2, true)]));
        assert!(state.is_showing_desktop());
        assert_eq!(
            state.toggle(&windows(&[(2, true)])),
            vec![Step::Unminimize(2)]
        );
    }

    #[test]
    fn forgetting_leaves_the_windows_where_they_are() {
        let mut state = ShowDesktop::new();
        state.toggle(&windows(&[(1, false)]));

        state.forget();
        assert!(!state.is_showing_desktop());
        // The next press minimizes again rather than restoring.
        assert_eq!(
            state.toggle(&windows(&[(1, true)])),
            vec![],
            "1 is already minimized, so there is nothing on screen to put away"
        );
    }
}
