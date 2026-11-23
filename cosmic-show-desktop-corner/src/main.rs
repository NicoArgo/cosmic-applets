// SPDX-License-Identifier: GPL-3.0-only

//! POP Flow — a small triangle in the bottom-right corner of every screen that
//! shows the desktop, like the panel's show-desktop button.
//!
//! It doesn't reimplement anything: a click runs
//! `cosmic-applet-show-desktop --toggle`, which shares its remembered sets with
//! the panel button, so the corner and the button are interchangeable — put
//! the windows away with one, bring them back with the other.
//!
//! There is one triangle per output, and each knows its output's name. With
//! the show-desktop setting on "this screen" (the default), a triangle runs
//! `--toggle --output <its screen>`, so only that screen's windows go away;
//! with "all screens", plain `--toggle`. A right click on the triangle flips
//! the setting and says so in a notification; with "all screens" the triangle
//! carries a thin notch, two layers instead of one.
//!
//! Resting the pointer on it for a moment (400 ms) does the same as a click,
//! like a hot corner. It fires once per visit: the pointer has to leave before
//! the corner will act on hover again, so sitting there doesn't flip the
//! desktop back and forth.
//!
//! It also works while something is being dragged — a file from the file
//! manager, a link from a browser. During a drag the compositor stops sending
//! pointer events and sends drag-and-drop ones instead, so the triangle listens
//! to those too: resting on it with a file in hand shows the desktop, and the
//! file can then be dropped there. The corner never accepts the drop itself.
//!
//! That is the default. The same binary serves any corner with any command —
//! `--corner top-left --exec cosmic-workspaces` is the overview corner — one
//! process per corner, each its own user service (see [`args`]).

mod args;
// Shared with the applet, which owns the setting; plain std, no libcosmic.
#[path = "../../cosmic-applet-show-desktop/src/config.rs"]
mod config;
mod shape;

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    data_device_manager::{
        DataDeviceManagerState, WritePipe,
        data_device::{DataDevice, DataDeviceHandler},
        data_offer::{DataOfferHandler, DragOffer},
        data_source::DataSourceHandler,
    },
    delegate_compositor, delegate_data_device, delegate_layer, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat, delegate_shm,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{
            EventLoop, LoopHandle, RegistrationToken,
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
    protocol::{
        wl_data_device::WlDataDevice, wl_data_device_manager::DndAction,
        wl_data_source::WlDataSource, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface,
    },
};

use args::Options;
use shape::{HOVER, IDLE, Look, PRESSED, SIZE, ScreenCorner};

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const NOTIFY_ICON: &str = "user-desktop-symbolic";
/// How often to look for a new accent color (the theme can change under us —
/// by hand in Settings, or by cosmic-wallsync following the wallpaper).
const THEME_POLL: Duration = Duration::from_secs(3);
/// Used until the theme can be read.
const FALLBACK_RGB: [f32; 3] = [0.58, 0.77, 0.99];

