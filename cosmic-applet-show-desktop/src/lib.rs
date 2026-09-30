// SPDX-License-Identifier: GPL-3.0-only

//! POP Flow — "show the desktop": one press puts every window on the current
//! workspace away, the next brings back exactly those.

mod localize;
pub mod show_desktop;
pub mod state_file;
pub mod toggle;
pub(crate) mod wayland_handler;
pub(crate) mod wayland_subscription;

use crate::{
    localize::localize,
    show_desktop::{ShowDesktop, Step, Window},
    wayland_subscription::{WaylandRequest, WaylandUpdate, WindowEntry, wayland_subscription},
};
use cosmic::{
    Element,
    app::{self, Core},
    cctk::sctk::reexports::calloop,
    iced::{self, Limits, Subscription, id::Id as WidgetId},
    widget::autosize::autosize,
};
use std::sync::LazyLock;

/// The windows the decision runs against, keyed by the compositor's stable
/// identifier rather than the Wayland handle — that is what the shared state
/// file can hold.
pub(crate) fn to_windows(entries: &[WindowEntry]) -> Vec<Window<String>> {
    entries
        .iter()
        .map(|entry| Window {
            id: entry.identifier.clone(),
            minimized: entry.minimized,
        })
        .collect()
}

static AUTOSIZE_MAIN_ID: LazyLock<WidgetId> = LazyLock::new(|| WidgetId::new("autosize-main"));

pub fn run() -> cosmic::iced::Result {
    localize();
    cosmic::applet::run::<ShowDesktopApplet>(())
}

#[derive(Default)]
struct ShowDesktopApplet {
    core: Core,
    tx: Option<calloop::channel::Sender<WaylandRequest>>,
    windows: Vec<WindowEntry>,
    /// Whether the next press restores. Cached from the shared state file so
    /// the icon does not read a file on every frame; refreshed whenever the
    /// window list changes, which a toggle by any route always causes.
    showing: bool,
    /// Pointer over the button: the icon grows to `HOVER_SCALE`.
    hovered: bool,
    /// Current icon scale, animated toward 1.0 or `HOVER_SCALE`.
    scale: f32,
    last_frame: Option<std::time::Instant>,
}

/// How much the icon grows under the pointer, and how long it takes — the
/// same as the folder buttons next to it.
const HOVER_SCALE: f32 = 1.25;
const SCALE_DURATION: f32 = 0.12;

#[derive(Clone, Debug)]
enum Message {
    Wayland(WaylandUpdate),
    Press,
    Hover(bool),
    Frame(std::time::Instant),
}

impl ShowDesktopApplet {
    fn send(&self, request: WaylandRequest) {
        if let Some(tx) = &self.tx
            && let Err(err) = tx.send(request)
        {
            tracing::error!("failed to reach the wayland thread: {err:?}");
        }
    }
}

impl cosmic::Application for ShowDesktopApplet {
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    type Message = Message;

    const APP_ID: &'static str = "com.popflow.CosmicAppletShowDesktop";

    fn init(core: Core, _flags: ()) -> (Self, app::Task<Message>) {
        (
            Self {
                core,
                scale: 1.0,
                ..Default::default()
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

    fn subscription(&self) -> Subscription<Message> {
        let target = if self.hovered { HOVER_SCALE } else { 1.0 };
        let frames = if (self.scale - target).abs() > f32::EPSILON {
            iced::window::frames().map(|(_, at)| Message::Frame(at))
        } else {
            Subscription::none()
        };
        // The pointer usually leaves this tiny surface in the same motion that
        // leaves the button; the surface's CursorLeft is what reliably says so.
        let left = iced::event::listen_with(|event, _, _| match event {
            iced::Event::Mouse(iced::mouse::Event::CursorLeft) => Some(Message::Hover(false)),
            _ => None,
        });
        Subscription::batch([wayland_subscription().map(Message::Wayland), frames, left])
    }

    fn update(&mut self, message: Message) -> app::Task<Message> {
        match message {
            Message::Wayland(update) => match update {
                WaylandUpdate::Init(tx) => {
                    self.tx = Some(tx);
                }
                WaylandUpdate::Finished => {
                    self.tx = None;
                    self.windows.clear();
                    self.showing = false;
                }
                WaylandUpdate::Windows(windows) => {
                    self.windows = windows;

                    // Windows we put away can be closed while the desktop is
                    // showing. Once the last one goes there is nothing to come
                    // back to, and the button should offer to minimize again.
                    let mut state = ShowDesktop::from_hidden(state_file::load());
                    state.retain_existing(&to_windows(&self.windows));
                    state_file::save(state.hidden());
                    self.showing = state.is_showing_desktop();
                }
            },
            Message::Hover(hovered) => {
                self.hovered = hovered;
                self.last_frame = None;
            }
            Message::Frame(at) => {
                let dt = self
                    .last_frame
                    .map_or(1.0 / 60.0, |last| at.duration_since(last).as_secs_f32())
                    .min(1.0 / 30.0);
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
                let windows = to_windows(&self.windows);

                // The file, not this struct, is the source of truth: a touchpad
                // gesture runs the same toggle from another process, and
                // whichever acted second would otherwise restore the wrong set.
                let mut state = ShowDesktop::from_hidden(state_file::load());
                let steps = state.toggle(&windows);
                state_file::save(state.hidden());
                self.showing = state.is_showing_desktop();

                for step in steps {
                    let (identifier, minimize) = match step {
                        Step::Minimize(id) => (id, true),
                        Step::Unminimize(id) => (id, false),
                    };
                    let Some(entry) = self
                        .windows
                        .iter()
                        .find(|entry| entry.identifier == identifier)
                    else {
                        continue;
                    };
                    self.send(if minimize {
                        WaylandRequest::Minimize(entry.handle.clone())
                    } else {
                        WaylandRequest::Unminimize(entry.handle.clone())
                    });
                }
            }
        }
        app::Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let showing = self.showing;
        // Two icons rather than one: with a single icon there is no way to tell
        // "press to hide" from "press to bring back", and the button is the
        // only thing on screen once the desktop is showing.
        let icon = if showing {
            "view-restore-symbolic"
        } else {
            "user-desktop-symbolic"
        };
        let suggested = self.core.applet.suggested_size(true);
        let icon = cosmic::widget::icon(cosmic::widget::icon::from_name(icon).symbolic(true).handle())
            .class(cosmic::theme::Svg::custom(|theme| iced::widget::svg::Style {
                color: Some(theme.cosmic().background(theme.transparent).on.into()),
            }))
            .width(iced::Length::Fixed(suggested.0 as f32 * self.scale))
            .height(iced::Length::Fixed(suggested.1 as f32 * self.scale));
        // The button keeps its size; only the icon grows, into the padding.
        let button = self
            .core
            .applet
            .button_from_element(icon, true)
            .on_press(Message::Press);

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
