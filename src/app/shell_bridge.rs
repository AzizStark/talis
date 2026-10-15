//! Owned GNOME stdin protocol. No public bus service, player logic or pixel IPC.
use super::transport::TransportUi;
use gtk::{gio, glib, prelude::*};
use serde::{Deserialize, Serialize};
use std::{
    cell::{Cell, RefCell},
    os::fd::AsFd,
    rc::Rc,
};

const MAX_LINE: usize = 512;

#[derive(Clone, Default, PartialEq, Serialize)]
pub(super) struct Layout {
    pub width: i32,
    pub height: i32,
    /// Actual rendered content in logical top-left source coordinates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viewport: Option<[f64; 4]>,
    /// Conservative union of every animated region, in the same source coordinates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub animation: Option<[f64; 4]>,
    pub pause_when_hidden: bool,
    pub hotspots: Vec<Hotspot>,
    #[serde(skip)]
    pub key: String,
}
#[derive(Clone, PartialEq, Serialize)]
pub(super) struct Hotspot {
    pub id: &'static str,
    pub rect: [f64; 4],
}

#[derive(Debug, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
enum Event {
    Press {
        seq: u64,
        generation: u64,
        x: f64,
        y: f64,
    },
    Release {
        seq: u64,
        generation: u64,
        x: f64,
        y: f64,
    },
    Motion {
        seq: u64,
        generation: u64,
        x: f64,
        y: f64,
    },
    Scroll {
        seq: u64,
        generation: u64,
        x: f64,
        y: f64,
        dy: f64,
    },
    Cancel {
        seq: u64,
    },
    Settings {
        seq: u64,
    },
    Visibility {
        seq: u64,
        generation: u64,
        hidden: bool,
        locked: bool,
    },
}
impl Event {
    fn seq(&self) -> u64 {
        match *self {
            Self::Press { seq, .. }
            | Self::Release { seq, .. }
            | Self::Motion { seq, .. }
            | Self::Scroll { seq, .. }
            | Self::Cancel { seq }
            | Self::Settings { seq }
            | Self::Visibility { seq, .. } => seq,
        }
    }
    fn point(&self) -> Option<(u64, f64, f64)> {
        match *self {
            Self::Press {
                generation, x, y, ..
            }
            | Self::Release {
                generation, x, y, ..
            }
            | Self::Motion {
                generation, x, y, ..
            }
            | Self::Scroll {
                generation, x, y, ..
            } => Some((generation, x, y)),
            _ => None,
        }
    }
}

// Chunked reads, not unbounded read_line: a broken sender cannot grow memory.
#[derive(Default)]
struct Lines(Vec<u8>);
impl Lines {
    fn push(&mut self, bytes: &[u8]) -> Result<Vec<Event>, &'static str> {
        let mut events = Vec::new();
        for &byte in bytes {
            if byte == b'\n' {
                let event = serde_json::from_slice(&self.0).map_err(|_| "Invalid Shell input")?;
                self.0.clear();
                events.push(event);
            } else {
                if self.0.len() >= MAX_LINE {
                    return Err("Oversized Shell input");
                }
                self.0.push(byte);
            }
        }
        Ok(events)
    }
}

