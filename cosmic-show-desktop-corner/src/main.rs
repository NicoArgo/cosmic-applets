// SPDX-License-Identifier: GPL-3.0-only

//! POP Flow — a small triangle in the bottom-left corner of the screen that
//! shows the desktop, like the panel's show-desktop button.
//!
//! It doesn't reimplement anything: a click runs
//! `cosmic-applet-show-desktop --toggle`, which shares its remembered set with
//! the panel button, so the corner and the button are interchangeable — put
//! the windows away with one, bring them back with the other.

mod shape;

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_compositor, delegate_layer, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_shm,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{
            EventLoop,
            timer::{TimeoutAction, Timer},
        },
        calloop_wayland_source::WaylandSource,
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
    },
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{
        Shm, ShmHandler,
        slot::{Buffer, SlotPool},
    },
};
use wayland_client::{
    Connection, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
};

use shape::{HOVER, IDLE, Look, PRESSED, SIZE};

const BTN_LEFT: u32 = 0x110;
const TOGGLE: &str = "cosmic-applet-show-desktop";
/// How often to look for a new accent color (the theme can change under us —
/// by hand in Settings, or by cosmic-wallsync following the wallpaper).
const THEME_POLL: Duration = Duration::from_secs(3);
/// Used until the theme can be read.
const FALLBACK_RGB: [f32; 3] = [0.58, 0.77, 0.99];

fn main() {
    let conn = Connection::connect_to_env().expect("no Wayland display");
    let (globals, event_queue) = registry_queue_init(&conn).expect("registry");
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh).expect("wl_compositor");
    let layer_shell = LayerShell::bind(&globals, &qh).expect("layer shell");
    let shm = Shm::bind(&globals, &qh).expect("wl_shm");

    let surface = compositor.create_surface(&qh);
    // Top: above ordinary windows (the corner must work while they cover it),
    // below fullscreen ones (a video shouldn't wear a triangle).
    let layer = layer_shell.create_layer_surface(
        &qh,
        surface,
        Layer::Top,
        Some("pop-flow-show-desktop-corner"),
        None,
    );
    layer.set_anchor(Anchor::BOTTOM | Anchor::LEFT);
    layer.set_size(SIZE, SIZE);
    // -1: sit in the very corner even if something reserves that edge.
    layer.set_exclusive_zone(-1);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    let region = Region::new(&compositor).expect("wl_region");
    for (x, y, w, h) in shape::input_staircase(4) {
        region.add(x, y, w, h);
    }
    layer.wl_surface().set_input_region(Some(region.wl_region()));
    layer.commit();

    let pool = SlotPool::new((SIZE * SIZE * 4) as usize, &shm).expect("shm pool");

    let mut corner = Corner {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        layer,
        pool,
        buffer: None,
        configured: false,
        scale: 1,
        pointer: None,
        hover: false,
        pressed: false,
        accent: Accent::load(),
        exit: false,
    };

    let mut event_loop: EventLoop<Corner> = EventLoop::try_new().expect("event loop");
    WaylandSource::new(conn, event_queue)
        .insert(event_loop.handle())
        .expect("wayland source");
    event_loop
        .handle()
        .insert_source(Timer::from_duration(THEME_POLL), |_, _, corner| {
            if corner.accent.refresh() {
                corner.draw();
            }
            TimeoutAction::ToDuration(THEME_POLL)
        })
        .expect("timer");

    while !corner.exit {
        event_loop.dispatch(None, &mut corner).expect("dispatch");
    }
}

/// The theme's accent color, re-read only when its file changes.
struct Accent {
    rgb: [f32; 3],
    stamp: Option<(PathBuf, SystemTime)>,
}

impl Accent {
    fn load() -> Self {
        let mut a = Accent {
            rgb: FALLBACK_RGB,
            stamp: None,
        };
        a.refresh();
        a
    }

    /// `~/.config/cosmic/com.system76.CosmicTheme.{Dark,Light}/v1/accent`,
    /// following the current mode. `v1` is what the installed COSMIC writes.
    fn path() -> Option<PathBuf> {
        let cosmic = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?
            .join("cosmic");
        let dark = std::fs::read_to_string(cosmic.join("com.system76.CosmicTheme.Mode/v1/is_dark"))
            .map(|s| s.trim() != "false")
            .unwrap_or(true);
        let theme = if dark { "Dark" } else { "Light" };
        Some(cosmic.join(format!("com.system76.CosmicTheme.{theme}/v1/accent")))
    }