fn main() {
    let options = match args::parse(&std::env::args().skip(1).collect::<Vec<_>>()) {
        Ok(Some(options)) => options,
        Ok(None) => {
            println!("{}", args::USAGE);
            return;
        }
        Err(err) => {
            eprintln!("{err}\n\n{}", args::USAGE);
            std::process::exit(2);
        }
    };

    let conn = Connection::connect_to_env().expect("no Wayland display");
    let (globals, event_queue) = registry_queue_init(&conn).expect("registry");
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh).expect("wl_compositor");
    let layer_shell = LayerShell::bind(&globals, &qh).expect("layer shell");
    let shm = Shm::bind(&globals, &qh).expect("wl_shm");
    // Optional: without it the corner still works, just not mid-drag.
    let data_device_manager = DataDeviceManagerState::bind(&globals, &qh)
        .map_err(|err| eprintln!("no wl_data_device_manager, corner ignores drags: {err}"))
        .ok();

    // One buffer's worth to start; the pool grows as surfaces (one per
    // output) and scales need more.
    let pool = SlotPool::new((SIZE * SIZE * 4) as usize, &shm).expect("shm pool");
    let event_loop: EventLoop<Corner> = EventLoop::try_new().expect("event loop");

    // The surfaces themselves come from `new_output`, one per screen, as the
    // compositor announces them during the first dispatches.
    let mut corner = Corner {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        compositor,
        layer_shell,
        shm,
        pool,
        surfaces: Vec::new(),
        next_id: 0,
        pointer: None,
        data_device_manager,
        data_device: None,
        drag_over: None,
        accent: Accent::load(),
        per_output: config::per_output(),
        loop_handle: event_loop.handle(),
        options,
        exit: false,
    };

    let mut event_loop = event_loop;
    WaylandSource::new(conn, event_queue)
        .insert(event_loop.handle())
        .expect("wayland source");
    event_loop
        .handle()
        .insert_source(Timer::from_duration(THEME_POLL), |_, _, corner| {
            // The setting too: COSMIC Settings can flip it under us.
            let per_output = config::per_output();
            let mode_changed = per_output != corner.per_output;
            corner.per_output = per_output;
            if corner.accent.refresh() || (mode_changed && corner.options.is_show_desktop()) {
                corner.draw_all();
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
        match std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| shape::parse_accent(&s))
        {
            Some(rgb) if rgb != self.rgb => {
                self.rgb = rgb;
                true
            }
            _ => false,
        }
    }
}

/// The triangle on one output.
struct Surface {
    /// Stable within this process, for timers to find their surface again
    /// after others came and went.
    id: u64,
    output: wl_output::WlOutput,
    /// The output's name (`eDP-1`), handed to the command.
    name: Option<String>,
    layer: LayerSurface,
    /// The buffer on screen. Replacing it drops the old one, which destroys
    /// its wl_buffer once the compositor releases it — nothing piles up in the
    /// compositor over a long session. Dropped with the surface when its
    /// output goes.
    buffer: Option<Buffer>,
    configured: bool,
    scale: u32,
    hover: bool,
    /// Left button held.
    pressed: bool,
    /// Right button held (flips the setting on release).
    menu_pressed: bool,
    /// The pending hover timer, while the pointer rests on the corner.
    dwell: Option<RegistrationToken>,
    /// The hover already acted during this visit; cleared when the pointer
    /// leaves.
    dwell_fired: bool,
    /// A `--toggle --wait` started when the pointer arrived, already holding
    /// the window list; acting just tells it to go. See [`Corner::prefetch`].
    prefetch: Option<std::process::Child>,
}

impl Surface {
    fn look(&self) -> Look {
        match (self.hover, self.pressed || self.menu_pressed) {
            (_, true) => PRESSED,
            (true, false) => HOVER,
            _ => IDLE,
        }
    }
}

struct Corner {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    /// Shared by every surface's buffers.
    pool: SlotPool,
    surfaces: Vec<Surface>,
    next_id: u64,
    pointer: Option<wl_pointer::WlPointer>,
    data_device_manager: Option<DataDeviceManagerState>,
    /// The seat's data device: drag-and-drop events arrive here, not on the
    /// pointer.
    data_device: Option<DataDevice>,
    /// The surface (by id) a drag is currently over.
    drag_over: Option<u64>,
    accent: Accent,
    /// The show-desktop setting as last read: only for drawing. Acting reads
    /// the file afresh.
    per_output: bool,
    loop_handle: LoopHandle<'static, Corner>,
    options: Options,
    exit: bool,
}

