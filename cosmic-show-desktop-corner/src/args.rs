// SPDX-License-Identifier: GPL-3.0-only

//! The command line: which corner, how long a rest counts, what to run.
//! With no arguments it is the original show-desktop triangle, so the
//! service written for that keeps working unchanged.

use std::time::Duration;

use crate::shape::ScreenCorner;

pub const DEFAULT_EXEC: &str = "cosmic-applet-show-desktop --toggle";
/// How long the pointer rests on the corner before it acts on its own. Long
/// enough that sweeping past on the way to the panel doesn't count.
pub const DEFAULT_DWELL: Duration = Duration::from_millis(600);

pub const USAGE: &str = "\
usage: cosmic-show-desktop-corner [--corner C] [--dwell-ms N] [--exec COMMAND]

  --corner C       top-left, top-right, bottom-left or bottom-right (default)
  --dwell-ms N     rest the pointer this long to act without a click
                   (default 600; 0 = click only)
  --exec COMMAND   run through sh -c on click or rest; {output} in it becomes
                   the name of the screen it was triggered on (eDP-1...)
                   (default: cosmic-applet-show-desktop --toggle, which gets
                   --output <screen> added when the setting is \"this screen\")

The triangle shows on every screen. On a show-desktop corner, a right click
flips between \"this screen\" and \"all screens\"
(~/.config/cosmic/com.popflow.ShowDesktop/v1/per_output).";

/// Placeholder in `--exec` for the triggering output's name.
pub const OUTPUT_PLACEHOLDER: &str = "{output}";

/// The command to run when the corner on `output` is triggered.
///
/// - `{output}` anywhere in the command is replaced by the output's name,
///   shell-quoted (empty quotes if the name isn't known yet).
/// - The default show-desktop command gets `--output <name>` appended when the
///   setting is per-screen; with "all screens" it runs as it is.
/// - Any other command runs unchanged.
pub fn command_for(exec: &str, output: Option<&str>, per_output: bool) -> String {
    if exec.contains(OUTPUT_PLACEHOLDER) {
        return exec.replace(OUTPUT_PLACEHOLDER, &shell_quote(output.unwrap_or("")));
    }
    match output {
        Some(name) if per_output && exec == DEFAULT_EXEC && !name.is_empty() => {
            format!("{exec} --output {}", shell_quote(name))
        }
        _ => exec.to_string(),
    }
}

/// Single quotes, with any single quote inside closed, escaped and reopened.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub corner: ScreenCorner,
    /// `None`: hovering never acts, only a click does.
    pub dwell: Option<Duration>,
    pub exec: String,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            corner: ScreenCorner::BottomRight,
            dwell: Some(DEFAULT_DWELL),
            exec: DEFAULT_EXEC.to_string(),
        }
    }
}

impl Options {
    /// Whether this corner shows the desktop — and so whether a right click
    /// flips the show-desktop setting, and the triangle shows which it is.
    /// An overview corner has nothing to do with it.
    pub fn is_show_desktop(&self) -> bool {
        self.exec.contains("cosmic-applet-show-desktop")
    }
}

/// `Ok(None)` is a request for help.
pub fn parse(args: &[String]) -> Result<Option<Options>, String> {
    let mut options = Options::default();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        // Both `--corner top-left` and `--corner=top-left`.
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_string())),
            _ => (arg.as_str(), None),
        };
        if matches!(flag, "-h" | "--help") {
            return Ok(None);
        }
        let mut value = || {
            inline
                .clone()
                .or_else(|| args.next().cloned())
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match flag {
            "--corner" => {
                let name = value()?;
                options.corner = ScreenCorner::from_name(&name)
                    .ok_or_else(|| format!("--corner {name}: not a corner"))?;
            }
            "--dwell-ms" => {
                let ms = value()?;
                let ms: u64 = ms
                    .parse()
                    .map_err(|_| format!("--dwell-ms {ms}: not a number of milliseconds"))?;
                options.dwell = (ms > 0).then(|| Duration::from_millis(ms));
            }
            "--exec" => {
                let exec = value()?;
                if exec.trim().is_empty() {
                    return Err("--exec needs a command".into());
                }
                options.exec = exec;
            }
            _ => return Err(format!("unknown argument {arg}")),
        }
    }
    Ok(Some(options))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_list(list: &[&str]) -> Result<Option<Options>, String> {
        parse(&list.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn no_arguments_is_the_show_desktop_triangle() {
        let options = parse_list(&[]).unwrap().unwrap();
        assert_eq!(options, Options::default());
        assert_eq!(options.corner, ScreenCorner::BottomRight);
        assert_eq!(options.dwell, Some(Duration::from_millis(600)));
        assert_eq!(options.exec, "cosmic-applet-show-desktop --toggle");
    }

    #[test]
    fn reads_every_option_in_both_spellings() {
        let options =
            parse_list(&["--corner", "top-left", "--dwell-ms=250", "--exec", "cosmic-workspaces"])
                .unwrap()
                .unwrap();
        assert_eq!(
            options,
            Options {
                corner: ScreenCorner::TopLeft,
                dwell: Some(Duration::from_millis(250)),
                exec: "cosmic-workspaces".into(),
            }
        );
        // A command with arguments and an `=` of its own stays whole.
        let options = parse_list(&["--exec=env A=b foo --bar"]).unwrap().unwrap();
        assert_eq!(options.exec, "env A=b foo --bar");
    }

    #[test]
    fn the_default_command_follows_the_setting() {
        assert_eq!(
            command_for(DEFAULT_EXEC, Some("HDMI-A-2"), true),
            "cosmic-applet-show-desktop --toggle --output 'HDMI-A-2'"
        );
        assert_eq!(command_for(DEFAULT_EXEC, Some("HDMI-A-2"), false), DEFAULT_EXEC);
        // Name not known yet: the toggle falls back to the focused window's.
        assert_eq!(command_for(DEFAULT_EXEC, None, true), DEFAULT_EXEC);
    }

    #[test]
    fn a_custom_command_gets_the_output_where_it_asks() {
        assert_eq!(
            command_for("notify-send corner {output}", Some("eDP-1"), false),
            "notify-send corner 'eDP-1'"
        );
        assert_eq!(command_for("x {output}", None, true), "x ''");
        assert_eq!(command_for("x {output}", Some("it's"), true), r"x 'it'\''s'");
        // No placeholder: untouched, whatever the setting.
        assert_eq!(command_for("cosmic-workspaces", Some("eDP-1"), true), "cosmic-workspaces");
    }

    #[test]
    fn only_show_desktop_corners_carry_the_setting() {
        assert!(Options::default().is_show_desktop());
        let overview = parse_list(&["--exec", "cosmic-workspaces"]).unwrap().unwrap();
        assert!(!overview.is_show_desktop());
    }

    #[test]
    fn zero_dwell_means_click_only() {
        let options = parse_list(&["--dwell-ms", "0"]).unwrap().unwrap();
        assert_eq!(options.dwell, None);
    }

    #[test]
    fn rejects_what_it_cannot_use() {
        assert!(parse_list(&["--corner", "middle"]).is_err());
        assert!(parse_list(&["--corner"]).is_err());
        assert!(parse_list(&["--dwell-ms", "soon"]).is_err());
        assert!(parse_list(&["--dwell-ms", "-5"]).is_err());
        assert!(parse_list(&["--exec", " "]).is_err());
        assert!(parse_list(&["--frobnicate"]).is_err());
        assert_eq!(parse_list(&["--help"]), Ok(None));
    }
}