    /// True when the color changed.
    fn refresh(&mut self) -> bool {
        let Some(path) = Self::path() else {
            return false;
        };
        let Ok(mtime) = std::fs::metadata(&path).and_then(|m| m.modified()) else {
            return false;
        };
        let stamp = Some((path.clone(), mtime));
        if stamp == self.stamp {
            return false;
        }
        self.stamp = stamp;
        match std::fs::read_to_string(&path).ok().and_then(|s| shape::parse_accent(&s)) {
            Some(rgb) if rgb != self.rgb => {
                self.rgb = rgb;
                true
            }
            _ => false,
        }
    }
}

struct Corner {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    layer: LayerSurface,
    pool: SlotPool,
    /// The buffer on screen. Replacing it drops the old one, which destroys
    /// its wl_buffer once the compositor releases it — nothing piles up in the
    /// compositor over a long session.
    buffer: Option<Buffer>,
    configured: bool,
    scale: u32,
    pointer: Option<wl_pointer::WlPointer>,
    hover: bool,
    pressed: bool,
    accent: Accent,
    exit: bool,
}

impl Corner {
    fn look(&self) -> Look {
        match (self.hover, self.pressed) {
            (_, true) => PRESSED,
            (true, false) => HOVER,
            _ => IDLE,
        }
    }

    fn draw(&mut self) {
        if !self.configured {
            return;
        }
        let side = (SIZE * self.scale) as i32;
        let pixels = shape::render(self.look(), self.accent.rgb, self.scale);
        let (buffer, canvas) = match self.pool.create_buffer(
            side,
            side,
            side * 4,
            wl_shm::Format::Argb8888,
        ) {
            Ok(b) => b,
            Err(err) => {
                eprintln!("buffer: {err}");
                return;
            }
        };
        canvas.copy_from_slice(&pixels);

        let surface = self.layer.wl_surface();
        surface.set_buffer_scale(self.scale as i32);
        surface.damage_buffer(0, 0, side, side);
        if let Err(err) = buffer.attach_to(surface) {
            eprintln!("attach: {err}");
            return;
        }
        self.layer.commit();
        self.buffer = Some(buffer);
    }

    fn toggle(&self) {
        match std::process::Command::new(TOGGLE).arg("--toggle").spawn() {
            // Reap it off-thread so no zombie lingers; the result is the
            // toggle's business, it logs its own failures.
            Ok(mut child) => {
                std::thread::spawn(move || child.wait());
            }
            Err(err) => eprintln!("{TOGGLE} --toggle: {err}"),
        }
    }
}

impl CompositorHandler for Corner {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        let scale = new_factor.max(1) as u32;
        if scale != self.scale {
            self.scale = scale;
            self.draw();
        }
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for Corner {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for Corner {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        // The output went away (or the compositor restarted); systemd brings
        // us back with a fresh surface.
        self.exit = true;
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &LayerSurface,
        _: LayerSurfaceConfigure,
        _: u32,
    ) {
        if !self.configured {
            self.configured = true;
            self.draw();
        }
    }
}

impl SeatHandler for Corner {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seat_state.get_pointer(qh, &seat).ok();
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer {
            if let Some(p) = self.pointer.take() {
                p.release();
            }
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl PointerHandler for Corner {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        let before = (self.hover, self.pressed);
        for event in events {
            if &event.surface != self.layer.wl_surface() {
                continue;
            }
            let (x, y) = event.position;
            let on_it = shape::inside(x, y, SIZE as f64);
            match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    self.hover = on_it;
                    if !on_it {
                        self.pressed = false;
                    }
                }
                PointerEventKind::Leave { .. } => {
                    self.hover = false;
                    self.pressed = false;
                }
                PointerEventKind::Press { button, .. } if button == BTN_LEFT && on_it => {
                    self.pressed = true;
                }
                // Act on release, like a button: pressing and sliding off is
                // a way to change your mind.
                PointerEventKind::Release { button, .. } if button == BTN_LEFT => {
                    if self.pressed && on_it {
                        self.toggle();
                    }
                    self.pressed = false;
                }
                _ => {}
            }
        }
        if (self.hover, self.pressed) != before {
            self.draw();
        }
    }
}

impl ShmHandler for Corner {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Corner {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

delegate_compositor!(Corner);
delegate_output!(Corner);
delegate_shm!(Corner);
delegate_seat!(Corner);
delegate_pointer!(Corner);
delegate_layer!(Corner);
delegate_registry!(Corner);