impl Corner {
    fn add_surface(&mut self, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        if self.surfaces.iter().any(|s| s.output == output) {
            return;
        }
        let info = self.output_state.info(&output);
        let name = info.as_ref().and_then(|info| info.name.clone());
        let scale = info
            .as_ref()
            .map_or(1, |info| info.scale_factor.max(1) as u32);

        let surface = self.compositor.create_surface(qh);
        // Top: above ordinary windows (the corner must work while they cover
        // it), below fullscreen ones (a video shouldn't wear a triangle).
        // The original keeps its name; the others are named for their corner.
        let namespace = match self.options.corner {
            ScreenCorner::BottomRight => "pop-flow-show-desktop-corner".to_string(),
            corner => format!("pop-flow-hot-corner-{}", corner.name()),
        };
        let layer = self.layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Top,
            Some(namespace),
            Some(&output),
        );
        layer.set_anchor(match self.options.corner {
            ScreenCorner::TopLeft => Anchor::TOP | Anchor::LEFT,
            ScreenCorner::TopRight => Anchor::TOP | Anchor::RIGHT,
            ScreenCorner::BottomLeft => Anchor::BOTTOM | Anchor::LEFT,
            ScreenCorner::BottomRight => Anchor::BOTTOM | Anchor::RIGHT,
        });
        layer.set_size(SIZE, SIZE);
        // -1: sit in the very corner even if something reserves that edge.
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        match Region::new(&self.compositor) {
            Ok(region) => {
                for (x, y, w, h) in shape::input_staircase(self.options.corner, 4) {
                    region.add(x, y, w, h);
                }
                layer
                    .wl_surface()
                    .set_input_region(Some(region.wl_region()));
                // The surface keeps its own copy; the region is destroyed
                // when it drops here.
            }
            Err(err) => eprintln!("wl_region: {err}"),
        }
        layer.commit();