pub(super) struct Bridge {
    ui: Rc<TransportUi>,
    settings: Option<glib::WeakRef<gtk::ApplicationWindow>>,
    layout: RefCell<Option<Layout>>,
    generation: Cell<u64>,
    sequence: Cell<u64>,
    task: RefCell<Option<glib::JoinHandle<()>>>,
    stopped: Cell<bool>,
    visibility: Rc<super::visibility::ShellController>,
}
impl Bridge {
    pub(super) fn attach(
        ui: &Rc<TransportUi>,
        settings: Option<&gtk::ApplicationWindow>,
        visibility: Rc<super::visibility::ShellController>,
    ) -> anyhow::Result<Rc<Self>> {
        let stream = gio::UnixInputStream::take_fd(std::io::stdin().as_fd().try_clone_to_owned()?);
        let bridge = Rc::new(Self {
            ui: ui.clone(),
            settings: settings.map(|w| w.downgrade()),
            layout: RefCell::new(None),
            generation: Cell::new(0),
            sequence: Cell::new(0),
            task: RefCell::new(None),
            stopped: Cell::new(false),
            visibility,
        });
        ui.set_shell_notify({
            let weak = Rc::downgrade(&bridge);
            Rc::new(move || {
                if let Some(bridge) = weak.upgrade() {
                    bridge.publish();
                }
            })
        });
        bridge.publish();
        let weak = Rc::downgrade(&bridge);
        *bridge.task.borrow_mut() = Some(glib::spawn_future_local(async move {
            let mut lines = Lines::default();
            loop {
                let bytes = stream
                    .read_bytes_future(1024, glib::Priority::DEFAULT)
                    .await;
                let Some(bridge) = weak.upgrade() else {
                    break;
                };
                match bytes {
                    Ok(bytes) if !bytes.is_empty() => match lines.push(bytes.as_ref()) {
                        Ok(events) => {
                            for event in events {
                                bridge.dispatch(event);
                            }
                        }
                        Err(error) => {
                            bridge.disconnect(error);
                            break;
                        }
                    },
                    _ => {
                        bridge.disconnect("Shell input pipe closed");
                        break;
                    }
                }
            }
        }));
        Ok(bridge)
    }
    fn publish(&self) {
        if self.stopped.get() {
            return;
        }
        let layout = self.ui.shell_layout();
        if self.layout.borrow().as_ref() == Some(&layout) {
            return;
        }
        self.ui.shell_cancel();
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let geometry_changed = self.layout.borrow().as_ref().is_none_or(|old| {
            old.width != layout.width
                || old.height != layout.height
                || old.viewport != layout.viewport
                || old.animation != layout.animation
        });
        self.visibility.set_generation(generation, geometry_changed);
        println!(
            "INPUT {}",
            serde_json::to_string(&serde_json::json!({
                "version": 1, "generation": generation, "layout": &layout,
            }))
            .expect("finite validated input geometry")
        );
        *self.layout.borrow_mut() = Some(layout);
    }
    fn dispatch(&self, event: Event) {
        if self.stopped.get() || event.seq() <= self.sequence.get() {
            return;
        }
        self.sequence.set(event.seq());
        self.publish(); // Revalidate geometry, selection, capabilities and freshness now.
        if let Some((generation, x, y)) = event.point() {
            if self.visibility.input_blocked() {
                self.ui.shell_cancel();
                return;
            }
            let valid = self.layout.borrow().as_ref().is_some_and(|layout| {
                generation == self.generation.get() && valid_point(layout, x, y)
            });
            if !valid {
                self.ui.shell_cancel();
                return;
            }
        }
        match event {
            Event::Visibility {
                generation,
                hidden,
                locked,
                ..
            } => {
                self.visibility.observe(generation, hidden, locked);
            }
            Event::Press { x, y, .. } => self.ui.shell_press(x, y),
            Event::Release { x, y, .. } => self.ui.shell_release(x, y),
            Event::Motion { x, y, .. } => self.ui.shell_motion(x, y),
            Event::Scroll { x, y, dy, .. } if dy.is_finite() && dy.abs() <= 20.0 => {
                self.ui.shell_scroll(x, y, dy)
            }
            Event::Cancel { .. } => self.ui.shell_cancel(),
            Event::Settings { .. } => {
                self.ui.shell_cancel();
                if self.visibility.locked() {
                    return;
                }
                self.ui.open_settings();
            }
            _ => self.ui.shell_cancel(),
        }
    }
    fn disconnect(&self, reason: &str) {
        self.stopped.set(true);
        self.ui.shell_cancel();
        tracing::warn!("{reason}; stopping owned scene source");
        if let Some(window) = self.settings.as_ref().and_then(|w| w.upgrade()) {
            if let Some(app) = window.application() {
                app.quit();
            }
        }
    }
    pub(super) fn stop(&self) {
        self.stopped.set(true);
        self.ui.shell_cancel();
        if let Some(task) = self.task.borrow_mut().take() {
            task.abort();
        }
    }
}
fn valid_point(layout: &Layout, x: f64, y: f64) -> bool {
    x.is_finite()
        && y.is_finite()
        && x >= 0.0
        && y >= 0.0
        && x < layout.width as f64
        && y < layout.height as f64
}

