// SPDX-License-Identifier: GPL-3.0-only

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> cosmic::iced::Result {
    tracing_subscriber::fmt::init();
    let _ = tracing_log::LogTracer::init();

    // The same switch as the panel button, for a shortcut, a script, or the
    // installer turning the mode on.
    let arg = std::env::args().nth(1);
    let result = match arg.as_deref() {
        Some("--on") => Some(cosmic_applet_vampire::mode::set(true).map(|()| true)),
        Some("--off") => Some(cosmic_applet_vampire::mode::set(false).map(|()| false)),
        Some("--toggle") => Some(cosmic_applet_vampire::mode::toggle()),
        Some("--status") => Some(Ok(cosmic_applet_vampire::mode::is_on())),
        _ => None,
    };
    if let Some(result) = result {
        match result {
            Ok(on) => {
                println!("{}", if on { "vampire" } else { "sleep" });
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