        eprintln!(
            "{} corner on {}",
            self.options.corner.name(),
            name.as_deref().unwrap_or("an unnamed output")
        );
        let id = self.next_id;
        self.next_id += 1;
        self.surfaces.push(Surface {
            id,
            output,
            name,
            layer,
            buffer: None,
            configured: false,
            scale,
            hover: false,
            pressed: false,
            menu_pressed: false,
            dwell: None,
            dwell_fired: false,
            prefetch: None,
        });
    }

    /// Drop a surface: its layer (destroying the wl_surface), its buffer and
    /// any pending timer go with it.
    fn remove_surface(&mut self, index: usize) {
        let mut surface = self.surfaces.remove(index);
        if let Some(token) = surface.dwell {
            self.loop_handle.remove(token);
        }
        cancel_prefetch(&mut surface.prefetch);
    }

    fn index_of(&self, wl_surface: &wl_surface::WlSurface) -> Option<usize> {
        self.surfaces
            .iter()
            .position(|s| s.layer.wl_surface() == wl_surface)
    }

    fn draw_all(&mut self) {
        for index in 0..self.surfaces.len() {
            self.draw(index);
        }
    }

    fn draw(&mut self, index: usize) {
        let all_screens = self.options.is_show_desktop() && !self.per_output;
        let Some(surface) = self.surfaces.get_mut(index) else {
            return;
        };
        if !surface.configured {
            return;
        }
        let side = (SIZE * surface.scale) as i32;
        let pixels = shape::render_mode(
            self.options.corner,
            surface.look(),
            self.accent.rgb,
            surface.scale,
            all_screens,
        );
        let (buffer, canvas) =
            match self
                .pool
                .create_buffer(side, side, side * 4, wl_shm::Format::Argb8888)
            {
                Ok(b) => b,
                Err(err) => {
                    eprintln!("buffer: {err}");
                    return;
                }
            };
        canvas.copy_from_slice(&pixels);

        let wl_surface = surface.layer.wl_surface();
        wl_surface.set_buffer_scale(surface.scale as i32);
        wl_surface.damage_buffer(0, 0, side, side);
        if let Err(err) = buffer.attach_to(wl_surface) {
            eprintln!("attach: {err}");
            return;
        }
        surface.layer.commit();
        surface.buffer = Some(buffer);
    }

    /// Start counting when the pointer arrives; forget it when it goes.
    fn track_dwell(&mut self, index: usize) {
        self.prefetch(index);
        let Some(dwell) = self.options.dwell else {
            return;
        };
        let Some(surface) = self.surfaces.get_mut(index) else {
            return;
        };
        if surface.hover && !surface.dwell_fired && surface.dwell.is_none() {
            let id = surface.id;
            let token =
                self.loop_handle
                    .insert_source(Timer::from_duration(dwell), move |_, _, corner| {
                        let Some(index) = corner.surfaces.iter().position(|s| s.id == id) else {
                            return TimeoutAction::Drop;
                        };
                        let surface = &mut corner.surfaces[index];
                        surface.dwell = None;
                        if surface.hover
                            && !surface.pressed
                            && !surface.menu_pressed
                            && !surface.dwell_fired
                        {
                            surface.dwell_fired = true;
                            corner.act(index);
                        }
                        TimeoutAction::Drop
                    });
            match token {
                Ok(token) => surface.dwell = Some(token),
                Err(err) => eprintln!("dwell timer: {err}"),
            }
        } else if !surface.hover {
            surface.dwell_fired = false;
            if let Some(token) = surface.dwell.take() {
                self.loop_handle.remove(token);
            }
        }
    }

    /// Show-desktop corner: while the pointer is on the triangle, keep a
    /// `--toggle --wait` running, so the window list is already in hand when
    /// the rest ends or the click lands. COSMIC sends that list 45 to 145 ms
    /// after a client connects (measured 23/11/2026); started at act time, as
    /// before, that wait came on top of the rest, every time.
    ///
    /// Only for the default command: a custom `--exec` runs as it always did.
    fn prefetch(&mut self, index: usize) {
        if self.options.exec != args::DEFAULT_EXEC {
            return;
        }
        let Some(surface) = self.surfaces.get_mut(index) else {
            return;
        };
        if !surface.hover {
            cancel_prefetch(&mut surface.prefetch);
            return;
        }
        if surface.prefetch.is_some() {
            return;
        }
        // Always with the output: the toggle reads the "this screen / all"
        // setting itself when it acts, so a right click in between still counts.
        let exec = format!(
            "{} --wait",
            args::command_for(args::DEFAULT_EXEC, surface.name.as_deref(), true)
        );
        match std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("exec {exec}"))
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            Ok(child) => surface.prefetch = Some(child),
            Err(err) => eprintln!("{exec}: {err}"),
        }
    }

    /// Run the corner's command for the screen it was triggered on (`--exec`,
    /// through `sh -c` so it can carry arguments the way a desktop shortcut
    /// would).
    fn act(&mut self, index: usize) {
        if let Some(mut child) = self.surfaces.get_mut(index).and_then(|s| s.prefetch.take()) {
            use std::io::Write;
            let sent = child
                .stdin
                .take()
                .is_some_and(|mut stdin| stdin.write_all(b"go\n").is_ok());
            reap(child, "show-desktop toggle");
            if sent {
                // Still on the triangle: the next click gets its list ready too.
                self.prefetch(index);
                return;
            }
        }
        let name = self.surfaces.get(index).and_then(|s| s.name.as_deref());
        // Read now, not from the cached copy: the setting may have changed in
        // Settings a moment ago.
        let exec = args::command_for(&self.options.exec, name, config::per_output());
        match std::process::Command::new("sh")
            .arg("-c")
            .arg(&exec)
            .spawn()
        {
            // Reap it off-thread so no zombie lingers, and say how it ended:
            // a command that fails silently looks exactly like a dead corner.
            Ok(mut child) => {
                std::thread::spawn(move || match child.wait() {
                    Ok(status) if status.success() => {}
                    Ok(status) => eprintln!("{exec} exited with {status}"),
                    Err(err) => eprintln!("{exec}: {err}"),
                });
            }
            Err(err) => eprintln!("{exec}: {err}"),
        }
    }

    /// Right click: switch between "this screen" and "all screens", and say
    /// which it is now.
    fn flip_scope(&mut self) {
        let per_output = !config::per_output();
        if let Err(err) = config::set_per_output(per_output) {
            eprintln!("could not save the show-desktop setting: {err}");
            return;
        }
        self.per_output = per_output;
        self.draw_all();
        let summary = if per_output {
            "Mostrar área de trabalho: só esta tela"
        } else {
            "Mostrar área de trabalho: todas as telas"
        };
        // Off the event loop: notify-send waits for the daemon's answer.
        std::thread::spawn(move || notify(summary));
    }
}

