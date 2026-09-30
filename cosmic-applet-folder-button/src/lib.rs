// SPDX-License-Identifier: GPL-3.0-only

//! POP Flow — a panel button for one folder (Pictures, Downloads).
//!
//! Pressing it brings forward a file-manager window already showing the folder
//! — unminimizing it, or switching to its workspace — and only opens a new
//! window when there is none. While such a window exists the icon is drawn in
//! the accent color, so the panel says "it's open" before you press.

pub mod folder;
mod wayland;

use cosmic::{
    Element,
    app::{self, Core},
    cctk::sctk::reexports::calloop,
    iced::{self, Length, Limits, id::Id as WidgetId},
    widget::{autosize::autosize, tooltip},
};
use std::sync::LazyLock;

use folder::{Kind, Window};
use wayland::{Handle, Request, Update};

static AUTOSIZE_MAIN_ID: LazyLock<WidgetId> = LazyLock::new(|| WidgetId::new("autosize-main"));

pub fn run(kind: Kind) -> cosmic::iced::Result {
    cosmic::applet::run::<FolderButton>(kind)
}

struct FolderButton {
    core: Core,
    kind: Kind,
    /// Cached folder name: the label, and what window titles are matched with.
    name: String,
    tx: Option<calloop::channel::Sender<Request>>,
    windows: Vec<Window<Handle>>,
}

#[derive(Clone, Debug)]
enum Message {
    Wayland(Update),
    Press,
}

impl FolderButton {
    fn is_open(&self) -> bool {
        folder::pick(&self.windows, &self.name).is_some()
    }

    fn open_new(&self) {
        let Some(path) = self.kind.path() else {
            tracing::error!("no XDG directory for {:?}", self.kind);
            return;
        };
        match std::process::Command::new("cosmic-files").arg(&path).spawn() {
            Ok(mut child) => {
                std::thread::spawn(move || child.wait());
            }
            Err(err) => tracing::error!("cosmic-files {}: {err}", path.display()),
        }
    }
}

impl cosmic::Application for FolderButton {
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = Kind;
    type Message = Message;

    const APP_ID: &'static str = "com.popflow.CosmicAppletFolderButton";

    fn init(core: Core, kind: Kind) -> (Self, app::Task<Message>) {
        (
            Self {
                core,
                kind,
                name: kind.name(),
                tx: None,
                windows: Vec::new(),
            },
            app::Task::none(),
        )
    }

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn subscription(&self) -> iced::Subscription<Message> {
        wayland::subscription().map(Message::Wayland)
    }

    fn update(&mut self, message: Message) -> app::Task<Message> {
        match message {
            Message::Wayland(Update::Init(tx)) => self.tx = Some(tx),
            Message::Wayland(Update::Finished) => {
                self.tx = None;
                self.windows.clear();
            }
            Message::Wayland(Update::Windows(windows)) => self.windows = windows,
            Message::Press => {
                // Resolve again: the folder may have been renamed or moved in
                // user-dirs.dirs since the applet started.
                self.name = self.kind.name();
                match (folder::pick(&self.windows, &self.name), &self.tx) {
                    (Some(window), Some(tx)) => {
                        if let Err(err) = tx.send(Request::Activate(window.handle.clone())) {
                            tracing::error!("failed to reach the wayland thread: {err:?}");
                            self.open_new();
                        }
                    }
                    _ => self.open_new(),
                }
            }
        }
        app::Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let open = self.is_open();
        let icon_name = match self.kind {
            Kind::Pictures => "folder-pictures-symbolic",
            Kind::Downloads => "folder-download-symbolic",
        };
        let suggested = self.core.applet.suggested_size(true);
        // Always an icon, like the rest of the applet row. While a window on
        // the folder is open the icon takes the accent color — the same "it's
        // open" cue the label used to give.
        let icon = cosmic::widget::icon(cosmic::widget::icon::from_name(icon_name).symbolic(true).handle())
            .class(cosmic::theme::Svg::custom(move |theme| {
                let cosmic = theme.cosmic();
                iced::widget::svg::Style {
                    color: Some(if open {
                        cosmic.accent_color().into()
                    } else {
                        cosmic.background(theme.transparent).on.into()
                    }),
                }
            }))
            .width(Length::Fixed(suggested.0 as f32))
            .height(Length::Fixed(suggested.1 as f32));
        let button = self
            .core
            .applet
            .button_from_element(icon, true)
            .on_press_down(Message::Press);

        autosize(
            tooltip(
                button,
                cosmic::widget::text::body(self.name.clone()),
                tooltip::Position::Bottom,
            ),
            AUTOSIZE_MAIN_ID.clone(),
        )
        .limits(Limits::NONE.min_width(1.).min_height(1.))
        .into()
    }

    fn style(&self) -> Option<iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}
