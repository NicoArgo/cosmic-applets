// SPDX-License-Identifier: GPL-3.0-only

//! Which folder a button stands for, and which open window shows it. No
//! Wayland here.

use std::path::PathBuf;

/// The file manager whose windows the button looks for.
pub const FILES_APP_ID: &str = "com.system76.CosmicFiles";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Pictures,
    Downloads,
}

impl Kind {
    pub fn from_arg(arg: &str) -> Option<Self> {
        match arg {
            "pictures" => Some(Kind::Pictures),
            "downloads" => Some(Kind::Downloads),
            _ => None,
        }
    }

    /// The XDG folder, resolved when asked rather than once: it follows the
    /// user's `user-dirs.dirs`, whatever it's named in their language.
    pub fn path(self) -> Option<PathBuf> {
        match self {
            Kind::Pictures => dirs::picture_dir(),
            Kind::Downloads => dirs::download_dir(),
        }
    }

    /// The label on the panel, and the name the file manager titles its
    /// window with: the folder's own name ("Imagens", "Downloads").
    pub fn name(self) -> String {
        self.path()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| match self {
                Kind::Pictures => "Pictures".into(),
                Kind::Downloads => "Downloads".into(),
            })
    }
}

/// A window, as far as the button cares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window<H> {
    pub handle: H,
    pub app_id: String,
    pub title: String,
    pub minimized: bool,
    pub active: bool,
}

/// Whether a window is the file manager showing `folder`.
///
/// COSMIC Files titles its window "<active tab> — <app name>", and a tab is
/// titled with the folder's name. So a Files window whose title starts with
/// "<folder> — " is on that folder. Matching by title is safe here because the
/// only action taken is bringing a window forward — never closing one.
pub fn shows(app_id: &str, title: &str, folder: &str) -> bool {
    app_id == FILES_APP_ID
        && title
            .strip_prefix(folder)
            .is_some_and(|rest| rest.starts_with(" — "))
}

/// The window a press should bring forward, if any shows the folder: one that
/// is already visible over a minimized one, so a press never has to unhide
/// something when a visible match exists.
pub fn pick<'a, H>(windows: &'a [Window<H>], folder: &str) -> Option<&'a Window<H>> {
    let mut matches = windows
        .iter()
        .filter(|w| shows(&w.app_id, &w.title, folder));
    let first = matches.next()?;
    Some(
        std::iter::once(first)
            .chain(matches)
            .find(|w| !w.minimized)
            .unwrap_or(first),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(title: &str, minimized: bool) -> Window<u32> {
        Window {
            handle: title.len() as u32 + minimized as u32,
            app_id: FILES_APP_ID.into(),
            title: title.into(),
            minimized,
            active: false,
        }
    }

    #[test]
    fn matches_the_folder_title_only() {
        assert!(shows(FILES_APP_ID, "Imagens — Arquivos COSMIC", "Imagens"));
        assert!(shows(FILES_APP_ID, "Downloads — COSMIC Files", "Downloads"));
        // A different folder that merely starts the same.
        assert!(!shows(FILES_APP_ID, "Imagens antigas — Arquivos COSMIC", "Imagens"));
        // The bare app title (no tab).
        assert!(!shows(FILES_APP_ID, "Arquivos COSMIC", "Imagens"));
        // Another app with a look-alike title.
        assert!(!shows("org.gnome.Nautilus", "Imagens — Arquivos COSMIC", "Imagens"));
    }

    #[test]
    fn prefers_a_visible_window() {
        let windows = [w("Imagens — Arquivos COSMIC", true), w("Imagens — Arquivos COSMIC", false)];
        assert!(!pick(&windows, "Imagens").unwrap().minimized);
    }

    #[test]
    fn a_minimized_match_is_still_a_match() {
        let windows = [w("Imagens — Arquivos COSMIC", true)];
        assert!(pick(&windows, "Imagens").is_some());
    }

    #[test]
    fn nothing_open_means_open_one() {
        let windows = [w("Downloads — Arquivos COSMIC", false)];
        assert!(pick(&windows, "Imagens").is_none());
    }

    #[test]
    fn args() {
        assert_eq!(Kind::from_arg("pictures"), Some(Kind::Pictures));
        assert_eq!(Kind::from_arg("downloads"), Some(Kind::Downloads));
        assert_eq!(Kind::from_arg("music"), None);
    }
}