/// The pointer left without acting: closing its stdin tells the waiting
/// toggle to exit without touching anything.
fn cancel_prefetch(prefetch: &mut Option<std::process::Child>) {
    if let Some(mut child) = prefetch.take() {
        drop(child.stdin.take());
        reap(child, "show-desktop prefetch");
    }
}

/// Wait for a child off the event loop, so no zombie lingers, and say how it
/// ended: a command that fails silently looks exactly like a dead corner.
fn reap(mut child: std::process::Child, what: &'static str) {
    std::thread::spawn(move || match child.wait() {
        Ok(status) if status.success() => {}
        Ok(status) => eprintln!("{what} exited with {status}"),
        Err(err) => eprintln!("{what}: {err}"),
    });
}

/// notify-send when there is one; otherwise straight to the notification
/// daemon over the session bus, through busctl, which systemd always brings.
fn notify(summary: &str) {
    let sent = std::process::Command::new("notify-send")
        .args([
            "--app-name=POP Flow",
            &format!("--icon={NOTIFY_ICON}"),
            summary,
        ])
        .status()
        .is_ok_and(|status| status.success());
    if sent {
        return;
    }
    let status = std::process::Command::new("busctl")
        .args([
            "--user",
            "call",
            // So the trailing -1 (no expiry preference) isn't read as a flag.
            "--",
            "org.freedesktop.Notifications",
            "/org/freedesktop/Notifications",
            "org.freedesktop.Notifications",
            "Notify",
            "susssasa{sv}i",
            "POP Flow",
            "0",
            NOTIFY_ICON,
            summary,
            "",
            "0",
            "0",
            "-1",
        ])
        .stdout(std::process::Stdio::null())
        .status();
    if !status.is_ok_and(|status| status.success()) {
        eprintln!("{summary} (no notification could be sent)");
    }
}

impl CompositorHandler for Corner {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        let scale = new_factor.max(1) as u32;
        if let Some(index) = self.index_of(surface)
            && scale != self.surfaces[index].scale
        {
            self.surfaces[index].scale = scale;
            self.draw(index);
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
    fn new_output(&mut self, _: &Connection, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        self.add_surface(qh, output);
    }

    fn update_output(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        // Names don't usually change, but they arrive in the same burst as
        // the output; keep the latest.
        let name = self.output_state.info(&output).and_then(|info| info.name);
        if let Some(surface) = self.surfaces.iter_mut().find(|s| s.output == output) {
            surface.name = name;
        }
    }

    fn output_destroyed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if let Some(index) = self.surfaces.iter().position(|s| s.output == output) {
            eprintln!(
                "{} corner off {}",
                self.options.corner.name(),
                self.surfaces[index]
                    .name
                    .as_deref()
                    .unwrap_or("an unnamed output")
            );
            self.remove_surface(index);
        }
    }
}

