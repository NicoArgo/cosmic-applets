// SPDX-License-Identifier: GPL-3.0-only

//! POP Flow — "show the desktop": one press puts every window on the current
//! workspace away, the next brings back exactly those.

mod localize;
pub mod show_desktop;
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
    cctk::{
        sctk::reexports::calloop,
        wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
    },
    iced::{self, Limits, Subscription, id::Id as WidgetId},
    widget::{autosize::autosize, tooltip},
};
use std::sync::LazyLock;

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
    state: ShowDesktop<ExtForeignToplevelHandleV1>,
}

#[derive(Clone, Debug)]
enum Message {
    Wayland(WaylandUpdate),
    Press,
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
        wayland_subscription().map(Message::Wayland)
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
                    // The thread is gone, so nothing we remember can be acted
                    // on any more. Forgetting leaves the windows exactly where
                    // they are; it only resets what the next press means.
                    self.state.forget();
                }
                WaylandUpdate::Windows(windows) => {
                    self.windows = windows;
                    // Windows we put away can be closed while the desktop is
                    // showing. Once the last one goes, there is nothing to come
                    // back to and the button should offer to minimize again.
                    let known: Vec<_> = self
                        .windows
                        .iter()
                        .map(|entry| Window {
                            id: entry.handle.clone(),
                            minimized: entry.minimized,
                        })
                        .collect();
                    self.state.retain_existing(&known);
                }
            },
            Message::Press => {
                let windows: Vec<_> = self
                    .windows
                    .iter()
                    .map(|entry| Window {
                        id: entry.handle.clone(),
                        minimized: entry.minimized,
                    })
                    .collect();

                for step in self.state.toggle(&windows) {
                    self.send(match step {
                        Step::Minimize(handle) => WaylandRequest::Minimize(handle),
                        Step::Unminimize(handle) => WaylandRequest::Unminimize(handle),
                    });
                }
            }
        }
        app::Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let showing = self.state.is_showing_desktop();
        // Two icons rather than one: with a single icon there is no way to tell
        // "press to hide" from "press to bring back", and the button is the
        // only thing on screen once the desktop is showing.
        let icon = if showing {
            "view-restore-symbolic"
        } else {
            "user-desktop-symbolic"
        };
        let label = if showing {
            fl!("restore-windows")
        } else {
            fl!("show-desktop")
        };

        let button = self
            .core
            .applet
            .icon_button(icon)
            .on_press(Message::Press);

        autosize(
            tooltip(
                button,
                cosmic::widget::text::body(label),
                tooltip::Position::FollowCursor,
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
