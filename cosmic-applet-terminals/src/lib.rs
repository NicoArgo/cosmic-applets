// SPDX-License-Identifier: GPL-3.0-only

//! POP Flow — a panel button that opens a terminal, and one dot per open
//! terminal window, in the color of the folder rule its active tab is in.
//!
//! The button always opens a new terminal; reaching one that is already open
//! is what the dots are for. A dot's left click brings its window forward
//! (unminimizing it, or switching to its workspace); a right click opens a
//! small menu that can also close it. The work of matching windows to colors
//! is in [`terminals`].

mod localize;
pub mod terminals;
mod wayland;

use cosmic::{
    Element,
    app::{self, Core},
    applet::{menu_button, padded_control},
    cctk::sctk::reexports::calloop,
    iced::{self, Alignment, Background, Border, Color, Length, Limits, Subscription, id::Id as WidgetId, window},
    surface,
    widget::{self, autosize::autosize, divider, mouse_area, text},
};
use std::{sync::LazyLock, time::Duration};

use terminals::{Dot, MAX_DOTS, Presence, Window};
use wayland::{Handle, Request, Update};

static AUTOSIZE_MAIN_ID: LazyLock<WidgetId> = LazyLock::new(|| WidgetId::new("autosize-main"));

/// Same hover growth as the other POP Flow panel buttons.
const HOVER_SCALE: f32 = 1.25;
const SCALE_DURATION: f32 = 0.12;
/// A dot's diameter and slot, as fractions of the panel's icon size.
const DOT: f32 = 1.0;
const DOT_HOVER: f32 = 1.2;
const DOT_SLOT: f32 = 1.35;
/// Window titles change on every prompt and re-read the files with them; this
/// only catches a color edited in a rule while the title stayed the same.
const REFRESH: Duration = Duration::from_secs(3);

pub fn run() -> cosmic::iced::Result {
    localize::localize();
    cosmic::applet::run::<Terminals>(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Button,
    Dot(usize),
}

#[derive(Default)]
struct Terminals {
    core: Core,
    tx: Option<calloop::channel::Sender<Request>>,
    windows: Vec<Window<Handle>>,
    /// Terminal windows in the order they opened — their slots on the panel.
    order: Vec<Handle>,
    presences: Vec<Presence>,
    hovered: Option<Target>,
    scale: f32,
    last_frame: Option<std::time::Instant>,
    /// The right-click menu, and the window it is about.
    menu: Option<(window::Id, Handle)>,
}

#[derive(Clone, Debug)]
enum Message {
    Wayland(Update),
    Refresh,
    Open,
    Activate(Handle),
    Close(Handle),
    Menu(Handle),
    MenuClosed(window::Id),
    Hover(Option<Target>),
    Left(window::Id),
    Frame(std::time::Instant),
    Surface(surface::Action),
}

impl Terminals {
    fn read_presences(&mut self) {
        self.presences = terminals::presence_dir()
            .map(|dir| terminals::read_presences(&dir))
            .unwrap_or_default();
    }

    fn dots(&self) -> Vec<Dot<Handle>> {
        terminals::dots(&self.order, &self.windows, &self.presences)
    }

    fn send(&self, request: Request) {
        match &self.tx {
            Some(tx) => {
                if let Err(err) = tx.send(request) {
                    tracing::error!("failed to reach the wayland thread: {err:?}");
                }
            }
            None => tracing::error!("no wayland thread yet"),
        }
    }

    fn open_new(&self) {
        let mut command = std::process::Command::new("cosmic-term");
        if let Some(home) = dirs::home_dir() {
            command.current_dir(home);
        }
        match command.spawn() {
            Ok(mut child) => {
                std::thread::spawn(move || child.wait());
            }
            Err(err) => tracing::error!("cosmic-term: {err}"),
        }
    }

    fn close_menu(&mut self) -> app::Task<Message> {
        match self.menu.take() {
            Some((id, _)) => cosmic::surface::surface_task(cosmic::surface::action::destroy_popup(id)),
            None => app::Task::none(),
        }
    }

    fn dot_view(&self, index: usize, dot: &Dot<Handle>, icon: f32) -> Element<'_, Message> {
        let hovered = self.hovered == Some(Target::Dot(index));
        let size = (icon * if hovered { DOT_HOVER } else { DOT }).round();
        let accent = dot.accent;
        let active = dot.active;
        let circle = widget::container(widget::Space::new().width(size).height(size)).class(cosmic::theme::Container::custom(
            move |theme| {
                let on: Color = theme.cosmic().background(theme.transparent).on.into();
                let fill = match accent {
                    Some([r, g, b]) => Color::from_rgb8(r, g, b),
                    None => Color { a: 0.4, ..on },
                };
                widget::container::Style {
                    background: Some(Background::Color(fill)),
                    // The focused window's dot wears a ring; every dot gets a
                    // faint edge so a very light color still reads on a light
                    // panel.
                    border: Border {
                        radius: (size / 2.0).into(),
                        width: if active { 2.0 } else { 1.0 },
                        color: if active { on } else { Color { a: 0.35, ..on } },
                    },
                    ..Default::default()
                }
            },
        ));
        // A fixed slot, so a dot growing under the pointer moves nothing.
        let slot = widget::container(circle)
            .width(Length::Fixed((icon * DOT_SLOT).round()))
            .height(Length::Fixed((icon * DOT_HOVER).round()))
            .align_x(Alignment::Center)
            .align_y(Alignment::Center);
        let area = mouse_area(slot)
            .on_press(Message::Activate(dot.handle.clone()))
            .on_right_press(Message::Menu(dot.handle.clone()))
            .on_enter(Message::Hover(Some(Target::Dot(index))))
            .on_exit(Message::Hover(None));
        self.core
            .applet
            .applet_tooltip(area, dot.name.clone(), self.menu.is_some(), Message::Surface, None)
            .into()
    }
}