impl LayerShellHandler for Corner {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        // Its output is going away; the others stay, and a screen plugged in
        // later gets a fresh triangle through `new_output`. (A compositor
        // restart drops the whole connection instead, and systemd brings us
        // back.)
        if let Some(index) = self.index_of(layer.wl_surface()) {
            self.remove_surface(index);
        }
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        _: LayerSurfaceConfigure,
        _: u32,
    ) {
        if let Some(index) = self.index_of(layer.wl_surface())
            && !self.surfaces[index].configured
        {
            self.surfaces[index].configured = true;
            self.draw(index);
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
        if capability == Capability::Pointer && self.data_device.is_none() {
            if let Some(manager) = &self.data_device_manager {
                self.data_device = Some(manager.get_data_device(qh, &seat));
            }
        }
        if capability == Capability::Pointer && self.pointer.is_none() {
            self.pointer = match self.seat_state.get_pointer(qh, &seat) {
                Ok(pointer) => Some(pointer),
                Err(err) => {
                    eprintln!("no pointer: {err}");
                    None
                }
            };
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
        let before: Vec<_> = self
            .surfaces
            .iter()
            .map(|s| (s.hover, s.pressed, s.menu_pressed))
            .collect();
        let mut flip = false;
        let show_desktop = self.options.is_show_desktop();
        for event in events {
            // One frame can leave one screen's triangle and enter another's.
            let Some(index) = self.index_of(&event.surface) else {
                continue;
            };
            let (x, y) = event.position;
            let on_it = shape::inside(self.options.corner, x, y, SIZE as f64);
            let mut act = false;
            let surface = &mut self.surfaces[index];
            match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    surface.hover = on_it;
                    if !on_it {
                        surface.pressed = false;
                        surface.menu_pressed = false;
                    }
                }
                PointerEventKind::Leave { .. } => {
                    surface.hover = false;
                    surface.pressed = false;
                    surface.menu_pressed = false;
                }
                PointerEventKind::Press { button, .. } if button == BTN_LEFT && on_it => {
                    surface.pressed = true;
                }
                PointerEventKind::Press { button, .. }
                    if button == BTN_RIGHT && on_it && show_desktop =>
                {
                    surface.menu_pressed = true;
                    // Reaching for the setting is not asking to show the
                    // desktop: the hover must not act during this visit.
                    surface.dwell_fired = true;
                }
                // Act on release, like a button: pressing and sliding off is
                // a way to change your mind.
                PointerEventKind::Release { button, .. } if button == BTN_LEFT => {
                    if surface.pressed && on_it {
                        // A click is its own decision: it counts for this
                        // visit, so the hover doesn't undo it right after.
                        surface.dwell_fired = true;
                        act = true;
                    }
                    surface.pressed = false;
                }
                PointerEventKind::Release { button, .. } if button == BTN_RIGHT => {
                    if surface.menu_pressed && on_it {
                        flip = true;
                    }
                    surface.menu_pressed = false;
                }
                _ => {}
            }
            if act {
                self.act(index);
            }
        }
        if flip {
            // Redraws every triangle, the pressed look included.
            self.flip_scope();
        }
        for index in 0..self.surfaces.len() {
            self.track_dwell(index);
            let s = &self.surfaces[index];
            if before.get(index) != Some(&(s.hover, s.pressed, s.menu_pressed)) {
                self.draw(index);
            }
        }
    }
}

impl Corner {
    /// A drag moved over (or off) a triangle: the same hover as the pointer's,
    /// so the same rest-to-act timer runs. Nothing is pressed during a drag.
    fn drag_hover(&mut self, id: Option<u64>, x: f64, y: f64) {
        let before = self.drag_over;
        self.drag_over = id;
        for index in 0..self.surfaces.len() {
            let surface = &mut self.surfaces[index];
            let ours = Some(surface.id) == id || Some(surface.id) == before;
            if !ours {
                continue;
            }
            let hover =
                Some(surface.id) == id && shape::inside(self.options.corner, x, y, SIZE as f64);
            if hover != surface.hover {
                surface.hover = hover;
                self.track_dwell(index);
                self.draw(index);
            }
        }
    }
}

impl DataDeviceHandler for Corner {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataDevice,
        x: f64,
        y: f64,
        wl_surface: &wl_surface::WlSurface,
    ) {
        let id = self.index_of(wl_surface).map(|i| self.surfaces[i].id);
        self.drag_hover(id, x, y);
    }

    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {
        self.drag_hover(None, 0.0, 0.0);
    }

    fn motion(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice, x: f64, y: f64) {
        self.drag_hover(self.drag_over, x, y);
    }

    fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}

    fn drop_performed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {
        // Never accepted, so nothing lands here; just stop hovering.
        self.drag_hover(None, 0.0, 0.0);
    }
}

impl DataOfferHandler for Corner {
    fn source_actions(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }
    fn selected_action(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }
}

// We never offer data; the trait comes with the data device.
impl DataSourceHandler for Corner {
    fn accept_mime(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataSource,
        _: Option<String>,
    ) {
    }
    fn send_request(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataSource,
        _: String,
        _: WritePipe,
    ) {
    }
    fn cancelled(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}
    fn dnd_dropped(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}
    fn dnd_finished(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}
    fn action(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource, _: DndAction) {}
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
delegate_data_device!(Corner);
delegate_registry!(Corner);
