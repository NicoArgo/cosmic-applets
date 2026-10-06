// SPDX-License-Identifier: GPL-3.0-only

//! The command line, for the routes that are not the panel button.

use crate::config;

pub const USAGE: &str = "\
usage: cosmic-applet-show-desktop                       (the panel applet)
       cosmic-applet-show-desktop --toggle [--output NAME] [--dry-run]
       cosmic-applet-show-desktop --scope screen|all|status

  --toggle         show the desktop, or bring back what the last one put away
  --output NAME    only on that output (eDP-1, HDMI-A-2...), when the setting
                   is \"this screen\"; without it, the focused window's output
  --dry-run        with --toggle: print the windows, their outputs, the scope
                   and what would happen, and touch nothing
  --scope screen   from now on, only the screen where it is triggered (default)
  --scope all      from now on, every screen
  --scope status   print the current setting: screen or all";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    Applet,
    Toggle {
        output: Option<String>,
        dry_run: bool,
    },
    SetScope {
        per_output: bool,
    },
    ScopeStatus,
    Help,
}

/// Anything without `--toggle` or `--scope` is the panel applet, whatever else
/// the panel passes along — as before there was a command line to speak of.
pub fn parse(args: &[String]) -> Result<Command, String> {
    let mut toggle = false;
    let mut dry_run = false;
    let mut output = None;
    let mut scope = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_string())),
            _ => (arg.as_str(), None),
        };
        let mut value = || {
            inline
                .clone()
                .or_else(|| args.next().cloned())
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match flag {
            "-h" | "--help" => return Ok(Command::Help),
            "--toggle" => toggle = true,
            "--dry-run" => dry_run = true,
            "--output" => output = Some(value()?).filter(|name| !name.is_empty()),
            "--scope" => scope = Some(value()?),
            _ => {}
        }
    }
    match (toggle, scope.as_deref()) {
        (true, Some(_)) => Err("--toggle and --scope are separate commands".into()),
        (true, None) => Ok(Command::Toggle { output, dry_run }),
        (false, Some("screen")) => Ok(Command::SetScope { per_output: true }),
        (false, Some("all")) => Ok(Command::SetScope { per_output: false }),
        (false, Some("status")) => Ok(Command::ScopeStatus),
        (false, Some(other)) => Err(format!("--scope {other}: expected screen, all or status")),
        (false, None) if output.is_some() || dry_run => {
            Err("--output and --dry-run go with --toggle".into())
        }
        (false, None) => Ok(Command::Applet),
    }
}

pub fn scope_name(per_output: bool) -> &'static str {
    if per_output { "screen" } else { "all" }
}

/// `--scope ...`: change or report the setting.
pub fn run_scope(command: &Command) -> Result<(), String> {
    match command {
        Command::SetScope { per_output } => {
            config::set_per_output(*per_output).map_err(|err| format!("{err}"))?;
            println!("{}", scope_name(*per_output));
            Ok(())
        }
        Command::ScopeStatus => {
            println!("{}", scope_name(config::per_output()));
            Ok(())
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_list(list: &[&str]) -> Result<Command, String> {
        parse(&list.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn toggles_with_or_without_an_output() {
        assert_eq!(
            parse_list(&["--toggle"]),
            Ok(Command::Toggle {
                output: None,
                dry_run: false
            })
        );
        assert_eq!(
            parse_list(&["--toggle", "--output", "HDMI-A-2"]),
            Ok(Command::Toggle {
                output: Some("HDMI-A-2".into()),
                dry_run: false
            })
        );
        assert_eq!(
            parse_list(&["--output=eDP-1", "--toggle", "--dry-run"]),
            Ok(Command::Toggle {
                output: Some("eDP-1".into()),
                dry_run: true
            })
        );
        // An empty name (a corner that didn't know its output yet) is none.
        assert_eq!(
            parse_list(&["--toggle", "--output", ""]),
            Ok(Command::Toggle {
                output: None,
                dry_run: false
            })
        );
    }

    #[test]
    fn scope_commands() {
        assert_eq!(
            parse_list(&["--scope", "screen"]),
            Ok(Command::SetScope { per_output: true })
        );
        assert_eq!(
            parse_list(&["--scope=all"]),
            Ok(Command::SetScope { per_output: false })
        );
        assert_eq!(parse_list(&["--scope", "status"]), Ok(Command::ScopeStatus));
        assert!(parse_list(&["--scope", "both"]).is_err());
        assert!(parse_list(&["--scope"]).is_err());
        assert!(parse_list(&["--scope", "all", "--toggle"]).is_err());
    }

    #[test]
    fn anything_else_is_the_applet() {
        assert_eq!(parse_list(&[]), Ok(Command::Applet));
        assert_eq!(
            parse_list(&["--something-the-panel-passes"]),
            Ok(Command::Applet)
        );
        assert_eq!(parse_list(&["--help"]), Ok(Command::Help));
        assert!(parse_list(&["--output", "eDP-1"]).is_err());
    }
}
