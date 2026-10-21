// SPDX-License-Identifier: GPL-3.0-only

//! POP Flow: "Open in terminal" for an app in the dock — the folder its
//! launcher names (`Path=`), else the git repository its program was built in
//! (symlinks followed, so `/usr/local/bin/x` linked into a checkout finds the
//! checkout). Apps that come from a package have neither, and get no item.
//! Same rules as the file manager's (cosmic-files `src/terminal_dir.rs`).

use std::path::{Path, PathBuf};

pub fn app_dir(exec: &str, work_dir: Option<&str>) -> Option<PathBuf> {
    if let Some(dir) = work_dir.map(PathBuf::from).filter(|d| d.is_dir()) {
        return Some(dir);
    }
    let program = resolve_program(exec)?;
    let program = std::fs::canonicalize(&program).unwrap_or(program);
    program
        .parent()?
        .ancestors()
        .find(|d| d.join(".git").exists())
        .map(Path::to_path_buf)
}

/// The default terminal (cosmic-term unless the system names another),
/// started in `dir`.
pub fn open(dir: &Path) {
    let terminal = ["x-terminal-emulator", "cosmic-term"]
        .into_iter()
        .find(|t| resolve_program(t).is_some())
        .unwrap_or("cosmic-term");
    let mut command = std::process::Command::new(terminal);
    command.current_dir(dir);
    match command.spawn() {
        // Reaped off-thread, so no zombie lingers in the panel.
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(err) => tracing::warn!("{terminal} in {}: {err}", dir.display()),
    }
}

/// The program an `Exec=` line runs: first word, quotes removed, past an
/// `env VAR=value` prefix, looked up in `PATH` when it has no slash.
fn resolve_program(exec: &str) -> Option<PathBuf> {
    let program = exec
        .split_whitespace()
        .map(|w| w.trim_matches('"'))
        .find(|w| *w != "env" && !(w.contains('=') && !w.starts_with('/')))?;
    if program.contains('/') {
        return Some(PathBuf::from(program)).filter(|p| p.is_file());
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(program))
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_repo_through_a_symlink() {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("target/release")).unwrap();
        let bin = repo.join("target/release/app");
        std::fs::write(&bin, "").unwrap();
        let link = root.path().join("app");
        std::os::unix::fs::symlink(&bin, &link).unwrap();
        let exec = format!("env A=1 \"{}\" %U", link.display());
        assert_eq!(app_dir(&exec, None), Some(repo));
    }

    #[test]
    fn packaged_apps_have_none_but_path_wins() {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("app");
        std::fs::write(&bin, "").unwrap();
        let exec = bin.display().to_string();
        assert_eq!(app_dir(&exec, None), None);
        let work = root.path().display().to_string();
        assert_eq!(app_dir(&exec, Some(&work)), Some(root.path().to_path_buf()));
    }
}
