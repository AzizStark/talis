use super::super::*;
use crate::session_bus::tests::{Bus, name, wait};
use glib::object::ObjectExt;

struct Fixture {
    connection: gio::DBusConnection,
    registration: Option<gio::RegistrationId>,
    active: Rc<Cell<bool>>,
    hold: Rc<Cell<bool>>,
    pending: Rc<RefCell<Vec<gio::DBusMethodInvocation>>>,
}
impl Fixture {
    async fn new(bus: &Bus, active: bool) -> Self {
        let connection = bus.fixture();
        let active = Rc::new(Cell::new(active));
        let hold = Rc::new(Cell::new(false));
        let pending = Rc::new(RefCell::new(vec![]));
        let info = gio::DBusNodeInfo::for_xml(
            r#"<node><interface name="org.cinnamon.ScreenSaver">
          <method name="GetActive"><arg type="b" direction="out"/></method>
          <signal name="ActiveChanged"><arg type="b"/></signal></interface></node>"#,
        )
        .unwrap();
        let registration = connection
            .register_object(PROVIDERS[0].1, &info.interfaces()[0])
            .method_call({
                let active = active.clone();
                let hold = hold.clone();
                let pending = pending.clone();
                move |_, _, _, _, _, _, invocation| {
                    if hold.get() {
                        pending.borrow_mut().push(invocation);
                    } else {
                        invocation.return_value(Some(&(active.get(),).to_variant()));
                    }
                }
            })
            .build()
            .unwrap();
        name(&connection, "RequestName", PROVIDERS[0].0).await;
        Self {
            connection,
            registration: Some(registration),
            active,
            hold,
            pending,
        }
    }
    fn signal(&self, active: bool) {
        self.active.set(active);
        self.connection
            .emit_signal(
                None,
                PROVIDERS[0].1,
                PROVIDERS[0].0,
                "ActiveChanged",
                Some(&(active,).to_variant()),
            )
            .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(id) = self.registration.take() {
            let _ = self.connection.unregister_object(id);
        }
    }
}

#[test]
#[ignore = "Requires the owned temporary session-bus fixture; terminates only its own daemon"]
fn private_bus_initial_failure_disconnect_recovery_and_shutdown() {
    let mut bus = Bus::new();
    let context = glib::MainContext::default();
    context
        .with_thread_default(|| {
            context.block_on(async {
                let monitor = Monitor::start();
                glib::timeout_future(Duration::from_millis(200)).await;
                assert!(monitor.connection.borrow().is_none());
                assert!(!monitor.active(Instant::now()));
                bus.start();
                let fixture = Fixture::new(&bus, true).await;
                wait(|| monitor.active(Instant::now())).await;
                let old = monitor.connection.borrow().clone().unwrap();
                assert!(
                    !old.property::<bool>("exit-on-close"),
                    "protection must not depend on MPRIS"
                );
                bus.stop();
                wait(|| old.is_closed()).await;
                assert!(
                    !monitor.active(Instant::now()),
                    "closed-bus lock state is immediately invalid"
                );
                wait(|| monitor.connection.borrow().is_none()).await;
                drop(fixture);
                bus.start();
                let fixture = Fixture::new(&bus, false).await;
                wait(|| {
                    monitor.providers[0]
                        .status
                        .get()
                        .is_some_and(|(active, _)| !active)
                })
                .await;
                assert_ne!(monitor.connection.borrow().as_ref().unwrap(), &old);
                fixture.signal(true);
                wait(|| monitor.active(Instant::now())).await;
                fixture.signal(false);
                wait(|| !monitor.active(Instant::now())).await;
                fixture.signal(true);
                wait(|| monitor.active(Instant::now())).await;
                name(&fixture.connection, "ReleaseName", PROVIDERS[0].0).await;
                wait(|| monitor.providers[0].owner.borrow().is_none()).await;
                assert!(!monitor.active(Instant::now()));
                name(&fixture.connection, "RequestName", PROVIDERS[0].0).await;
                wait(|| monitor.active(Instant::now())).await;
                fixture.hold.set(true);
                wait(|| !fixture.pending.borrow().is_empty()).await;
                monitor.stop();
                assert!(monitor.calls.borrow().is_empty());
                assert!(monitor.signals.borrow().is_empty());
                for invocation in fixture.pending.borrow_mut().drain(..) {
                    invocation.return_value(Some(&(true,).to_variant()));
                }
                glib::timeout_future(Duration::from_millis(100)).await;
                assert!(
                    monitor
                        .providers
                        .iter()
                        .all(|p| p.status.get().is_none() && p.owner.borrow().is_none())
                );
                assert!(!monitor.active(Instant::now()));
                // Stop before acquisition completes; a late result cannot attach subscriptions.
                let stopped = Monitor::start();
                stopped.stop();
                glib::timeout_future(Duration::from_millis(100)).await;
                assert!(stopped.connection.borrow().is_none());
                drop(fixture);
                bus.stop();
                let stopped = Monitor::start();
                glib::timeout_future(Duration::from_millis(100)).await;
                stopped.stop();
                bus.start();
                glib::timeout_future(crate::session_bus::RETRY + Duration::from_millis(100)).await;
                assert!(
                    stopped.connection.borrow().is_none(),
                    "shutdown must cancel retries"
                );
            })
        })
        .unwrap();
}
