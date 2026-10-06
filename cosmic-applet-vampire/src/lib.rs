// SPDX-License-Identifier: GPL-3.0-only

//! POP Flow — a panel switch between "vampire mode" (the computer only sleeps
//! when you tell it to) and "sleep mode" (lid and idle may put it to sleep).
//! The work is in [`mode`]; this is the button. A left click flips the
//! mode; a right or middle click opens a small menu that also offers it for
//! a while ("awake for 1 h").

mod localize;
pub mod mode;
pub mod watch;

use crate::localize::localize;
use cosmic::{
    Element,
    app::{self, Core},
    applet::{menu_button, padded_control},
    iced::{self, Limits, Subscription, id::Id as WidgetId, window},
    surface,
    widget::{autosize::autosize, divider, mouse_area, text},
};
use std::{
    sync::LazyLock,
    time::{Duration, SystemTime},
};

static AUTOSIZE_MAIN_ID: LazyLock<WidgetId> = LazyLock::new(|| WidgetId::new("autosize-main"));
static BAT: &[u8] = include_bytes!("bat.svg");

/// The mode can also change from the command line; the button catches up
/// this often.
const REFRESH: Duration = Duration::from_secs(3);
/// Same hover growth as the other POP Flow panel buttons.
const HOVER_SCALE: f32 = 1.25;
const SCALE_DURATION: f32 = 0.12;
/// The menu's "awake for a while" choices, in minutes.
const FOR_A_WHILE: [u32; 2] = [60, 180];

pub fn run() -> cosmic::iced::Result {
    localize();
    cosmic::applet::run::<VampireApplet>(())
}

#[derive(Default)]
struct VampireApplet {
    core: Core,
    on: bool,
    /// When a temporary mode ends; `None` when off or on for good.
    deadline: Option<SystemTime>,
    /// A switch is running; a second press waits for it.
    busy: bool,
    hovered: bool,
    scale: f32,
    last_frame: Option<std::time::Instant>,
    popup: Option<window::Id>,
}

/// What the menu (or the button) can ask for.
#[derive(Clone, Copy, Debug)]
enum Choice {
    Toggle,
    On,
    Off,
    For(u32),
}

impl Choice {
    fn run(self) -> Result<bool, String> {
        match self {
            Choice::Toggle => mode::toggle(),
            Choice::On => mode::set(true).map(|()| true),
            Choice::Off => mode::set(false).map(|()| false),
            Choice::For(minutes) => mode::set_for(minutes).map(|()| true),
        }
    }
}

#[derive(Clone, Debug)]
enum Message {
    Press,
    Choose(Choice),
    Menu,
    MenuClosed(window::Id),
    Switched(Result<bool, String>),
    Refresh,
    Hover(window::Id, bool),
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
                deadline: mode::deadline(),
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

    fn on_close_requested(&self, id: window::Id) -> Option<Message> {
        Some(Message::MenuClosed(id))
    }

    fn subscription(&self) -> Subscription<Message> {
        let target = if self.hovered { HOVER_SCALE } else { 1.0 };
        let frames = if (self.scale - target).abs() > f32::EPSILON {
            iced::window::frames().map(|(_, at)| Message::Frame(at))
        } else {
            Subscription::none()
        };
        // From the surface, not a mouse area — see the show-desktop applet.
        // Tagged with the surface: the menu is one too, and the pointer in it
        // isn't on the button.
        let hover = iced::event::listen_with(|event, _, id| match event {
            iced::Event::Mouse(
                iced::mouse::Event::CursorEntered | iced::mouse::Event::CursorMoved { .. },
            ) => Some(Message::Hover(id, true)),
            iced::Event::Mouse(iced::mouse::Event::CursorLeft) => Some(Message::Hover(id, false)),
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
            Message::Press => return self.update(Message::Choose(Choice::Toggle)),
            Message::Choose(choice) => {
                let close = self.close_menu();
                if self.busy {
                    return close;
                }
                self.busy = true;
                let switch = app::Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || choice.run())
                            .await
                            .unwrap_or_else(|err| Err(err.to_string()))
                    },
                    |result| cosmic::Action::App(Message::Switched(result)),
                );
                return app::Task::batch([close, switch]);
            }
            Message::Menu => {
                if self.popup.is_some() {
                    return self.close_menu();
                }
                return cosmic::surface::surface_task(cosmic::surface::action::app_popup(
                    |_| Default::default(),
                    |app: &mut VampireApplet| {
                        let id = window::Id::unique();
                        app.popup = Some(id);
                        app.core.applet.get_popup_settings(
                            app.core.main_window_id().unwrap(),
                            id,
                            None,
                            None,
                            None,
                        )
                    },
                    None,
                ));
            }
            Message::MenuClosed(id) => {
                if self.popup == Some(id) {
                    self.popup = None;
                }
            }
            Message::Switched(result) => {
                self.busy = false;
                if let Err(err) = result {
                    tracing::error!("vampire mode switch failed: {err}");
                }
                self.read_mode();
            }
            Message::Refresh => {
                if !self.busy {
                    self.read_mode();
                }
            }
            Message::Hover(id, hovered)
                if hovered != self.hovered && Some(id) != self.popup =>
            {
                self.hovered = hovered;
                self.last_frame = None;
            }
            Message::Hover(..) => {}
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
        let button = mouse_area(button)
            .on_right_press(Message::Menu)
            .on_middle_press(Message::Menu);
        let tooltip = match (self.on, self.deadline) {
            (true, Some(at)) => fl!(
                "vampire-on-for",
                left = mode::format_remaining(mode::remaining(at, SystemTime::now()))
            ),
            (true, None) => fl!("vampire-on"),
            (false, _) => fl!("vampire-off"),
        };

        autosize(
            self.core.applet.applet_tooltip(
                button,
                tooltip,
                self.popup.is_some(),
                Message::Surface,
                None,
            ),
            AUTOSIZE_MAIN_ID.clone(),
        )
        .limits(Limits::NONE.min_width(1.).min_height(1.))
        .into()
    }

    fn view_window(&self, id: window::Id) -> Element<'_, Message> {
        if self.popup != Some(id) {
            return text("").into();
        }
        let item = |label: String, choice| {
            menu_button(text::body(label))
                .on_press_maybe((!self.busy).then_some(Message::Choose(choice)))
        };
        let mut menu = cosmic::widget::Column::new();
        for minutes in FOR_A_WHILE {
            menu = menu.push(item(
                fl!("awake-for", hours = (minutes / 60).to_string()),
                Choice::For(minutes),
            ));
        }
        menu = menu.push(padded_control(divider::horizontal::default()));
        // The rest of the menu is the plain switch, worded for where the
        // mode is now; a temporary mode can also be made permanent.
        menu = match (self.on, self.deadline) {
            (false, _) => menu.push(item(fl!("menu-on"), Choice::On)),
            (true, Some(_)) => menu
                .push(item(fl!("menu-keep-on"), Choice::On))
                .push(item(fl!("menu-off"), Choice::Off)),
            (true, None) => menu.push(item(fl!("menu-off"), Choice::Off)),
        };
        self.core
            .applet
            .popup_container(menu.padding([8, 0]))
            .into()
    }

    fn style(&self) -> Option<iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}

impl VampireApplet {
    fn read_mode(&mut self) {
        self.on = mode::is_on();
        self.deadline = if self.on { mode::deadline() } else { None };
    }

    fn close_menu(&mut self) -> app::Task<Message> {
        match self.popup.take() {
            Some(id) => cosmic::surface::surface_task(cosmic::surface::action::destroy_popup(id)),
            None => app::Task::none(),
        }
    }
}