impl cosmic::Application for Terminals {
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    type Message = Message;

    const APP_ID: &'static str = "com.popflow.CosmicAppletTerminals";

    fn init(core: Core, _flags: ()) -> (Self, app::Task<Message>) {
        let mut applet = Self {
            core,
            scale: 1.0,
            ..Default::default()
        };
        applet.read_presences();
        (applet, app::Task::none())
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
        let target = if self.hovered == Some(Target::Button) {
            HOVER_SCALE
        } else {
            1.0
        };
        let frames = if (self.scale - target).abs() > f32::EPSILON {
            iced::window::frames().map(|(_, at)| Message::Frame(at))
        } else {
            Subscription::none()
        };
        // Mouse areas see the pointer move between the button and the dots,
        // but not leave the whole surface in one motion — see the folder
        // button. The surface's own exit clears whatever was hovered.
        let left = iced::event::listen_with(|event, _, id| match event {
            iced::Event::Mouse(iced::mouse::Event::CursorLeft) => Some(Message::Left(id)),
            _ => None,
        });
        Subscription::batch([
            wayland::subscription().map(Message::Wayland),
            iced::time::every(REFRESH).map(|_| Message::Refresh),
            frames,
            left,
        ])
    }

    fn update(&mut self, message: Message) -> app::Task<Message> {
        match message {
            Message::Wayland(Update::Init(tx)) => self.tx = Some(tx),
            Message::Wayland(Update::Finished) => {
                self.tx = None;
                self.windows.clear();
                self.order.clear();
            }
            Message::Wayland(Update::Windows(windows)) => {
                // The terminal writes its file before it sets the title, so
                // the title change that brought this list is already on disk.
                self.windows = windows;
                terminals::keep_order(&mut self.order, &self.windows);
                self.read_presences();
                if let Some((_, handle)) = &self.menu
                    && !self.order.contains(handle)
                {
                    return self.close_menu();
                }
            }
            Message::Refresh => self.read_presences(),
            Message::Open => {
                self.open_new();
                return self.close_menu();
            }
            Message::Activate(handle) => {
                self.send(Request::Activate(handle));
                return self.close_menu();
            }
            Message::Close(handle) => {
                self.send(Request::Close(handle));
                return self.close_menu();
            }
            Message::Menu(handle) => {
                let same = self.menu.as_ref().is_some_and(|(_, h)| *h == handle);
                let close = self.close_menu();
                if same {
                    return close;
                }
                let open = cosmic::surface::surface_task(cosmic::surface::action::app_popup(
                    |_| Default::default(),
                    move |app: &mut Terminals| {
                        let id = window::Id::unique();
                        app.menu = Some((id, handle.clone()));
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
                return app::Task::batch([close, open]);
            }
            Message::MenuClosed(id) => {
                if self.menu.as_ref().is_some_and(|(menu, _)| *menu == id) {
                    self.menu = None;
                }
            }
            Message::Hover(target) => {
                if target != self.hovered {
                    self.hovered = target;
                    self.last_frame = None;
                }
            }
            Message::Left(id) => {
                if self.core.main_window_id() == Some(id) {
                    self.hovered = None;
                    self.last_frame = None;
                }
            }
            Message::Frame(at) => {
                let dt = self
                    .last_frame
                    .map_or(1.0 / 60.0, |last| at.duration_since(last).as_secs_f32())
                    .min(1.0 / 30.0);
                self.last_frame = Some(at);
                let target = if self.hovered == Some(Target::Button) {
                    HOVER_SCALE
                } else {
                    1.0
                };
                let step = (HOVER_SCALE - 1.0) * dt / SCALE_DURATION;
                self.scale = if self.scale < target {
                    (self.scale + step).min(target)
                } else {
                    (self.scale - step).max(target)
                };
            }
            Message::Surface(action) => {
                return cosmic::task::message(cosmic::Action::Cosmic(cosmic::app::Action::Surface(
                    action,
                )));
            }
        }
        app::Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let suggested = self.core.applet.suggested_size(true);
        let icon_size = suggested.0 as f32;
        let icon = widget::icon(widget::icon::from_name("utilities-terminal-symbolic").symbolic(true).handle())
            .class(cosmic::theme::Svg::custom(|theme| iced::widget::svg::Style {
                color: Some(theme.cosmic().background(theme.transparent).on.into()),
            }))
            .width(Length::Fixed(icon_size * self.scale))
            .height(Length::Fixed(suggested.1 as f32 * self.scale));
        let button = self.core.applet.button_from_element(icon, true).on_press(Message::Open);
        let button = mouse_area(button)
            .on_enter(Message::Hover(Some(Target::Button)))
            .on_exit(Message::Hover(None));
        let button = self.core.applet.applet_tooltip(
            button,
            fl!("open-terminal"),
            self.menu.is_some(),
            Message::Surface,
            None,
        );

        let dots = self.dots();
        let mut items: Vec<Element<'_, Message>> = vec![button.into()];
        for (index, dot) in dots.iter().take(MAX_DOTS).enumerate() {
            items.push(self.dot_view(index, dot, icon_size));
        }
        if dots.len() > MAX_DOTS {
            let more = dots.len() - MAX_DOTS;
            items.push(
                self.core
                    .applet
                    .applet_tooltip(
                        widget::container(text::caption(format!("+{more}")))
                            .height(Length::Fixed(icon_size))
                            .align_y(Alignment::Center)
                            .padding([0, 4]),
                        fl!("more-terminals", count = more.to_string()),
                        self.menu.is_some(),
                        Message::Surface,
                        None,
                    )
                    .into(),
            );
        }

        let content: Element<'_, Message> = if self.core.applet.is_horizontal() {
            widget::Row::with_children(items).align_y(Alignment::Center).into()
        } else {
            widget::Column::with_children(items).align_x(Alignment::Center).into()
        };

        autosize(content, AUTOSIZE_MAIN_ID.clone())
            .limits(Limits::NONE.min_width(1.).min_height(1.))
            .into()
    }

    fn view_window(&self, id: window::Id) -> Element<'_, Message> {
        let Some((_, handle)) = self.menu.as_ref().filter(|(menu, _)| *menu == id) else {
            return text("").into();
        };
        let name = self
            .dots()
            .into_iter()
            .find(|dot| dot.handle == *handle)
            .map(|dot| dot.name)
            .unwrap_or_default();
        let menu = widget::Column::new()
            .push(padded_control(text::heading(name)))
            .push(padded_control(divider::horizontal::default()))
            .push(menu_button(text::body(fl!("bring-forward"))).on_press(Message::Activate(handle.clone())))
            .push(menu_button(text::body(fl!("close-window"))).on_press(Message::Close(handle.clone())));
        self.core.applet.popup_container(menu.padding([8, 0])).into()
    }

    fn style(&self) -> Option<iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}
