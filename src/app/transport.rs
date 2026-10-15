//! Invisible scene hotspots with finite, baked mechanical button feedback.
use super::{lifecycle::SharedRenderer, state::AppState};
use crate::playback::{
    mpris::Monitor,
    transport::{Command, Target},
};
use crate::transport_geometry::Ellipse;
use gtk::{glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Instant,
};

struct Pressed {
    command: Command,
    target: Target,
    revision: u64,
    ellipse: Ellipse,
    scene: crate::config::SceneVariant,
}

struct DesktopReceiver {
    window: glib::WeakRef<gtk::ApplicationWindow>,
    widget: glib::WeakRef<gtk::DrawingArea>,
    placement: crate::desktop::SurfacePlacement,
    /// Receiver-local coordinates translated into scene-GLArea coordinates.
    origin: Cell<[i32; 2]>,
    size: Cell<[i32; 2]>,
}

pub(super) struct TransportUi {
    area: glib::WeakRef<gtk::GLArea>,
    window: glib::WeakRef<gtk::ApplicationWindow>,
    state: Rc<RefCell<AppState>>,
    renderer: SharedRenderer,
    monitor: Monitor,
    desktop: bool,
    direct_input: bool,
    receiver: Option<DesktopReceiver>,
    pressed: RefCell<Option<Pressed>>,
    settings: RefCell<Option<glib::WeakRef<gtk::ApplicationWindow>>>,
    settings_tray: RefCell<Option<std::rc::Weak<super::tray::TrayUi>>>,
    pressed_logo: Cell<Option<Ellipse>>,
    pointer: Cell<Option<(f64, f64)>>,
    input: RefCell<Option<Vec<[i32; 4]>>>,
    animations: RefCell<[crate::button_motion::ButtonMotion; 3]>,
    knob_motion: RefCell<crate::knob_motion::KnobMotion>,
    animation_running: Cell<bool>,
    shell_notify: RefCell<Option<Rc<dyn Fn()>>>,
    weak: std::rc::Weak<Self>,
}
impl TransportUi {
    pub(super) fn set_settings(
        &self,
        window: Option<&gtk::ApplicationWindow>,
        tray: Option<&Rc<super::tray::TrayUi>>,
    ) {
        *self.settings.borrow_mut() = window.map(|w| w.downgrade());
        *self.settings_tray.borrow_mut() = tray.map(Rc::downgrade);
        self.sync();
    }
    pub(super) fn open_settings(&self) {
        if let Some(tray) = self
            .settings_tray
            .borrow()
            .as_ref()
            .and_then(|t| t.upgrade())
        {
            tray.show();
        } else if let Some(window) = self.settings.borrow().as_ref().and_then(|w| w.upgrade()) {
            window.unminimize();
            window.present();
        }
    }
    fn logo_ellipse(&self) -> Option<Ellipse> {
        self.settings.borrow().as_ref()?.upgrade()?;
        let area = self.area.upgrade()?;
        if area.error().is_some() || super::visibility::suspended(&area) {
            return None;
        }
        crate::transport_geometry::TransportSettings::project_logo(
            area.width(),
            area.height(),
            area.scale_factor(),
            self.renderer.borrow().as_ref()?.scene_aspect(),
        )
    }
    pub(super) fn shell_layout(&self) -> super::shell_bridge::Layout {
        use super::shell_bridge::{Hotspot, Layout};
        let mut layout = Layout::default();
        let Some(area) = self.area.upgrade() else {
            return layout;
        };
        layout.width = area.width();
        layout.height = area.height();
        layout.viewport = self.renderer.borrow().as_ref().and_then(|renderer| {
            super::shell_bridge::scene_viewport(
                layout.width,
                layout.height,
                area.scale_factor(),
                renderer.scene_aspect(),
            )
        });
        let scale = area.scale_factor();
        layout.animation = self.renderer.borrow().as_ref().and_then(|renderer| {
            let width = layout.width.checked_mul(scale)?;
            let height = layout.height.checked_mul(scale)?;
            let rect = crate::desktop::geometry::Rect::from_gl(
                renderer.regional_rect(width, height, scale)?,
                height,
            )?;
            Some([rect.x, rect.y, rect.w, rect.h].map(|n| n as f64 / scale as f64))
        });
        let s = self.state.borrow();
        layout.pause_when_hidden = s.config.render.pause_when_hidden;
        let f = s.playback.borrow();
        let now = Instant::now();
        // Exclude observation timestamps and volume value: routine polls and
        // knob animation must not rebuild Shell input actors or cancel a press.
        layout.key = format!(
            "{:?}/{}/{:?}/{:?}",
            s.config.scene.variant,
            f.selection_revision,
            f.transport
                .as_ref()
                .map(|t| (&t.player, &t.owner, t.status, t.capabilities)),
            f.volume.as_ref().map(|t| (&t.player, &t.owner))
        );
        if let Some(e) = self.logo_ellipse() {
            layout.hotspots.push(super::shell_bridge::Hotspot {
                id: "settings",
                rect: [e.x, e.y, e.w, e.h],
            });
        }
        if !s.config.transport.enabled || super::visibility::suspended(&area) {
            return layout;
        }
        for (id, command) in [
            ("play_pause", Command::PlayPause),
            ("previous", Command::Previous),
            ("next", Command::Next),
        ] {
            if f.command_target(command, now).is_some() {
                if let Some(e) = self.ellipse(command) {
                    layout.hotspots.push(Hotspot {
                        id,
                        rect: [e.x, e.y, e.w, e.h],
                    });
                }
            }
        }
        if f.volume_target(now).is_some() {
            if let Some(e) = self.volume_ellipse() {
                layout.hotspots.push(Hotspot {
                    id: "volume",
                    rect: [e.x, e.y, e.w, e.h],
                });
            }
        }
        layout
    }
    pub(super) fn set_shell_notify(&self, notify: Rc<dyn Fn()>) {
        *self.shell_notify.borrow_mut() = Some(notify);
    }
    pub(super) fn shell_press(&self, x: f64, y: f64) {
        self.press(1, x, y);
    }
    pub(super) fn shell_release(&self, x: f64, y: f64) {
        self.release(1, x, y);
    }
    pub(super) fn shell_scroll(&self, x: f64, y: f64, dy: f64) {
        self.pointer.set(Some((x, y)));
        self.scroll(dy);
    }
    pub(super) fn shell_cancel(&self) {
        self.cancel_capture();
        self.pointer.set(None);
    }
    pub(super) fn shell_motion(&self, x: f64, y: f64) {
        self.update_pointer(x, y);
    }
    fn index(c: Command) -> usize {
        match c {
            Command::PlayPause => 0,
            Command::Previous => 1,
            Command::Next => 2,
        }
    }
    fn cancel_capture(&self) {
        self.pressed_logo.set(None);
        let pressed = self.pressed.borrow_mut().take();
        if let Some(p) = pressed {
            self.animations.borrow_mut()[Self::index(p.command)].cancel(Instant::now());
            self.ensure_animation();
        }
    }
    pub(super) fn suspend(&self) {
        self.cancel_capture();
        if self.direct_input {
            self.sync();
        }
    }
    fn ensure_animation(&self) {
        if self.animation_running.get()
            || !(self.animations.borrow().iter().any(|m| m.moving())
                || self.knob_motion.borrow().moving())
        {
            return;
        }
        let Some(area) = self.area.upgrade() else {
            return;
        };
        if super::visibility::suspended(&area) || area.error().is_some() {
            return;
        }
        self.animation_running.set(true);
        let weak = self.weak.clone();
        let next = Cell::new(Instant::now());
        let tick = move |area: &gtk::GLArea| {
            let Some(ui) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if super::visibility::suspended(area) || area.error().is_some() || !area.is_realized() {
                ui.animation_running.set(false);
                return glib::ControlFlow::Break;
            }
            let now = Instant::now();
            if now < next.get() {
                return glib::ControlFlow::Continue;
            }
            ui.sync();
            let (progress, mut moving) = {
                let mut motions = ui.animations.borrow_mut();
                for m in motions.iter_mut() {
                    m.advance(now);
                }
                (
                    std::array::from_fn(|i| motions[i].position()),
                    motions.iter().any(|m| m.moving()),
                )
            };
            if let Some(renderer) = ui.renderer.borrow_mut().as_mut() {
                if renderer.set_button_progress(progress) {
                    super::presenter::queue_render(area);
                }
                let mut knob = ui.knob_motion.borrow_mut();
                knob.advance(now);
                moving |= knob.moving();
                if renderer.set_knob_angle(knob.angle()) {
                    super::presenter::queue_render(area);
                }
                tracing::trace!(
                    ?progress,
                    knob_angle = knob.angle(),
                    moving,
                    "Transport feedback frame"
                );
            }
            if !moving {
                ui.animation_running.set(false);
                return glib::ControlFlow::Break;
            }
            next.set(
                now + std::time::Duration::from_secs_f64(
                    1.0 / ui.state.borrow().config.render.fps as f64,
                ),
            );
            glib::ControlFlow::Continue
        };
        if self.state.borrow().shell_surface {
            // Minimized owned sources need the same on-demand timer as arm/record
            // motion; a GTK tick alone may never run after pause/idle.
            let interval = std::time::Duration::from_secs_f64(
                1.0 / self.state.borrow().config.render.fps as f64,
            );
            let weak_area = area.downgrade();
            glib::timeout_add_local(interval, move || {
                let Some(area) = weak_area.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                tick(&area)
            });
        } else {
            area.add_tick_callback(move |area, _| tick(area));
        }
    }
    fn event_point(&self, x: f64, y: f64) -> (f64, f64) {
        let [ox, oy] = self.receiver.as_ref().map_or([0, 0], |r| r.origin.get());
        (x + ox as f64, y + oy as f64)
    }
    fn update_pointer(&self, x: f64, y: f64) {
        let (x, y) = self.event_point(x, y);
        self.pointer.set(Some((x, y)));
        if self.pressed_logo.get().is_some_and(|e| !e.contains(x, y)) {
            self.pressed_logo.set(None);
        }
        if self
            .pressed
            .borrow()
            .as_ref()
            .is_some_and(|p| !p.ellipse.contains(x, y))
        {
            self.cancel_capture();
        }
        self.sync();
    }
    fn event_widget(&self) -> Option<gtk::Widget> {
        if self.desktop && !self.direct_input {
            self.receiver.as_ref()?.widget.upgrade().map(|w| w.upcast())
        } else {
            self.area.upgrade().map(|w| w.upcast())
        }
    }
    fn ellipse(&self, c: Command) -> Option<Ellipse> {
        let area = self.area.upgrade()?;
        let renderer = self.renderer.borrow();
        let aspect = renderer.as_ref()?.scene_aspect();
        let state = self.state.borrow();
        if state.mode != crate::config::PreviewMode::Arm || area.error().is_some() {
            return None;
        }
        state
            .config
            .transport
            .project(c, area.width(), area.height(), area.scale_factor(), aspect)
    }
    fn hit(&self, x: f64, y: f64) -> Option<(Command, Ellipse, Target, u64)> {
        Command::ALL.into_iter().find_map(|c| {
            let ellipse = self.ellipse(c)?;
            if !ellipse.contains(x, y) {
                return None;
            }
            let state = self.state.borrow();
            let feed = state.playback.borrow();
            Some((
                c,
                ellipse,
                feed.command_target(c, Instant::now())?.clone(),
                feed.selection_revision,
            ))
        })
    }
    fn volume_ellipse(&self) -> Option<Ellipse> {
        let area = self.area.upgrade()?;
        let r = self.renderer.borrow();
        let s = self.state.borrow();
        if s.mode != crate::config::PreviewMode::Arm || area.error().is_some() {
            return None;
        }
        s.config.transport.project_volume(
            area.width(),
            area.height(),
            area.scale_factor(),
            r.as_ref()?.scene_aspect(),
        )
    }
    fn volume_available(&self) -> bool {
        self.state
            .borrow()
            .playback
            .borrow()
            .volume_target(Instant::now())
            .is_some()
    }
    fn sync_knob(&self) {
        let now = Instant::now();
        let volume = {
            let s = self.state.borrow();
            if s.mode == crate::config::PreviewMode::Arm && s.config.transport.enabled {
                s.playback.borrow().volume_target(now).map(|t| t.value)
            } else {
                None
            }
        };
        if !self
            .renderer
            .borrow()
            .as_ref()
            .is_some_and(|r| r.knob_supported())
        {
            self.knob_motion.borrow_mut().stop();
            return;
        }
        match volume {
            Some(value) => self.knob_motion.borrow_mut().retarget(value, now),
            None => self.knob_motion.borrow_mut().stop(),
        }
        // Handles renderer replacement too: the settled marker is uploaded to
        // the new scene once, without resetting its angle or starting a loop.
        if let Some(renderer) = self.renderer.borrow_mut().as_mut() {
            if renderer.set_knob_angle(self.knob_motion.borrow().angle()) {
                if let Some(area) = self.area.upgrade() {
                    super::presenter::queue_render(&area);
                }
            }
        }
        self.ensure_animation();
    }
    fn scroll(&self, dy: f64) -> bool {
        let Some((x, y)) = self.pointer.get() else {
            return false;
        };
        if !self.volume_ellipse().is_some_and(|e| e.contains(x, y)) {
            return false;
        }
        let Some(delta) = crate::playback::volume::wheel_delta(dy) else {
            return false;
        };
        self.monitor.adjust_volume(delta).is_ok()
    }
    pub(super) fn sync(&self) {
        let Some(area) = self.area.upgrade() else {
            return;
        };
        self.sync_knob();
        if self
            .pressed_logo
            .get()
            .is_some_and(|e| self.logo_ellipse() != Some(e))
        {
            self.pressed_logo.set(None);
        }
        let invalid_capture = self.pressed.borrow().as_ref().is_some_and(|p| {
            let s = self.state.borrow();
            let f = s.playback.borrow();
            p.scene != s.config.scene.variant
                || p.revision != f.selection_revision
                || self.ellipse(p.command) != Some(p.ellipse)
                || !f
                    .command_target(p.command, Instant::now())
                    .is_some_and(|t| t.same_player(&p.target) && t.status == p.target.status)
        });
        if invalid_capture {
            self.cancel_capture();
        }
        if self.state.borrow().shell_surface {
            if let Some(notify) = self.shell_notify.borrow().as_ref() {
                notify();
            }
            return;
        }
        if self.direct_input {
            if let Some(scene) = self.window.upgrade() {
                if let Some(surface) = scene.surface() {
                    let mut spans = Vec::new();
                    if let Some(e) = self.logo_ellipse() {
                        spans.extend(e.spans([0.0, 0.0]));
                    }
                    if !super::visibility::suspended(&area)
                        && self.state.borrow().config.transport.enabled
                    {
                        for c in Command::ALL {
                            if self
                                .state
                                .borrow()
                                .playback
                                .borrow()
                                .command_target(c, Instant::now())
                                .is_some()
                            {
                                if let Some(e) = self.ellipse(c) {
                                    spans.extend(e.spans([0.0, 0.0]));
                                }
                            }
                        }
                        if self.volume_available() {
                            if let Some(e) = self.volume_ellipse() {
                                spans.extend(e.spans([0.0, 0.0]));
                            }
                        }
                    }
                    if self.input.borrow().as_ref() != Some(&spans) {
                        self.cancel_capture();
                        let region = gtk::cairo::Region::create();
                        let offset = area
                            .compute_bounds(&scene)
                            .map_or([0, 0], |r| [r.x().round() as i32, r.y().round() as i32]);
                        for [x, y, w, h] in &spans {
                            if region
                                .union_rectangle(&gtk::cairo::RectangleInt::new(
                                    *x + offset[0],
                                    *y + offset[1],
                                    *w,
                                    *h,
                                ))
                                .is_err()
                            {
                                surface.set_input_region(&gtk::cairo::Region::create());
                                return;
                            }
                        }
                        surface.set_input_region(&region);
                        // Wayland input regions are double-buffered surface
                        // state. Commit availability changes even when scene
                        // animation has just settled and no GL render follows.
                        surface.queue_render();
                        *self.input.borrow_mut() = Some(spans);
                    }
                }
            }
        } else if self.desktop {
            let Some(scene) = self.window.upgrade() else {
                return;
            };
            let Some(receiver) = &self.receiver else {
                return;
            };
            let Some(window) = receiver.window.upgrade() else {
                return;
            };
            if window.surface().is_none() {
                return;
            }
            let offset = area
                .compute_bounds(&scene)
                .map_or([0.0, 0.0], |r| [r.x() as f64, r.y() as f64]);
            let all: Vec<_> = Command::ALL
                .into_iter()
                .filter_map(|c| self.ellipse(c))
                .chain(self.volume_ellipse())
                .chain(self.logo_ellipse())
                .flat_map(|e| e.spans([0.0, 0.0]))
                .collect();
            let left = all.iter().map(|r| r[0]).min().unwrap_or(0);
            let top = all.iter().map(|r| r[1]).min().unwrap_or(0);
            let right = all.iter().map(|r| r[0] + r[2]).max().unwrap_or(1);
            let bottom = all.iter().map(|r| r[1] + r[3]).max().unwrap_or(1);
            let size = [(right - left).max(1), (bottom - top).max(1)];
            if receiver.origin.get() != [left, top] || receiver.size.get() != size {
                self.cancel_capture();
                receiver.origin.set([left, top]);
                receiver.size.set(size);
                if let Some(widget) = receiver.widget.upgrade() {
                    widget.set_content_width(size[0]);
                    widget.set_content_height(size[1]);
                }
                window.set_default_size(size[0], size[1]);
                *self.input.borrow_mut() = None;
            }
            let mut spans = Vec::new();
            if let Some(e) = self.logo_ellipse() {
                spans.extend(e.spans([0.0, 0.0]));
            }
            for c in Command::ALL {
                let available = {
                    let s = self.state.borrow();
                    let f = s.playback.borrow();
                    f.command_target(c, Instant::now()).is_some()
                };
                if available {
                    if let Some(e) = self.ellipse(c) {
                        spans.extend(e.spans([0.0, 0.0]));
                    }
                }
            }
            if self.volume_available() {
                if let Some(e) = self.volume_ellipse() {
                    spans.extend(e.spans([0.0, 0.0]));
                }
            }
            if !receiver
                .placement
                .can_receive(&window)
                .unwrap_or_else(|error| {
                    tracing::warn!("Cannot confirm transport input layer: {error:#}");
                    false
                })
            {
                spans.clear();
            }
            if self.input.borrow().as_ref() != Some(&spans) {
                self.cancel_capture();
                let region = gtk::cairo::Region::create();
                for [x, y, w, h] in &spans {
                    if let Err(error) = region.union_rectangle(&gtk::cairo::RectangleInt::new(
                        *x - left,
                        *y - top,
                        *w,
                        *h,
                    )) {
                        tracing::warn!("Cannot build transport input shape: {error}");
                        let _ = receiver
                            .placement
                            .set_input_region(&window, &gtk::cairo::Region::create());
                        return;
                    }
                }
                if let Err(error) = receiver.placement.set_input_region(&window, &region) {
                    tracing::warn!("Cannot apply transport input region: {error:#}");
                    window.set_visible(false);
                    return;
                }
                *self.input.borrow_mut() = Some(spans);
            }
            // Keep the receiver mapped across commands/capability updates.
            // An empty mask passes all input through without a WM remap race.
            let active = !all.is_empty()
                && (self.logo_ellipse().is_some() || self.state.borrow().playback.borrow().enabled);
            if active {
                let [x, y] = receiver.origin.get();
                let rect = [
                    x + offset[0].round() as i32,
                    y + offset[1].round() as i32,
                    size[0],
                    size[1],
                ];
                if let Err(error) = receiver.placement.position(&window, &scene, rect) {
                    tracing::warn!("Cannot position transport input: {error:#}");
                    window.set_visible(false);
                    return;
                }
                if !window.is_visible() {
                    // No present()/raise(): keep this managed control receiver
                    // below normal applications and never steal keyboard focus.
                    window.set_visible(true);
                    if let Err(error) = receiver.placement.position(&window, &scene, rect) {
                        tracing::warn!("Cannot position mapped transport input: {error:#}");
                        window.set_visible(false);
                    }
                }
            } else {
                window.set_visible(false);
            }
        }
        let Some(event_widget) = self.event_widget() else {
            return;
        };
        let hovered = self.pointer.get().and_then(|(x, y)| self.hit(x, y));
        let knob_hovered = self.volume_available()
            && self
                .pointer
                .get()
                .is_some_and(|(x, y)| self.volume_ellipse().is_some_and(|e| e.contains(x, y)));
        let logo_hovered = self
            .pointer
            .get()
            .is_some_and(|(x, y)| self.logo_ellipse().is_some_and(|e| e.contains(x, y)));
        event_widget.set_cursor_from_name(
            (hovered.is_some() || knob_hovered || logo_hovered).then_some("pointer"),
        );
    }
    fn press(&self, button: i32, x: f64, y: f64) -> bool {
        self.cancel_capture();
        if button != 1 {
            return false;
        }
        if let Some(e) = self.logo_ellipse().filter(|e| e.contains(x, y)) {
            self.pressed_logo.set(Some(e));
            return true;
        }
        let Some((command, ellipse, target, revision)) = self.hit(x, y) else {
            tracing::trace!(x, y, "Transport press outside available controls");
            return false;
        };
        tracing::trace!(?command, x, y, "Transport press captured");
        *self.pressed.borrow_mut() = Some(Pressed {
            command,
            ellipse,
            target,
            revision,
            scene: self.state.borrow().config.scene.variant,
        });
        if self
            .renderer
            .borrow()
            .as_ref()
            .is_some_and(|r| r.buttons_supported())
        {
            self.animations.borrow_mut()[Self::index(command)].press(Instant::now());
            self.ensure_animation();
        }
        true
    }
    fn release(&self, button: i32, x: f64, y: f64) {
        self.sync();
        if let Some(e) = self.pressed_logo.take() {
            if button == 1 && e.contains(x, y) && self.logo_ellipse() == Some(e) {
                self.open_settings();
            }
            return;
        }
        let Some(p) = self.pressed.borrow_mut().take() else {
            return;
        };
        if button != 1 || !p.ellipse.contains(x, y) {
            self.animations.borrow_mut()[Self::index(p.command)].cancel(Instant::now());
            self.ensure_animation();
            return;
        }
        let valid = self
            .hit(x, y)
            .is_some_and(|(command, ellipse, _, _)| command == p.command && ellipse == p.ellipse);
        if valid {
            self.animations.borrow_mut()[Self::index(p.command)].release(Instant::now());
        } else {
            self.animations.borrow_mut()[Self::index(p.command)].cancel(Instant::now());
        }
        self.ensure_animation();
        if !valid {
            return;
        }
        if let Err(error) = self.monitor.command_for(p.command, &p.target, p.revision) {
            tracing::debug!("Transport click ignored: {error}");
        }
    }
    pub(super) fn attach(
        area: &gtk::GLArea,
        window: &gtk::ApplicationWindow,
        state: Rc<RefCell<AppState>>,
        renderer: SharedRenderer,
        monitor: Monitor,
        host: Option<&crate::desktop::DesktopHost>,
    ) -> anyhow::Result<Rc<Self>> {
        let desktop = host.is_some();
        let direct_input =
            host.is_some_and(|h| h.kind() == crate::desktop::BackendKind::LayerShell);
        let receiver = if let Some(host) = host.filter(|_| !direct_input) {
            let app = window
                .application()
                .ok_or_else(|| anyhow::anyhow!("Transport needs an application"))?;
            let input = gtk::ApplicationWindow::builder()
                .application(&app)
                .title("Talis · Transport input")
                .default_width(1)
                .default_height(1)
                .build();
            let widget = host.input_widget();
            input.set_child(Some(&widget));
            let placement = match host.configure_input(&input) {
                Ok(p) => p,
                Err(e) => {
                    input.destroy();
                    return Err(e);
                }
            };
            window.connect_unrealize({
                let weak = input.downgrade();
                move |_| {
                    if let Some(input) = weak.upgrade() {
                        input.destroy();
                    }
                }
            });
            Some(DesktopReceiver {
                window: input.downgrade(),
                widget: widget.downgrade(),
                placement,
                origin: Cell::new([0, 0]),
                size: Cell::new([1, 1]),
            })
        } else {
            None
        };
        let ui = Rc::new_cyclic(|weak| Self {
            area: area.downgrade(),
            window: window.downgrade(),
            state,
            renderer,
            monitor,
            desktop,
            direct_input,
            receiver,
            pressed: RefCell::new(None),
            settings: RefCell::new(None),
            settings_tray: RefCell::new(None),
            pressed_logo: Cell::new(None),
            pointer: Cell::new(None),
            input: RefCell::new(None),
            animations: RefCell::new(std::array::from_fn(|_| Default::default())),
            knob_motion: RefCell::new(Default::default()),
            animation_running: Cell::new(false),
            shell_notify: RefCell::new(None),
            weak: weak.clone(),
        });
        if ui.state.borrow().shell_surface {
            // No native input to the minimized source or a second X11 receiver.
            area.set_can_target(false);
            area.connect_resize({
                let ui = ui.clone();
                move |_, _, _| {
                    ui.cancel_capture();
                    ui.sync();
                }
            });
            area.connect_notify_local(Some("scale-factor"), {
                let ui = ui.clone();
                move |_, _| {
                    ui.cancel_capture();
                    ui.sync();
                }
            });
            area.connect_realize({
                let ui = ui.clone();
                move |_| ui.sync()
            });
            return Ok(ui);
        }
        area.set_can_target(!desktop || direct_input);
        let event_widget = ui
            .event_widget()
            .ok_or_else(|| anyhow::anyhow!("Missing transport receiver"))?;
        let click = gtk::GestureClick::new();
        click.set_button(1);
        // GestureClick's n_press is a click count, not a mouse button. The
        // gesture already filters to primary-button input; repeated clicks
        // must still pass button 1 through the capture/release validation.
        click.connect_pressed({
            let ui = ui.clone();
            move |gesture, _, x, y| {
                let (x, y) = ui.event_point(x, y);
                if ui.press(1, x, y) {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                }
            }
        });
        click.connect_released({
            let ui = ui.clone();
            move |_, _, x, y| {
                let (x, y) = ui.event_point(x, y);
                ui.release(1, x, y);
            }
        });
        click.connect_cancel({
            let ui = ui.clone();
            move |_, _| {
                ui.cancel_capture();
            }
        });
        click.connect_stopped({
            let ui = ui.clone();
            move |_| {
                tracing::trace!("Transport click gesture stopped");
                ui.cancel_capture();
            }
        });
        event_widget.add_controller(click);
        let motion = gtk::EventControllerMotion::new();
        // Input-region restoration/remapping can enter beneath a stationary
        // pointer without a motion event. Restore hover before its next wheel.
        motion.connect_enter({
            let ui = ui.clone();
            move |_, x, y| ui.update_pointer(x, y)
        });
        motion.connect_motion({
            let ui = ui.clone();
            move |_, x, y| ui.update_pointer(x, y)
        });
        motion.connect_leave({
            let ui = ui.clone();
            move |_| {
                ui.pointer.set(None);
                ui.cancel_capture();
                ui.sync();
            }
        });
        event_widget.add_controller(motion);
        let scroll = gtk::EventControllerScroll::new(
            gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::DISCRETE,
        );
        scroll.connect_scroll({
            let ui = ui.clone();
            move |_, _, dy| {
                if ui.scroll(dy) {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
        });
        event_widget.add_controller(scroll);
        area.connect_resize({
            let ui = ui.clone();
            move |_, _, _| {
                ui.cancel_capture();
                ui.sync();
            }
        });
        area.connect_notify_local(Some("scale-factor"), {
            let ui = ui.clone();
            move |_, _| {
                ui.cancel_capture();
                ui.sync();
            }
        });
        window.connect_map({
            let ui = ui.clone();
            move |_| {
                *ui.input.borrow_mut() = None;
                ui.sync();
            }
        });
        if let Some(input) = ui.receiver.as_ref().and_then(|r| r.window.upgrade()) {
            input.connect_map({
                let weak = Rc::downgrade(&ui);
                move |_| {
                    let weak = weak.clone();
                    // Allow the WM to acknowledge the post-map BELOW request.
                    glib::timeout_add_local_once(std::time::Duration::from_millis(30), move || {
                        if let Some(ui) = weak.upgrade() {
                            ui.sync();
                        }
                    });
                }
            });
        }
        ui.sync();
        Ok(ui)
    }
}

#[cfg(test)]
mod tests;
