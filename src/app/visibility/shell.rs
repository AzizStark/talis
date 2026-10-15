//! Owned Shell evidence has a short lease: lost/stale coverage fails open.
use super::*;

const LEASE: Duration = Duration::from_millis(1500);

#[derive(Default)]
struct Evidence {
    sample: Option<(Instant, u64, bool, bool)>,
}
impl Evidence {
    fn retag(&mut self, previous: u64, generation: u64, geometry_changed: bool) {
        if !geometry_changed {
            if let Some((_, sampled_generation, _, _)) = self.sample.as_mut() {
                if *sampled_generation == previous {
                    *sampled_generation = generation;
                }
            }
        }
    }
    fn mode(&self, now: Instant, generation: u64, enabled: bool) -> (Mode, bool) {
        let Some((at, sampled_generation, hidden, locked)) = self.sample else {
            return (Mode::Visible, false);
        };
        if !enabled
            || now.saturating_duration_since(at) > LEASE
            || (sampled_generation != generation && !locked)
        {
            return (Mode::Visible, false);
        }
        if locked {
            (Mode::Hidden, true)
        } else if hidden {
            (Mode::AnimationCovered, false)
        } else {
            (Mode::Visible, false)
        }
    }
}

pub(in crate::app) struct ShellController {
    area: glib::WeakRef<gtk::GLArea>,
    gate: Rc<Gate>,
    state: Rc<RefCell<AppState>>,
    running: Rc<Cell<bool>>,
    transport: RefCell<Option<Weak<TransportUi>>>,
    evidence: RefCell<Evidence>,
    generation: Cell<u64>,
    timer: RefCell<Option<glib::SourceId>>,
}
impl ShellController {
    pub(in crate::app) fn attach(
        area: &gtk::GLArea,
        state: Rc<RefCell<AppState>>,
        running: Rc<Cell<bool>>,
    ) -> Rc<Self> {
        let gate = Rc::new(Gate::default());
        gate::install(area, gate.clone());
        area.connect_resize(|area, _, _| invalidate_layout(area));
        area.connect_notify_local(Some("scale-factor"), |area, _| invalidate_layout(area));
        let this = Rc::new(Self {
            area: area.downgrade(),
            gate,
            state,
            running,
            transport: RefCell::new(None),
            evidence: RefCell::new(Evidence::default()),
            generation: Cell::new(0),
            timer: RefCell::new(None),
        });
        let weak = Rc::downgrade(&this);
        *this.timer.borrow_mut() = Some(glib::timeout_add_local(
            Duration::from_millis(250),
            move || {
                let Some(this) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                if this.area.upgrade().is_none() {
                    return glib::ControlFlow::Break;
                }
                this.poll();
                glib::ControlFlow::Continue
            },
        ));
        this
    }
    pub(in crate::app) fn set_transport(&self, ui: &Rc<TransportUi>) {
        *self.transport.borrow_mut() = Some(Rc::downgrade(ui));
    }
    pub(in crate::app) fn set_generation(&self, generation: u64, geometry_changed: bool) {
        let previous = self.generation.replace(generation);
        self.evidence
            .borrow_mut()
            .retag(previous, generation, geometry_changed);
        // Do not reenter bridge publication here. The next poll retires old evidence.
    }
    pub(in crate::app) fn observe(&self, generation: u64, hidden: bool, locked: bool) {
        if generation != self.generation.get() {
            return;
        }
        self.evidence.borrow_mut().sample = Some((Instant::now(), generation, hidden, locked));
        self.poll();
    }
    pub(in crate::app) fn input_blocked(&self) -> bool {
        self.evidence
            .borrow()
            .mode(Instant::now(), self.generation.get(), true)
            .0
            != Mode::Visible
    }
    pub(in crate::app) fn locked(&self) -> bool {
        self.evidence
            .borrow()
            .mode(Instant::now(), self.generation.get(), true)
            .0
            == Mode::Hidden
    }
    fn poll(&self) {
        let Some(area) = self.area.upgrade() else {
            return;
        };
        let (mode, forced) = self.evidence.borrow().mode(
            Instant::now(),
            self.generation.get(),
            self.state.borrow().config.render.pause_when_hidden,
        );
        let ui = self.transport.borrow().as_ref().and_then(Weak::upgrade);
        super::apply_mode(
            &area,
            &self.gate,
            &self.state,
            &self.running,
            ui.as_ref(),
            mode,
            forced,
        );
    }
    pub(in crate::app) fn stop(&self) {
        if let Some(timer) = self.timer.borrow_mut().take() {
            timer.remove();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shell_evidence_expires_and_geometry_changes_retire_coverage_not_lock() {
        let now = Instant::now();
        let mut e = Evidence::default();
        assert_eq!(e.mode(now, 1, true), (Mode::Visible, false));
        e.sample = Some((now, 1, true, false));
        assert_eq!(e.mode(now, 1, true), (Mode::AnimationCovered, false));
        assert_eq!(e.mode(now, 2, true), (Mode::Visible, false));
        assert_eq!(
            e.mode(now + LEASE + Duration::from_millis(1), 1, true),
            (Mode::Visible, false)
        );
        e.sample = Some((now, 1, true, true));
        assert_eq!(e.mode(now, 2, true), (Mode::Hidden, true));
        assert_eq!(e.mode(now, 1, false), (Mode::Visible, false));
        assert_eq!(
            e.mode(now + LEASE + Duration::from_millis(1), 1, true),
            (Mode::Visible, false)
        );
    }
    #[test]
    fn input_layout_updates_preserve_fresh_coverage_but_never_revive_retired_geometry() {
        let now = Instant::now();
        let mut e = Evidence {
            sample: Some((now, 1, true, false)),
        };
        e.retag(1, 2, false);
        assert_eq!(e.mode(now, 2, true), (Mode::AnimationCovered, false));
        e.retag(2, 3, true);
        assert_eq!(e.mode(now, 3, true), (Mode::Visible, false));
        e.retag(3, 4, false);
        assert_eq!(e.mode(now, 4, true), (Mode::Visible, false));
        assert_eq!(
            e.sample.unwrap().0,
            now,
            "publication cannot renew an evidence lease"
        );
    }
}
