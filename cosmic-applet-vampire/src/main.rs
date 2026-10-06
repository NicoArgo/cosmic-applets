// SPDX-License-Identifier: GPL-3.0-only

use cosmic_applet_vampire::{mode, watch};
use std::time::SystemTime;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// What the command line asks for; nothing means "be the panel button".
#[derive(Debug, PartialEq)]
enum Cli {
    On,
    Off,
    Toggle,
    Status,
    For(u32),
    Watch,
    Refresh,
}

fn parse(args: &[String]) -> Result<Option<Cli>, String> {
    let cli = match args.first().map(String::as_str) {
        Some("--on") => Cli::On,
        Some("--off") => Cli::Off,
        Some("--toggle") => Cli::Toggle,
        Some("--status") => Cli::Status,
        Some("--for") => {
            let minutes = args.get(1).ok_or("--for needs a number of minutes")?;
            match minutes.parse::<u32>() {
                Ok(m) if m > 0 => Cli::For(m),
                _ => return Err(format!("--for {minutes}: not a number of minutes above 0")),
            }
        }
        Some("--watch") => Cli::Watch,
        Some("--refresh") => Cli::Refresh,
        _ => return Ok(None),
    };
    Ok(Some(cli))
}

fn main() -> cosmic::iced::Result {
    tracing_subscriber::fmt::init();
    let _ = tracing_log::LogTracer::init();

    // The same switch as the panel button, for a shortcut, a script, or the
    // installer turning the mode on.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match parse(&args) {
        Ok(None) => None,
        Ok(Some(Cli::On)) => Some(mode::set(true).map(|()| true)),
        Ok(Some(Cli::Off)) => Some(mode::set(false).map(|()| false)),
        Ok(Some(Cli::Toggle)) => Some(mode::toggle()),
        Ok(Some(Cli::Status)) => Some(Ok(mode::is_on())),
        Ok(Some(Cli::For(minutes))) => Some(mode::set_for(minutes).map(|()| true)),
        Ok(Some(Cli::Refresh)) => Some(mode::refresh()),
        Ok(Some(Cli::Watch)) => Some(watch::run().map(|()| false)),
        Err(err) => Some(Err(err)),
    };
    if let Some(result) = result {
        match result {
            Ok(on) => {
                // First line stays one word, for scripts; the time left, if
                // any, goes on a second.
                println!("{}", if on { "vampire" } else { "sleep" });
                if let Some(at) = mode::deadline().filter(|_| on) {
                    let left = mode::remaining(at, SystemTime::now());
                    println!("{} left", mode::format_remaining(left));
                }
                std::process::exit(0);
            }
            Err(err) => {
                eprintln!("{err}");
                std::process::exit(1);
            }
        }
    }

    tracing::info!("Starting POP Flow vampire applet {VERSION}");
    cosmic_applet_vampire::run()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_the_switches() {
        assert_eq!(parse(&args(&[])), Ok(None));
        assert_eq!(parse(&args(&["--on"])), Ok(Some(Cli::On)));
        assert_eq!(parse(&args(&["--status"])), Ok(Some(Cli::Status)));
        assert_eq!(parse(&args(&["--watch"])), Ok(Some(Cli::Watch)));
    }

    #[test]
    fn for_takes_whole_minutes_above_zero() {
        assert_eq!(parse(&args(&["--for", "60"])), Ok(Some(Cli::For(60))));
        assert!(parse(&args(&["--for"])).is_err());
        assert!(parse(&args(&["--for", "0"])).is_err());
        assert!(parse(&args(&["--for", "1.5"])).is_err());
        assert!(parse(&args(&["--for", "-3"])).is_err());
    }
}
