// SPDX-License-Identifier: GPL-3.0-only

fn main() -> cosmic::iced::Result {
    tracing_subscriber::fmt::init();
    let _ = tracing_log::LogTracer::init();
    cosmic_applet_terminals::run()
}
