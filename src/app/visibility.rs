//! Desktop-only rendering gate. Media observation and asset loading stay alive.
mod gate;
#[cfg(feature = "wayland-desktop")]
mod kwin;
mod shell;
use crate::desktop::screensaver;
use gate::{Gate, Mode};
pub(super) use gate::{
    begin_background, begin_frame, generation, invalidate_layout, render_pending,
    request_background, request_content, suspended,
};
pub(super) use shell::ShellController;

use super::{scheduling, state::AppState, transport::TransportUi};
use crate::desktop::{
    DesktopHost,
    visibility::{Hidden, Layout, Probe, Targets},
};
use gtk::{glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

/// Require two distinct confirmations of coverage, and fail open on stale or
/// uncertain evidence. Lock/power-off signals are already direct confirmations.
#[derive(Default)]
struct Evidence {
    sample: Option<(Targets, Instant, Option<Hidden>)>,
    coverage_count: u8,
}
impl Evidence {
    fn record(&mut self, targets: Targets, at: Instant, hidden: Option<Hidden>) {
        self.coverage_count = if hidden.is_some_and(Hidden::coverage)
            && self.sample.as_ref().is_some_and(|(old, old_at, reason)| {
                old == &targets
                    && *reason == hidden
                    && at > *old_at
                    && at.saturating_duration_since(*old_at) <= Duration::from_secs(1)
            }) {
            self.coverage_count.saturating_add(1)
        } else {
            u8::from(hidden.is_some_and(Hidden::coverage))
        };
        self.sample = Some((targets, at, hidden));
    }
    fn hidden(&self, targets: &Targets, now: Instant) -> Option<Hidden> {
        let (sample_targets, at, hidden) = self.sample.as_ref()?;
        if targets != sample_targets || now.saturating_duration_since(*at) > Duration::from_secs(1)
        {
            return None;
        }
        hidden.filter(|h| !h.coverage() || self.coverage_count >= 2)
    }
}

pub(super) struct Controller {
    host: DesktopHost,
    area: glib::WeakRef<gtk::GLArea>,
    scene: glib::WeakRef<gtk::ApplicationWindow>,
    gate: Rc<Gate>,
    state: Rc<RefCell<AppState>>,
    running: Rc<Cell<bool>>,
    transport: RefCell<Option<Weak<TransportUi>>>,
    probe: RefCell<Option<Probe>>,
    evidence: RefCell<Evidence>,
    screensaver: Rc<screensaver::Monitor>,
    timer: RefCell<Option<glib::SourceId>>,
    #[cfg(feature = "wayland-desktop")]
    kwin: RefCell<Option<Rc<kwin::Monitor>>>,
}
impl Controller {
    pub(super) fn attach(
        area: &gtk::GLArea,
        scene: &gtk::ApplicationWindow,
        state: Rc<RefCell<AppState>>,
        running: Rc<Cell<bool>>,
        host: DesktopHost,
    ) -> Rc<Self> {
        let gate = Rc::new(Gate::default());
        gate::install(area, gate.clone());
        area.connect_resize(|area, _, _| invalidate_layout(area));
        area.connect_notify_local(Some("scale-factor"), |area, _| invalidate_layout(area));
        let probe = host
            .capabilities()
            .visibility_probe
            .then(|| {
                host
            .visibility_probe()
            .map_err(|error| {
                tracing::warn!(%error, "Visibility probe unavailable; coverage stays active");
            })
            .ok()
            })
            .flatten();
        let controller = Rc::new(Self {
            host,
            area: area.downgrade(),
            scene: scene.downgrade(),
            gate,
            state,
            running,
            transport: RefCell::new(None),
            probe: RefCell::new(probe),
            evidence: RefCell::new(Evidence::default()),
            screensaver: screensaver::Monitor::start(),
            timer: RefCell::new(None),
            #[cfg(feature = "wayland-desktop")]
            kwin: RefCell::new(None),
        });
        let weak = Rc::downgrade(&controller);
        *controller.timer.borrow_mut() = Some(glib::timeout_add_local(
            Duration::from_millis(250),
            move || {
                let Some(controller) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                if controller.area.upgrade().is_none() {
                    return glib::ControlFlow::Break;
                }
                controller.poll();
                glib::ControlFlow::Continue
            },
        ));
        controller
    }
    pub(super) fn set_transport(self: &Rc<Self>, ui: &Rc<TransportUi>) {
        *self.transport.borrow_mut() = Some(Rc::downgrade(ui));
        #[cfg(feature = "wayland-desktop")]
        if self.host.kind() == crate::desktop::BackendKind::LayerShell
            && DesktopHost::plasma_session()
            && self.kwin.borrow().is_none()
        {
            let weak = Rc::downgrade(self);
            *self.kwin.borrow_mut() = Some(kwin::Monitor::start(Rc::new(move || {
                weak.upgrade()
                    .and_then(|controller| kwin::Target::from_controller(&controller))
            })));
        }
    }
    fn targets(&self, area: &gtk::GLArea) -> Option<Targets> {
        let scene = self.scene.upgrade()?;
        let patch = super::presenter::native_window(area).filter(|window| window.is_mapped());
        self.host.visibility_targets(
            &scene,
            patch.as_ref(),
            Layout {
                animation: patch
                    .as_ref()
                    .and_then(|_| super::presenter::native_coverage_rect(area)),
                scene_size: [
                    area.width() * area.scale_factor(),
                    area.height() * area.scale_factor(),
                ],
                generation: generation(area),
            },
        )
    }
    fn poll(&self) {
        let Some(area) = self.area.upgrade() else {
            return;
        };
        let now = Instant::now();
        let enabled = self.state.borrow().config.render.pause_when_hidden;
        // Native alignment must remain correct even when suspension is disabled.
        let mut targets = (enabled || super::presenter::native_window(&area).is_some())
            .then(|| self.targets(&area))
            .flatten();
        if let Some(probe) = self.probe.borrow().as_ref() {
            if let Some(sample) = probe.latest() {
                if targets.as_ref() == Some(&sample.targets)
                    && now.saturating_duration_since(sample.at) <= Duration::from_secs(1)
                {
                    if let Some(origin) = sample.scene_origin {
                        super::presenter::relocate_native(&area, origin);
                        if generation(&area) != sample.targets.generation {
                            // Relocation retires coverage from the old native
                            // bounds. Never apply that sample to the new layout.
                            targets = self.targets(&area);
                        }
                    }
                }
                self.evidence
                    .borrow_mut()
                    .record(sample.targets, sample.at, sample.hidden);
            }
            if let Some(targets) = &targets {
                probe.request(targets.clone());
            }
        }
        if !enabled {
            *self.evidence.borrow_mut() = Evidence::default();
        }
        let native = targets
            .as_ref()
            .filter(|_| enabled)
            .and_then(|t| self.evidence.borrow().hidden(t, now));
        let locked = self.screensaver.active(now);
        let unmapped = self.host.kind() == crate::desktop::BackendKind::LayerShell
            && self.scene.upgrade().is_none_or(|scene| !scene.is_mapped());
        let forced = enabled && (locked || unmapped || native == Some(Hidden::ScreenOff));
        let kwin_covered = {
            #[cfg(feature = "wayland-desktop")]
            {
                self.kwin
                    .borrow()
                    .as_ref()
                    .is_some_and(|monitor| monitor.covered(now))
            }
            #[cfg(not(feature = "wayland-desktop"))]
            {
                false
            }
        };
        let mode = if !enabled {
            Mode::Visible
        } else if forced || native == Some(Hidden::Covered) {
            Mode::Hidden
        } else if native == Some(Hidden::AnimationCovered) || kwin_covered {
            Mode::AnimationCovered
        } else {
            Mode::Visible
        };
        apply_mode(
            &area,
            &self.gate,
            &self.state,
            &self.running,
            self.transport
                .borrow()
                .as_ref()
                .and_then(Weak::upgrade)
                .as_ref(),
            mode,
            forced,
        );
    }
    pub(super) fn stop(&self) {
        if let Some(timer) = self.timer.borrow_mut().take() {
            timer.remove();
        }
        self.screensaver.stop();
        self.probe.borrow_mut().take();
        #[cfg(feature = "wayland-desktop")]
        if let Some(monitor) = self.kwin.borrow_mut().take() {
            monitor.stop();
        }
    }
}

/// Both desktop providers use the same media-preserving render transition.
fn apply_mode(
    area: &gtk::GLArea,
    gate: &Gate,
    state: &Rc<RefCell<AppState>>,
    running: &Rc<Cell<bool>>,
    transport: Option<&Rc<TransportUi>>,
    mode: Mode,
    forced: bool,
) {
    let old = gate.set_mode(mode, forced);
    let layout_resumed = gate.resume_notification.replace(false);
    if old == mode && !layout_resumed {
        return;
    }
    if mode != Mode::Visible {
        if let Some(ui) = transport {
            ui.suspend();
        }
        state
            .borrow_mut()
            .artwork_rotation
            .resume_at(Instant::now());
        tracing::info!(?mode, forced, "Scene rendering suspended");
        if mode == Mode::AnimationCovered {
            // Drain a content request racing the coverage transition and
            // consume state changes deferred by a previous full hide.
            super::presenter::queue_content_render(area);
        }
    } else {
        state.borrow_mut().resume_at(Instant::now());
        // Refresh once even if playback has stopped while hidden. Asset
        // revisions and empty-stand state are consumed by normal rendering.
        super::presenter::queue_render(area);
        super::presenter::refresh_background(area);
        scheduling::ensure_animation(area, state.clone(), running.clone());
        if let Some(ui) = transport {
            ui.sync();
        }
        tracing::info!("Scene rendering resumed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn coverage_debounces_but_uncertainty_and_staleness_resume_immediately() {
        let now = Instant::now();
        let target = Targets::fixture(Layout {
            animation: None,
            scene_size: [100, 100],
            generation: 0,
        });
        let mut e = Evidence::default();
        e.record(target.clone(), now, Some(Hidden::Covered));
        assert_eq!(e.hidden(&target, now), None);
        e.record(
            target.clone(),
            now + Duration::from_millis(250),
            Some(Hidden::Covered),
        );
        assert_eq!(
            e.hidden(&target, now + Duration::from_millis(250)),
            Some(Hidden::Covered)
        );
        assert_eq!(e.hidden(&target, now + Duration::from_secs(2)), None);
        let mut changed = target.clone();
        changed.generation = 1;
        assert_eq!(e.hidden(&changed, now), None);
        e.record(target.clone(), now + Duration::from_millis(500), None);
        assert_eq!(e.hidden(&target, now + Duration::from_millis(500)), None);
        e.record(
            target.clone(),
            now + Duration::from_millis(750),
            Some(Hidden::ScreenOff),
        );
        assert_eq!(
            e.hidden(&target, now + Duration::from_millis(750)),
            Some(Hidden::ScreenOff)
        );
    }
    #[test]
    fn animation_coverage_requires_fresh_samples_for_the_current_layout() {
        let now = Instant::now();
        let target = Targets::fixture(Layout {
            animation: Some(crate::desktop::visibility::Rect {
                x: 894,
                y: 582,
                w: 868,
                h: 636,
            }),
            scene_size: [2560, 1440],
            generation: 7,
        });
        let mut evidence = Evidence::default();
        evidence.record(target.clone(), now, Some(Hidden::AnimationCovered));
        assert_eq!(evidence.hidden(&target, now), None);
        // Reading the same worker result twice cannot confirm coverage.
        evidence.record(target.clone(), now, Some(Hidden::AnimationCovered));
        assert_eq!(evidence.hidden(&target, now), None);
        let later = now + Duration::from_millis(250);
        evidence.record(target.clone(), later, Some(Hidden::AnimationCovered));
        assert_eq!(
            evidence.hidden(&target, later),
            Some(Hidden::AnimationCovered)
        );
        let mut changed = target.clone();
        changed.generation = 8;
        assert_eq!(evidence.hidden(&changed, later), None);
        changed = target.clone();
        changed.animation = None;
        assert_eq!(evidence.hidden(&changed, later), None);
        evidence.record(target.clone(), later + Duration::from_millis(250), None);
        assert_eq!(
            evidence.hidden(&target, later + Duration::from_millis(250)),
            None
        );
    }
}
