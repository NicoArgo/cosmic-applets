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
    widget::autosize::autosize,
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
    /// Pointer over the button: the icon grows to `HOVER_SCALE`.
    hovered: bool,
    /// Current icon scale, animated toward 1.0 or `HOVER_SCALE`.
    scale: f32,
    last_frame: Option<std::time::Instant>,
}

/// How much the icon grows under the pointer, and how long it takes.
const HOVER_SCALE: f32 = 1.25;
const SCALE_DURATION: f32 = 0.12;

#[derive(Clone, Debug)]
enum Message {
    Wayland(Update),
    Press,
    Hover(bool),
    Frame(std::time::Instant),
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
                hovered: false,
                scale: 1.0,
                last_frame: None,
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
        let target = if self.hovered { HOVER_SCALE } else { 1.0 };
        // Frames only while the icon is growing or shrinking.
        let frames = if (self.scale - target).abs() > f32::EPSILON {
            iced::window::frames().map(|(_, at)| Message::Frame(at))
        } else {
            iced::Subscription::none()
        };
        iced::Subscription::batch([wayland::subscription().map(Message::Wayland), frames])
    }

    fn update(&mut self, message: Message) -> app::Task<Message> {
        match message {
            Message::Wayland(Update::Init(tx)) => self.tx = Some(tx),
            Message::Wayland(Update::Finished) => {
                self.tx = None;
                self.windows.clear();
            }
            Message::Wayland(Update::Windows(windows)) => self.windows = windows,
            Message::Hover(hovered) => {
                self.hovered = hovered;
                self.last_frame = None;
            }
            Message::Frame(at) => {
                let dt = self
                    .last_frame
                    .map_or(1.0 / 60.0, |last| at.duration_since(last).as_secs_f32());
                self.last_frame = Some(at);
                let target = if self.hovered { HOVER_SCALE } else { 1.0 };
                let step = (HOVER_SCALE - 1.0) * dt / SCALE_DURATION;
                self.scale = if self.scale < target {
                    (self.scale + step).min(target)
                } else {
                    (self.scale - step).max(target)
                };
            }
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
            .width(Length::Fixed(suggested.0 as f32 * self.scale))
            .height(Length::Fixed(suggested.1 as f32 * self.scale));
        // The button keeps its size — only the icon inside grows, into the
        // button's padding, so nothing on the panel shifts.
        let button = self
            .core
            .applet
            .button_from_element(icon, true)
            .on_press_down(Message::Press);

        autosize(
            cosmic::widget::mouse_area(button)
                .on_enter(Message::Hover(true))
                .on_exit(Message::Hover(false)),
            AUTOSIZE_MAIN_ID.clone(),
        )
        .limits(Limits::NONE.min_width(1.).min_height(1.))
        .into()
    }

    fn style(&self) -> Option<iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}
