// SPDX-License-Identifier: GPL-3.0-only

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> cosmic::iced::Result {
    tracing_subscriber::fmt::init();
    let _ = tracing_log::LogTracer::init();

    // `--toggle` is the same action as pressing the panel button, for whoever
    // has no panel button to press: a keyboard shortcut, or a touchpad gesture
    // dispatched by the compositor.
    if std::env::args().any(|arg| arg == "--toggle") {
        return match cosmic_applet_show_desktop::toggle::run() {
            Ok(()) => Ok(()),
            Err(err) => {
                tracing::error!("show-desktop toggle failed: {err}");
                std::process::exit(1);
            }
        };
    }

    tracing::info!("Starting POP Flow show-desktop applet {VERSION}");

    cosmic_applet_show_desktop::run()
}