pub(super) fn scene_viewport(width: i32, height: i32, scale: i32, aspect: f32) -> Option<[f64; 4]> {
    if width <= 0 || height <= 0 || scale <= 0 || !aspect.is_finite() || aspect <= 0.0 {
        return None;
    }
    let ph = height.checked_mul(scale)?;
    let (x, y, w, h) =
        crate::transport_geometry::fitted_viewport(width.checked_mul(scale)?, ph, aspect);
    let rect = crate::desktop::geometry::Rect::from_gl([x, y, w, h], ph)?;
    Some([rect.x, rect.y, rect.w, rect.h].map(|n| n as f64 / scale as f64))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shell_viewport_matches_renderer_and_handles_work_area_constraints() {
        assert_eq!(
            scene_viewport(2048, 1120, 1, 16.0 / 9.0),
            Some([28.0, 0.0, 1991.0, 1120.0])
        );
        assert_eq!(
            scene_viewport(1920, 1080, 2, 16.0 / 9.0),
            Some([0.0, 0.0, 1920.0, 1080.0])
        );
        assert_eq!(
            scene_viewport(1000, 1000, 1, 16.0 / 9.0),
            Some([0.0, 219.0, 1000.0, 563.0])
        );
        assert_eq!(scene_viewport(0, 1080, 1, 16.0 / 9.0), None);
        assert_eq!(scene_viewport(1920, 1080, 0, 16.0 / 9.0), None);
        assert_eq!(scene_viewport(1920, 1080, 1, f32::NAN), None);
        assert_eq!(scene_viewport(i32::MAX, 1080, 2, 16.0 / 9.0), None);
    }
    #[test]
    fn protocol_is_bounded_typed_and_handles_fragmented_reads() {
        let mut lines = Lines::default();
        assert!(lines.push(b"{\"event\":\"set").unwrap().is_empty());
        let events = lines
            .push(b"tings\",\"seq\":1}\n{\"event\":\"cancel\",\"seq\":2}\n")
            .unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].seq(), 2);
        let visibility = lines.push(b"{\"event\":\"visibility\",\"seq\":3,\"generation\":2,\"hidden\":true,\"locked\":false}\n").unwrap();
        assert_eq!(visibility[0].seq(), 3);
        for bad in [
            r#"{"event":"exec","seq":1}"#,
            r#"{"event":"settings","seq":1,"command":"Play"}"#,
            r#"{"event":"scroll","seq":1,"generation":1,"x":0,"y":0,"dy":"NaN"}"#,
            r#"{"event":"visibility","seq":1,"generation":1,"hidden":"true","locked":false}"#,
            r#"{"event":"visibility","seq":1,"generation":1,"hidden":true}"#,
        ] {
            assert!(
                Lines::default()
                    .push(format!("{bad}\n").as_bytes())
                    .is_err()
            );
        }
        assert!(Lines::default().push(&vec![b'x'; MAX_LINE + 1]).is_err());
    }
    #[test]
    fn invalid_coordinates_are_never_forwarded() {
        let layout = Layout {
            width: 1920,
            height: 1080,
            ..Default::default()
        };
        for (x, y) in [
            (f64::NAN, 0.0),
            (0.0, f64::INFINITY),
            (-1.0, 0.0),
            (1920.0, 0.0),
            (0.0, 1080.0),
        ] {
            assert!(!valid_point(&layout, x, y));
        }
        assert!(valid_point(&layout, 1919.5, 1079.5));
    }
}
