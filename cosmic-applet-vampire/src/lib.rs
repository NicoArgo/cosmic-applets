// SPDX-License-Identifier: GPL-3.0-only

//! POP Flow — a panel switch between "vampire mode" (the computer only sleeps
//! when you tell it to) and "sleep mode" (lid and idle may put it to sleep).
//! The work is in [`mode`]; this is the button.

mod localize;
pub mod mode;

use crate::localize::localize;
use cosmic::{
    Element,
    app::{self, Core},
    iced::{self, Limits, Subscription, id::Id as WidgetId},
    surface,
    widget::autosize::autosize,
};
use std::{sync::LazyLock, time::Duration};

static AUTOSIZE_MAIN_ID: LazyLock<WidgetId> = LazyLock::new(|| WidgetId::new("autosize-main"));
static BAT: &[u8] = include_bytes!("bat.svg");

/// The mode can also change from the command line; the button catches up
/// this often.
const REFRESH: Duration = Duration::from_secs(3);
/// Same hover growth as the other POP Flow panel buttons.
const HOVER_SCALE: f32 = 1.25;
const SCALE_DURATION: f32 = 0.12;

pub fn run() -> cosmic::iced::Result {
    localize();
    cosmic::applet::run::<VampireApplet>(())
}

#[derive(Default)]
struct VampireApplet {
    core: Core,
    on: bool,
    /// A switch is running; a second press waits for it.
    busy: bool,
    hovered: bool,
    scale: f32,
    last_frame: Option<std::time::Instant>,
}

#[derive(Clone, Debug)]
enum Message {
    Press,
    Switched(Result<bool, String>),
    Refresh,
    Hover(bool),
    Frame(std::time::Instant),
    Surface(surface::Action),
}

impl cosmic::Application for VampireApplet {
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    type Message = Message;

    const APP_ID: &'static str = "com.popflow.CosmicAppletVampire";

    fn init(core: Core, _flags: ()) -> (Self, app::Task<Message>) {
        (
            Self {
                core,
                on: mode::is_on(),
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
        // From the surface, not a mouse area — see the show-desktop applet.
        let hover = iced::event::listen_with(|event, _, _| match event {
            iced::Event::Mouse(
                iced::mouse::Event::CursorEntered | iced::mouse::Event::CursorMoved { .. },
            ) => Some(Message::Hover(true)),
            iced::Event::Mouse(iced::mouse::Event::CursorLeft) => Some(Message::Hover(false)),
            _ => None,
        });
        Subscription::batch([
            iced::time::every(REFRESH).map(|_| Message::Refresh),
            frames,
            hover,
        ])
    }

    fn update(&mut self, message: Message) -> app::Task<Message> {
        match message {
            Message::Press if !self.busy => {
                self.busy = true;
                return app::Task::perform(
                    async {
                        tokio::task::spawn_blocking(mode::toggle)
                            .await
                            .unwrap_or_else(|err| Err(err.to_string()))
                    },
                    |result| cosmic::Action::App(Message::Switched(result)),
                );
            }
            Message::Press => {}
            Message::Switched(result) => {
                self.busy = false;
                match result {
                    Ok(on) => self.on = on,
                    Err(err) => {
                        tracing::error!("vampire mode switch failed: {err}");
                        self.on = mode::is_on();
                    }
                }
            }
            Message::Refresh => {
                if !self.busy {
                    self.on = mode::is_on();
                }
            }
            Message::Hover(hovered) if hovered != self.hovered => {
                self.hovered = hovered;
                self.last_frame = None;
            }
            Message::Hover(_) => {}
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
            Message::Surface(action) => {
                return cosmic::task::message(cosmic::Action::Cosmic(
                    cosmic::app::Action::Surface(action),
                ));
            }
        }
        app::Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        // A bat while the computer stays up, the moon while it may sleep: the
        // icon says the mode now in effect, not what a press would do.
        let handle = if self.on {
            cosmic::widget::icon::from_svg_bytes(BAT).symbolic(true)
        } else {
            cosmic::widget::icon::from_name("weather-clear-night-symbolic")
                .symbolic(true)
                .handle()
        };
        let suggested = self.core.applet.suggested_size(true);
        let icon = cosmic::widget::icon(handle)
            .class(cosmic::theme::Svg::custom(|theme| iced::widget::svg::Style {
                color: Some(theme.cosmic().background(theme.transparent).on.into()),
            }))
            .width(iced::Length::Fixed(suggested.0 as f32 * self.scale))
            .height(iced::Length::Fixed(suggested.1 as f32 * self.scale));
        let button = self
            .core
            .applet
            .button_from_element(icon, true)
            .on_press(Message::Press);
        let tooltip = if self.on {
            fl!("vampire-on")
        } else {
            fl!("vampire-off")
        };

        autosize(
            self.core
                .applet
                .applet_tooltip(button, tooltip, false, Message::Surface, None),
            AUTOSIZE_MAIN_ID.clone(),
        )
        .limits(Limits::NONE.min_width(1.).min_height(1.))
        .into()
    }

    fn style(&self) -> Option<iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}
