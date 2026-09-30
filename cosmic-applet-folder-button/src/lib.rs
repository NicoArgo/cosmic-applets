// SPDX-License-Identifier: GPL-3.0-only

//! POP Flow — a panel button for one folder (Pictures, Downloads).
//!
//! Pressing it brings forward a file-manager window already showing the folder
//! — unminimizing it, or switching to its workspace — and only opens a new
//! window when there is none. While such a window exists the label is drawn in
//! the accent color, so the panel says "it's open" before you press.

pub mod folder;
mod wayland;

use cosmic::{
    Element,
    app::{self, Core},
    applet::cosmic_panel_config::{PanelAnchor, PanelSize},
    applet::Size,
    cctk::sctk::reexports::calloop,
    iced::{self, Alignment, Length, Limits, id::Id as WidgetId},
    widget::{autosize::autosize, row, space},
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
        // Same presentation rule as cosmic-panel-button, which these buttons
        // replace: an icon on a vertical panel or a big one, text otherwise.
        let icon_mode = matches!(self.core.applet.anchor, PanelAnchor::Left | PanelAnchor::Right)
            || matches!(
                self.core.applet.size,
                Size::PanelSize(PanelSize::S | PanelSize::M | PanelSize::L | PanelSize::XL)
            );

        let button = if icon_mode {
            let icon = match (self.kind, open) {
                (_, true) => "folder-open-symbolic",
                (Kind::Pictures, false) => "folder-pictures-symbolic",
                (Kind::Downloads, false) => "folder-download-symbolic",
            };
            self.core.applet.icon_button(icon)
        } else {
            let mut label = self.core.applet.text(self.name.clone());
            if open {
                label = label.class(cosmic::theme::Text::Accent);
            }
            let content = row![
                label,
                space::vertical().height(Length::Fixed(
                    (self.core.applet.suggested_size(true).1
                        + 2 * self.core.applet.suggested_padding(true).1) as f32
                ))
            ]
            .align_y(Alignment::Center);
            cosmic::widget::button::custom(content)
                .padding([0, self.core.applet.suggested_padding(true).0])
                .class(cosmic::theme::Button::AppletIcon)
        };

        autosize(button.on_press_down(Message::Press), AUTOSIZE_MAIN_ID.clone())
            .limits(Limits::NONE.min_width(1.).min_height(1.))
            .into()
    }

    fn style(&self) -> Option<iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}
