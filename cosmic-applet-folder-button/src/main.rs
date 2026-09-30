// SPDX-License-Identifier: GPL-3.0-only

use cosmic_applet_folder_button::folder::Kind;

fn main() -> cosmic::iced::Result {
    tracing_subscriber::fmt::init();
    let _ = tracing_log::LogTracer::init();

    let Some(kind) = std::env::args().nth(1).as_deref().and_then(Kind::from_arg) else {
        eprintln!("usage: cosmic-applet-folder-button pictures|downloads");
        std::process::exit(2);
    };
    cosmic_applet_folder_button::run(kind)
}
