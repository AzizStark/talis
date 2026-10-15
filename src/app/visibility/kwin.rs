//! Owned, leased KWin coverage evidence. No compositor setting or window write.
use super::*;
use gtk::gio;
use serde::{Deserialize, Serialize};

const PATH: &str = "/io/github/azizstark/Talis/Visibility";
const KWIN: &str = "org.kde.KWin";
const LEASE: Duration = Duration::from_millis(1500);
const XML: &str = r#"<node><interface name="io.github.azizstark.Talis.Visibility">
<method name="Snapshot"><arg type="s" direction="out"/></method>
<method name="Observe"><arg type="s" direction="in"/><arg type="b" direction="out"/></method>
</interface></node>"#;

#[derive(Clone, PartialEq, Serialize)]
pub(super) struct Target {
    generation: u64,
    pid: u32,
    enabled: bool,
    width: i32,
    height: i32,
    animation: [f64; 4],
}
impl Target {
    pub(super) fn from_controller(controller: &Controller) -> Option<Self> {
        let area = controller.area.upgrade()?;
        let scene = controller.scene.upgrade()?;
        let ui = controller.transport.borrow().as_ref()?.upgrade()?;
        let layout = ui.shell_layout();
        let mut animation = layout.animation?;
        let offset = area.compute_point(&scene, &gtk::graphene::Point::new(0.0, 0.0))?;
        animation[0] += offset.x() as f64;
        animation[1] += offset.y() as f64;
        let width = scene.width();
        let height = scene.height();
        if width <= 0
            || height <= 0
            || !animation.iter().all(|v| v.is_finite())
            || animation[0] < 0.0
            || animation[1] < 0.0
            || animation[2] <= 0.0
            || animation[3] <= 0.0
            || animation[0] + animation[2] > width as f64
            || animation[1] + animation[3] > height as f64
        {
            return None;
        }
        Some(Self {
            generation: generation(&area),
            pid: std::process::id(),
            enabled: layout.pause_when_hidden && scene.is_mapped(),
            width,
            height,
            animation,
        })
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Report {
    ticket: u64,
    covered: bool,
    reason: String,
}
#[derive(Default)]
struct Evidence {
    sent: Option<(u64, Target)>,
    sample: Option<(Target, Instant, bool)>,
    reason: Option<String>,
}
impl Evidence {
    fn observe(&mut self, text: &str, target: Option<Target>, now: Instant) -> bool {
        if text.len() > 512 {
            return false;
        }
        let Ok(report) = serde_json::from_str::<Report>(text) else {
            return false;
        };
        if !matches!(
            report.reason.as_str(),
            "covered" | "confirming" | "exposed" | "uncertain" | "disabled-or-show-desktop"
        ) {
            return false;
        }
        if report.covered != (report.reason == "covered") {
            return false;
        }
        let Some((ticket, sent)) = self.sent.as_ref() else {
            return false;
        };
        if report.ticket != *ticket || target.as_ref() != Some(sent) {
            return false;
        }
        if self.reason.as_deref() != Some(report.reason.as_str()) {
            tracing::debug!(reason = report.reason, "KWin coverage decision");
            self.reason = Some(report.reason);
        }
        self.sample = Some((sent.clone(), now, report.covered));
        self.sent = None; // No replay may renew a lease.
        true
    }
    fn covered(&self, target: Option<Target>, now: Instant) -> bool {
        self.sample.as_ref().is_some_and(|(sample, at, hidden)| {
            *hidden
                && sample.enabled
                && target.as_ref() == Some(sample)
                && now.saturating_duration_since(*at) <= LEASE
        })
    }
}
struct Backend {
    connection: gio::DBusConnection,
    owner: String,
    name: String,
    file: gio::File,
    registration: Option<gio::RegistrationId>,
    owner_signal: Option<gio::SignalSubscription>,
}
impl Drop for Backend {
    fn drop(&mut self) {
        self.owner_signal.take();
        if let Some(id) = self.registration.take() {
            let _ = self.connection.unregister_object(id);
        }
        // Unique owner, owned plugin name, no auto-start and no other scripts touched.
        self.connection.call(
            Some(&self.owner),
            "/Scripting",
            "org.kde.kwin.Scripting",
            "unloadScript",
            Some(&(self.name.as_str(),).to_variant()),
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            500,
            None::<&gio::Cancellable>,
            |_| {},
        );
        let _ = self.file.delete(None::<&gio::Cancellable>);
    }
}
pub(super) struct Monitor {
    target: Rc<dyn Fn() -> Option<Target>>,
    backend: RefCell<Option<Backend>>,
    evidence: RefCell<Evidence>,
    ticket: Cell<u64>,
    snapshot: RefCell<Option<glib::JoinHandle<()>>>,
    snapshot_pending: Cell<bool>,
    last_contact: Cell<Option<Instant>>,
    failure_reported: Cell<bool>,
    revision: Cell<u64>,
    task: RefCell<Option<glib::JoinHandle<()>>>,
    stopped: Cell<bool>,
}
impl Monitor {
    pub(super) fn start(target: Rc<dyn Fn() -> Option<Target>>) -> Rc<Self> {
        let this = Rc::new(Self {
            target,
            backend: RefCell::new(None),
            evidence: RefCell::new(Evidence::default()),
            ticket: Cell::new(0),
            snapshot: RefCell::new(None),
            snapshot_pending: Cell::new(false),
            last_contact: Cell::new(None),
            failure_reported: Cell::new(false),
            revision: Cell::new(0),
            task: RefCell::new(None),
            stopped: Cell::new(false),
        });
        let weak = Rc::downgrade(&this);
        *this.task.borrow_mut() = Some(glib::spawn_future_local(async move {
            loop {
                let Some(this) = weak.upgrade().filter(|m| !m.stopped.get()) else {
                    break;
                };
                let connection = crate::session_bus::connect().await;
                if let Ok(connection) = connection {
                    let owner = connection
                        .call_future(
                            Some("org.freedesktop.DBus"),
                            "/org/freedesktop/DBus",
                            "org.freedesktop.DBus",
                            "GetNameOwner",
                            Some(&(KWIN,).to_variant()),
                            None,
                            gio::DBusCallFlags::NO_AUTO_START,
                            500,
                        )
                        .await
                        .ok()
                        .and_then(|v| v.get::<(String,)>().map(|v| v.0));
                    let changed = this.backend.borrow().as_ref().is_none_or(|b| {
                        Some(b.owner.as_str()) != owner.as_deref()
                            || b.connection != connection
                            || b.connection.is_closed()
                    }) || this
                        .last_contact
                        .get()
                        .is_some_and(|at| at.elapsed() > Duration::from_secs(4));
                    if changed {
                        this.reset();
                        if let Some(owner) = owner {
                            if let Err(error) = this.install(connection, owner).await {
                                if !this.failure_reported.replace(true) {
                                    tracing::warn!(%error, "KWin coverage unavailable; rendering stays active");
                                }
                            }
                        }
                    }
                } else {
                    this.reset();
                }
                drop(this);
                glib::timeout_future(Duration::from_secs(2)).await;
            }
        }));
        this
    }
    async fn install(
        self: &Rc<Self>,
        connection: gio::DBusConnection,
        owner: String,
    ) -> anyhow::Result<()> {
        let revision = self.revision.get();
        let unique = connection
            .unique_name()
            .ok_or_else(|| anyhow::anyhow!("No bus identity"))?;
        let script = include_str!("kwin.js")
            .replace("__SERVICE__", &serde_json::to_string(unique.as_str())?);
        let (file, stream) = gio::File::new_tmp(Some("talis-kwin-XXXXXX.js"))?;
        let name = format!(
            "talis_visibility_{}_{}",
            std::process::id(),
            unique.replace([':', '.'], "_")
        );
        let mut backend = Backend {
            connection: connection.clone(),
            owner: owner.clone(),
            name,
            file,
            registration: None,
            owner_signal: None,
        };
        let (written, error) = stream
            .output_stream()
            .write_all(script.as_bytes(), None::<&gio::Cancellable>)?;
        anyhow::ensure!(
            written == script.len() && error.is_none(),
            "Incomplete temporary KWin script"
        );
        stream.close(None::<&gio::Cancellable>)?;
        let info = gio::DBusNodeInfo::for_xml(XML)?;
        let weak = Rc::downgrade(self);
        let sender_owner = owner.clone();
        backend.registration = Some(
            connection
                .register_object(PATH, &info.interfaces()[0])
                .method_call(move |connection, sender, _, _, method, args, invocation| {
                    let Some(this) = weak.upgrade().filter(|m| !m.stopped.get()) else {
                        invocation.return_dbus_error(
                            "io.github.azizstark.Talis.Stopped",
                            "Observer stopped",
                        );
                        return;
                    };
                    if sender != Some(sender_owner.as_str()) || connection.is_closed() {
                        invocation.return_dbus_error(
                            "io.github.azizstark.Talis.Denied",
                            "Only the owned KWin owner is accepted",
                        );
                        return;
                    }
                    this.last_contact.set(Some(Instant::now()));
                    if method == "Snapshot" && args.get::<()>().is_some() {
                        if this.snapshot_pending.replace(true) {
                            invocation.return_dbus_error(
                                "io.github.azizstark.Talis.Busy",
                                "One snapshot at a time",
                            );
                            return;
                        }
                        let weak = Rc::downgrade(&this);
                        let connection = connection.clone();
                        let owner = sender_owner.clone();
                        *this.snapshot.borrow_mut() = Some(glib::spawn_future_local(async move {
                            glib::timeout_future(Duration::from_millis(500)).await;
                            let Some(this) = weak.upgrade().filter(|m| !m.stopped.get()) else {
                                invocation.return_dbus_error(
                                    "io.github.azizstark.Talis.Stopped",
                                    "Observer stopped",
                                );
                                return;
                            };
                            let showing = connection
                                .call_future(
                                    Some(&owner),
                                    "/KWin",
                                    "org.freedesktop.DBus.Properties",
                                    "Get",
                                    Some(&(KWIN, "showingDesktop").to_variant()),
                                    None,
                                    gio::DBusCallFlags::NO_AUTO_START,
                                    300,
                                )
                                .await
                                .ok()
                                .and_then(|v| {
                                    v.get::<(glib::Variant,)>().and_then(|v| v.0.get::<bool>())
                                });
                            let valid = this.backend.borrow().as_ref().is_some_and(|b| {
                                b.owner == owner
                                    && b.connection == connection
                                    && !connection.is_closed()
                            });
                            let target = valid.then(|| (this.target)()).flatten();
                            let reply = if let Some(target) = target {
                                let ticket = this.ticket.get().wrapping_add(1);
                                this.ticket.set(ticket);
                                this.evidence.borrow_mut().sent = Some((ticket, target.clone()));
                                let mut value = serde_json::to_value(&target).unwrap();
                                value["ticket"] = ticket.into();
                                value["showing_desktop"] = serde_json::to_value(showing).unwrap();
                                value.to_string()
                            } else {
                                "null".into()
                            };
                            this.snapshot_pending.set(false);
                            invocation.return_value(Some(&(reply,).to_variant()));
                        }));
                    } else if method == "Observe" {
                        let accepted = args.get::<(String,)>().is_some_and(|v| {
                            this.evidence.borrow_mut().observe(
                                &v.0,
                                (this.target)(),
                                Instant::now(),
                            )
                        });
                        invocation.return_value(Some(&(accepted,).to_variant()));
                    } else {
                        invocation.return_dbus_error(
                            "io.github.azizstark.Talis.Invalid",
                            "Invalid method",
                        );
                    }
                })
                .build()?,
        );
        let weak = Rc::downgrade(self);
        let watched_owner = owner.clone();
        let signal = connection.subscribe_to_signal(
            Some("org.freedesktop.DBus"),
            Some("org.freedesktop.DBus"),
            Some("NameOwnerChanged"),
            Some("/org/freedesktop/DBus"),
            Some(KWIN),
            gio::DBusSignalFlags::NONE,
            move |signal| {
                if let Some((name, old, new)) = signal.parameters.get::<(String, String, String)>()
                {
                    if name == KWIN && old == watched_owner && new != watched_owner {
                        if let Some(this) = weak.upgrade() {
                            this.reset();
                        }
                    }
                }
            },
        );
        backend.owner_signal = Some(signal);
        let path = backend
            .file
            .path()
            .ok_or_else(|| anyhow::anyhow!("No script path"))?;
        let result = connection
            .call_future(
                Some(&owner),
                "/Scripting",
                "org.kde.kwin.Scripting",
                "loadScript",
                Some(&(path.to_string_lossy().as_ref(), backend.name.as_str()).to_variant()),
                None,
                gio::DBusCallFlags::NO_AUTO_START,
                1000,
            )
            .await?;
        let (id,) = result
            .get::<(i32,)>()
            .ok_or_else(|| anyhow::anyhow!("Invalid script identity"))?;
        anyhow::ensure!(id >= 0, "KWin refused temporary visibility script");
        anyhow::ensure!(
            !self.stopped.get() && self.revision.get() == revision,
            "KWin owner changed during installation"
        );
        // Publish before run so the first callback is already tied to this owner.
        *self.backend.borrow_mut() = Some(backend);
        self.last_contact.set(Some(Instant::now()));
        let mut result = connection
            .call_future(
                Some(&owner),
                &format!("/Scripting/Script{id}"),
                "org.kde.kwin.Script",
                "run",
                None,
                None,
                gio::DBusCallFlags::NO_AUTO_START,
                1000,
            )
            .await;
        if result.as_ref().err().is_some_and(|error| {
            gio::DBusError::remote_error(error).as_deref()
                == Some("org.freedesktop.DBus.Error.UnknownObject")
        }) {
            result = connection
                .call_future(
                    Some(&owner),
                    &format!("/{id}"),
                    "org.kde.kwin.Script",
                    "run",
                    None,
                    None,
                    gio::DBusCallFlags::NO_AUTO_START,
                    1000,
                )
                .await;
        }
        if let Err(error) = result {
            self.reset();
            return Err(error.into());
        }
        tracing::info!("KWin geometric coverage observer started");
        self.failure_reported.set(false);
        Ok(())
    }
    pub(super) fn covered(&self, now: Instant) -> bool {
        if self.stopped.get()
            || self
                .backend
                .borrow()
                .as_ref()
                .is_none_or(|b| b.connection.is_closed())
        {
            return false;
        }
        self.evidence.borrow().covered((self.target)(), now)
    }
    fn reset(&self) {
        self.revision.set(self.revision.get().wrapping_add(1));
        self.snapshot_pending.set(false);
        self.last_contact.set(None);
        if let Some(task) = self.snapshot.borrow_mut().take() {
            task.abort();
        }
        self.backend.borrow_mut().take();
        *self.evidence.borrow_mut() = Evidence::default();
    }
    pub(super) fn stop(&self) {
        self.stopped.set(true);
        if let Some(task) = self.task.borrow_mut().take() {
            task.abort();
        }
        self.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn target() -> Target {
        Target {
            generation: 1,
            pid: 1,
            enabled: true,
            width: 100,
            height: 100,
            animation: [10.0, 10.0, 50.0, 50.0],
        }
    }
    #[test]
    fn reports_are_leased_geometry_bound_bounded_and_single_use() {
        let now = Instant::now();
        let t = target();
        let mut evidence = Evidence {
            sent: Some((1, t.clone())),
            sample: None,
            reason: None,
        };
        assert!(!evidence.observe(
            r#"{"ticket":2,"covered":true,"reason":"covered"}"#,
            Some(t.clone()),
            now
        ));
        let report = r#"{"ticket":1,"covered":true,"reason":"covered"}"#;
        assert!(evidence.observe(report, Some(t.clone()), now));
        assert!(evidence.covered(Some(t.clone()), now));
        assert!(!evidence.observe(report, Some(t.clone()), now));
        assert!(!evidence.covered(Some(t.clone()), now + LEASE + Duration::from_millis(1)));
        let mut changed = t.clone();
        changed.animation[0] += 1.0;
        assert!(!evidence.covered(Some(changed), now));
        let mut disabled = t;
        disabled.enabled = false;
        assert!(!evidence.covered(Some(disabled), now));
        assert!(!evidence.observe(&"x".repeat(513), None, now));
    }
}
