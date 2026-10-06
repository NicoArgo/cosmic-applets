// SPDX-License-Identifier: GPL-3.0-only

use cosmic_applet_show_desktop::cli::{self, Command};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> cosmic::iced::Result {
    tracing_subscriber::fmt::init();
    let _ = tracing_log::LogTracer::init();

    let command = match cli::parse(&std::env::args().skip(1).collect::<Vec<_>>()) {
        Ok(command) => command,
        Err(err) => {
            eprintln!("{err}\n\n{}", cli::USAGE);
            std::process::exit(2);
        }
    };

    match command {
        Command::Help => {
            println!("{}", cli::USAGE);
            Ok(())
        }
        // `--toggle` is the same action as pressing the panel button, for
        // whoever has no panel button to press: a keyboard shortcut, a screen
        // corner, or a touchpad gesture dispatched by the compositor.
        Command::Toggle { output, dry_run } => {
            match cosmic_applet_show_desktop::toggle::run(output, dry_run) {
                Ok(()) => Ok(()),
                Err(err) => {
                    tracing::error!("show-desktop toggle failed: {err}");
                    std::process::exit(1);
                }
            }
        }
        Command::SetScope { .. } | Command::ScopeStatus => match cli::run_scope(&command) {
            Ok(()) => Ok(()),
            Err(err) => {
                eprintln!("show-desktop: {err}");
                std::process::exit(1);
            }
        },
        Command::Applet => {
            tracing::info!("Starting POP Flow show-desktop applet {VERSION}");
            cosmic_applet_show_desktop::run()
        }
    }
}
