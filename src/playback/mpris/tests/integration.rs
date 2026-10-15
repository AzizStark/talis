//! Run explicitly inside `dbus-run-session`, never on the user's session bus.
use super::super::*;
use std::cell::Cell;

pub(crate) struct Fixture {
    connection: gio::DBusConnection,
    pub(crate) data: Rc<RefCell<Properties>>,
    pub(crate) commands: Rc<RefCell<Vec<String>>>,
    pub(crate) command_delay: Rc<Cell<u64>>,
    pub(crate) reject: Rc<Cell<bool>>,
    pub(crate) volume_writes: Rc<RefCell<Vec<f64>>>,
    pub(crate) volume_delay: Rc<Cell<u64>>,
    /// Acknowledge Set immediately, but publish its value later.
    pub(crate) volume_apply_delay: Rc<Cell<u64>>,
    pub(crate) volume_emit_signal: Rc<Cell<bool>>,
    pub(crate) volume_reads: Rc<Cell<u32>>,
    pub(crate) volume_reject: Rc<Cell<bool>>,
    pub(crate) volume_noop: Rc<Cell<bool>>,
    command_registration: Option<gio::RegistrationId>,
    pub(crate) delay: Rc<Cell<u64>>,
    calls: Rc<Cell<u32>>,
    registration: Option<gio::RegistrationId>,
}
impl Fixture {
    pub(crate) async fn new(name: &str, status: &str, position: i64, length: i64) -> Self {
        let connection = gio::DBusConnection::for_address_sync(
            &std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap(),
            gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
            None,
            None::<&gio::Cancellable>,
        )
        .unwrap();
        connection.set_exit_on_close(false);
        let data = Rc::new(RefCell::new(Properties::new()));
        let delay = Rc::new(Cell::new(0));
        let calls = Rc::new(Cell::new(0));
        let commands = Rc::new(RefCell::new(Vec::new()));
        let command_delay = Rc::new(Cell::new(0));
        let reject = Rc::new(Cell::new(false));
        let volume_writes = Rc::new(RefCell::new(Vec::new()));
        let volume_delay = Rc::new(Cell::new(0));
        let volume_apply_delay = Rc::new(Cell::new(0));
        let volume_emit_signal = Rc::new(Cell::new(true));
        let volume_reads = Rc::new(Cell::new(0));
        let volume_reject = Rc::new(Cell::new(false));
        let volume_noop = Rc::new(Cell::new(false));
        let player = gio::DBusNodeInfo::for_xml(
            r#"<node><interface name="org.mpris.MediaPlayer2.Player">
          <method name="Play"/><method name="Pause"/><method name="Previous"/><method name="Next"/>
        </interface></node>"#,
        )
        .unwrap();
        let command_registration = connection
            .register_object(PATH, &player.interfaces()[0])
            .method_call({
                let data = data.clone();
                let commands = commands.clone();
                let delay = command_delay.clone();
                let reject = reject.clone();
                move |connection, _, _, _, method, _, invocation| {
                    commands.borrow_mut().push(method.into());
                    if reject.get() {
                        invocation.return_dbus_error(
                            "org.freedesktop.DBus.Error.NotSupported",
                            "Fixture rejected command",
                        );
                        return;
                    }
                    {
                        let mut p = data.borrow_mut();
                        match method {
                            "Play" => {
                                p.insert("PlaybackStatus".into(), "Playing".to_variant());
                            }
                            "Pause" => {
                                p.insert("PlaybackStatus".into(), "Paused".to_variant());
                            }
                            "Next" | "Previous" => {
                                p.insert("Position".into(), 0_i64.to_variant());
                                if let Some(mut m) =
                                    p.get("Metadata").and_then(|v| v.get::<Properties>())
                                {
                                    m.insert(
                                        "mpris:trackid".into(),
                                        glib::variant::ObjectPath::try_from(format!(
                                            "/track/{method}"
                                        ))
                                        .unwrap()
                                        .to_variant(),
                                    );
                                    p.insert("Metadata".into(), m.to_variant());
                                }
                            }
                            _ => panic!("Unexpected fixture method: {method}"),
                        }
                    }
                    connection
                        .emit_signal(
                            None,
                            PATH,
                            PROPERTIES,
                            "PropertiesChanged",
                            Some(
                                &(PLAYER, data.borrow().clone(), Vec::<String>::new()).to_variant(),
                            ),
                        )
                        .unwrap();
                    let ms = delay.replace(0);
                    if ms == 0 {
                        invocation.return_value(Some(&().to_variant()));
                    } else {
                        glib::timeout_add_local_once(Duration::from_millis(ms), move || {
                            invocation.return_value(Some(&().to_variant()))
                        });
                    }
                }
            })
            .build()
            .unwrap();
        let info = gio::DBusNodeInfo::for_xml(r#"<node><interface name="org.freedesktop.DBus.Properties">
          <method name="GetAll"><arg type="s" direction="in"/><arg type="a{sv}" direction="out"/></method>
          <method name="Get"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="out"/></method>
          <method name="Set"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="in"/></method>
        </interface></node>"#).unwrap();
        let registration = connection
            .register_object(PATH, &info.interfaces()[0])
            .method_call({
                let data = data.clone();
                let delay = delay.clone();
                let calls = calls.clone();
                let volume_writes = volume_writes.clone();
                let volume_delay = volume_delay.clone();
                let volume_apply_delay = volume_apply_delay.clone();
                let volume_emit_signal = volume_emit_signal.clone();
                let volume_reads = volume_reads.clone();
                let volume_reject = volume_reject.clone();
                let volume_noop = volume_noop.clone();
                move |connection, _, _, _, method, parameters, invocation| {
                    if method == "Get" {
                        volume_reads.set(volume_reads.get() + 1);
                        assert_eq!(
                            parameters.get::<(String, String)>().unwrap(),
                            (PLAYER.into(), "Volume".into())
                        );
                        if let Some(value) = data.borrow().get("Volume") {
                            invocation.return_value(Some(&(value.clone(),).to_variant()));
                        } else {
                            invocation.return_dbus_error(
                                "org.freedesktop.DBus.Error.UnknownProperty",
                                "Volume missing",
                            );
                        }
                        return;
                    }
                    if method == "Set" {
                        let (interface, key, value) =
                            parameters.get::<(String, String, glib::Variant)>().unwrap();
                        assert_eq!((interface.as_str(), key.as_str()), (PLAYER, "Volume"));
                        let value = value.get::<f64>().unwrap();
                        assert!((0.0..=1.0).contains(&value));
                        volume_writes.borrow_mut().push(value);
                        if volume_reject.get() {
                            invocation.return_dbus_error(
                                "org.freedesktop.DBus.Error.PropertyReadOnly",
                                "Read-only volume",
                            );
                            return;
                        }
                        let data = data.clone();
                        let noop = volume_noop.get();
                        let emit_signal = volume_emit_signal.get();
                        let apply_delay = volume_apply_delay.get();
                        let invocation = if apply_delay > 0 {
                            invocation.return_value(Some(&().to_variant()));
                            None
                        } else {
                            Some(invocation)
                        };
                        let apply = move || {
                            if !noop {
                                data.borrow_mut()
                                    .insert("Volume".into(), value.to_variant());
                                let changed =
                                    Properties::from([("Volume".into(), value.to_variant())]);
                                if emit_signal {
                                    connection
                                        .emit_signal(
                                            None,
                                            PATH,
                                            PROPERTIES,
                                            "PropertiesChanged",
                                            Some(
                                                &(PLAYER, changed, Vec::<String>::new())
                                                    .to_variant(),
                                            ),
                                        )
                                        .unwrap();
                                }
                            }
                            if let Some(invocation) = invocation {
                                invocation.return_value(Some(&().to_variant()));
                            }
                        };
                        let ms = if apply_delay > 0 {
                            apply_delay
                        } else {
                            volume_delay.replace(0)
                        };
                        if ms == 0 {
                            apply();
                        } else {
                            glib::timeout_add_local_once(Duration::from_millis(ms), apply);
                        }
                        return;
                    }
                    assert_eq!(method, "GetAll", "unexpected fixture property method");
                    assert_eq!(parameters.get::<(String,)>().unwrap().0, PLAYER);
                    calls.set(calls.get() + 1);
                    let reply = (data.borrow().clone(),).to_variant();
                    let delay_ms = delay.replace(0);
                    if delay_ms == 0 {
                        invocation.return_value(Some(&reply));
                    } else {
                        glib::timeout_add_local_once(Duration::from_millis(delay_ms), move || {
                            invocation.return_value(Some(&reply));
                        });
                    }
                }
            })
            .build()
            .unwrap();
        let fixture = Self {
            connection,
            data,
            delay,
            calls,
            registration: Some(registration),
            commands,
            command_delay,
            reject,
            command_registration: Some(command_registration),
            volume_writes,
            volume_delay,
            volume_apply_delay,
            volume_emit_signal,
            volume_reads,
            volume_reject,
            volume_noop,
        };
        fixture.track("one", status, position, length);
        fixture.name("RequestName", name).await;
        fixture
    }
    pub(crate) async fn name(&self, method: &str, name: &str) {
        let parameters = if method == "RequestName" {
            (name, 0u32).to_variant()
        } else {
            (name,).to_variant()
        };
        self.connection
            .call_future(
                Some(BUS),
                BUS_PATH,
                BUS,
                method,
                Some(&parameters),
                None,
                gio::DBusCallFlags::NO_AUTO_START,
                TIMEOUT_MS,
            )
            .await
            .unwrap();
    }
    pub(crate) fn track(&self, id: &str, status: &str, position: i64, length: i64) {
        let metadata: Properties = [
            (
                "mpris:trackid".into(),
                glib::variant::ObjectPath::try_from(format!("/track/{id}"))
                    .unwrap()
                    .to_variant(),
            ),
            ("mpris:length".into(), length.to_variant()),
        ]
        .into();
        *self.data.borrow_mut() = [
            ("Metadata".into(), metadata.to_variant()),
            ("PlaybackStatus".into(), status.to_variant()),
            ("Rate".into(), 1.0f64.to_variant()),
            ("Position".into(), position.to_variant()),
            ("CanControl".into(), true.to_variant()),
            ("CanPlay".into(), true.to_variant()),
            ("CanPause".into(), true.to_variant()),
            ("CanGoNext".into(), true.to_variant()),
            ("CanGoPrevious".into(), true.to_variant()),
        ]
        .into();
    }
    pub(crate) fn changed(&self) {
        self.connection
            .emit_signal(
                None,
                PATH,
                PROPERTIES,
                "PropertiesChanged",
                Some(&(PLAYER, self.data.borrow().clone(), Vec::<String>::new()).to_variant()),
            )
            .unwrap();
    }
    pub(crate) fn volume_changed(&self, value: f64) {
        self.data
            .borrow_mut()
            .insert("Volume".into(), value.to_variant());
        self.connection
            .emit_signal(
                None,
                PATH,
                PROPERTIES,
                "PropertiesChanged",
                Some(
                    &(
                        PLAYER,
                        Properties::from([("Volume".into(), value.to_variant())]),
                        Vec::<String>::new(),
                    )
                        .to_variant(),
                ),
            )
            .unwrap();
    }
    fn seek(&self, position: i64) {
        self.data
            .borrow_mut()
            .insert("Position".into(), position.to_variant());
        self.connection
            .emit_signal(
                None,
                PATH,
                PLAYER,
                "Seeked",
                Some(&(position,).to_variant()),
            )
            .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(id) = self.command_registration.take() {
            self.connection.unregister_object(id).unwrap();
        }
        if let Some(id) = self.registration.take() {
            self.connection.unregister_object(id).unwrap();
        }
        let _ = self.connection.close_sync(None::<&gio::Cancellable>);
    }
}
pub(crate) async fn wait_for(
    feed: &Rc<RefCell<PlaybackFeed>>,
    label: &str,
    check: impl Fn(&PlaybackFeed) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(4);
    while !check(&feed.borrow()) && Instant::now() < deadline {
        glib::timeout_future(Duration::from_millis(25)).await;
    }
    assert!(check(&feed.borrow()), "{label}: {:?}", feed.borrow());
}
fn paused_progress(feed: &PlaybackFeed, value: f32) -> bool {
    feed.status == Status::Paused
        && feed
            .progress_at(Instant::now())
            .is_some_and(|p| (p - value).abs() < 0.00001)
}

#[test]
#[ignore = "Requires a private dbus-run-session; set TALIS_MPRIS_TEST_BUS=1"]
fn private_bus_timing_only_and_missing_rate_compatibility() {
    assert_eq!(std::env::var("TALIS_MPRIS_TEST_BUS").as_deref(), Ok("1"));
    let context = glib::MainContext::default();
    context
        .with_thread_default(|| {
            context.block_on(async {
                let name = "org.mpris.MediaPlayer2.talis_timing_fixture";
                let fixture = Fixture::new(name, "Playing", 158_455_000, 238_027_000).await;
                let timing_only = |status: &str, position: i64, duration: i64| {
                    fixture.track("unused", status, position, duration);
                    let mut data = fixture.data.borrow_mut();
                    let mut metadata = data["Metadata"].get::<Properties>().unwrap();
                    metadata.insert(
                        "mpris:trackid".into(),
                        glib::variant::ObjectPath::try_from(
                            "/org/mpris/MediaPlayer2/TrackList/NoTrack",
                        )
                        .unwrap()
                        .to_variant(),
                    );
                    data.insert("Metadata".into(), metadata.to_variant());
                };
                timing_only("Playing", 158_455_000, 238_027_000);
                let feed = Rc::new(RefCell::new(PlaybackFeed::default()));
                feed.borrow_mut().set_enabled(true);
                let monitor = Monitor::start(feed.clone(), || {});
                wait_for(&feed, "Totem timing without track identity", |f| {
                    f.moving(Instant::now())
                        && f.snapshot
                            .as_ref()
                            .is_some_and(|s| s.position == Duration::from_micros(158_455_000))
                })
                .await;
                assert!(feed.borrow().artwork.is_none());
                assert!(feed.borrow().artwork_track.is_none());
                assert!(feed.borrow().transport.is_some());

                timing_only("Paused", 30_000_000, 120_000_000);
                fixture.changed();
                wait_for(&feed, "timing-only pause", |f| paused_progress(f, 0.25)).await;
                fixture.seek(90_000_000);
                wait_for(&feed, "timing-only seek", |f| paused_progress(f, 0.75)).await;
                timing_only("Paused", 90_000_000, 180_000_000);
                fixture.changed();
                wait_for(&feed, "timing-only duration change", |f| {
                    paused_progress(f, 0.5)
                })
                .await;
                timing_only("Stopped", 90_000_000, 180_000_000);
                fixture.changed();
                wait_for(&feed, "stopped timing-only source", |f| {
                    f.status == Status::Stopped && f.snapshot.is_some()
                })
                .await;
                assert!(feed.borrow().progress_at(Instant::now()).is_none());
                assert!(!feed.borrow().sleeve_visible());

                // Reproduce Firefox's omitted Rate and generic track identity, with artwork.
                fixture.track("firefox", "Playing", 168_000_000, 231_000_000);
                {
                    let mut data = fixture.data.borrow_mut();
                    data.remove("Rate");
                    let mut metadata = data["Metadata"].get::<Properties>().unwrap();
                    metadata.insert(
                        "mpris:trackid".into(),
                        glib::variant::ObjectPath::try_from("/org/mpris/MediaPlayer2/firefox")
                            .unwrap()
                            .to_variant(),
                    );
                    metadata.insert("xesam:title".into(), "Video title".to_variant());
                    metadata.insert(
                        "xesam:url".into(),
                        "https://example.test/video".to_variant(),
                    );
                    metadata.insert(
                        "mpris:artUrl".into(),
                        "file:///tmp/firefox-test.png".to_variant(),
                    );
                    data.insert("Metadata".into(), metadata.to_variant());
                }
                fixture.changed();
                wait_for(&feed, "Firefox without Rate", |f| {
                    f.moving(Instant::now()) && f.artwork.is_some()
                })
                .await;
                let snapshot = feed.borrow().snapshot.clone().unwrap();
                assert_eq!(snapshot.rate, 1.0);
                assert_eq!(
                    snapshot.position_at(snapshot.observed_at + Duration::from_secs(2)),
                    Duration::from_secs(170)
                );
                let firefox_track = feed.borrow().artwork_track.clone();
                fixture
                    .data
                    .borrow_mut()
                    .insert("PlaybackStatus".into(), "Paused".to_variant());
                fixture.changed();
                wait_for(&feed, "Firefox pause without Rate", |f| {
                    paused_progress(f, 168.0 / 231.0)
                })
                .await;
                fixture.seek(21_000_000);
                wait_for(&feed, "Firefox seek without Rate", |f| {
                    paused_progress(f, 21.0 / 231.0)
                })
                .await;
                assert_eq!(feed.borrow().artwork_track, firefox_track);

                for invalid_rate in [
                    "wrong".to_variant(),
                    0.0_f64.to_variant(),
                    f64::NAN.to_variant(),
                ] {
                    fixture
                        .data
                        .borrow_mut()
                        .insert("Rate".into(), invalid_rate);
                    fixture.changed();
                    wait_for(&feed, "present malformed Rate must not fall back", |f| {
                        f.snapshot.is_none() && f.notice == "Invalid playback rate"
                    })
                    .await;
                    assert!(
                        feed.borrow().artwork.is_some(),
                        "bad timing must not erase artwork"
                    );
                    fixture.data.borrow_mut().remove("Rate");
                    fixture.changed();
                    wait_for(&feed, "recover after malformed Rate", |f| {
                        paused_progress(f, 21.0 / 231.0)
                    })
                    .await;
                }

                // Unknown duration remains unknown, even when an otherwise valid clock exists.
                timing_only("Playing", 124_474_307, 0);
                fixture.changed();
                wait_for(&feed, "unknown duration is not guessed", |f| {
                    f.status == Status::Playing
                        && f.snapshot.as_ref().is_some_and(|s| s.duration.is_zero())
                })
                .await;
                assert!(feed.borrow().progress_at(Instant::now()).is_none());
                fixture.name("ReleaseName", name).await;
                wait_for(&feed, "timing-only player closes", |f| {
                    f.active.is_none() && f.snapshot.is_none()
                })
                .await;
                timing_only("Paused", 20_000_000, 100_000_000);
                fixture.name("RequestName", name).await;
                wait_for(&feed, "timing-only player returns", |f| {
                    paused_progress(f, 0.2)
                })
                .await;
                monitor.stop();
                assert!(fixture.commands.borrow().is_empty());
                assert!(fixture.volume_writes.borrow().is_empty());
            })
        })
        .unwrap();
}

#[test]
#[ignore = "Requires a private dbus-run-session; set TALIS_MPRIS_TEST_BUS=1"]
fn private_bus_track_seek_timeout_and_player_recovery() {
    assert_eq!(
        std::env::var("TALIS_MPRIS_TEST_BUS").as_deref(),
        Ok("1"),
        "Use a private test bus"
    );
    let context = glib::MainContext::default();
    context
        .with_thread_default(|| {
            context.block_on(async {
                let name = "org.mpris.MediaPlayer2.talis_fixture";
                let fixture = Fixture::new(name, "Paused", 20_000_000, 100_000_000).await;
                let feed = Rc::new(RefCell::new(PlaybackFeed::default()));
                feed.borrow_mut().set_enabled(true);
                let notifications = Rc::new(Cell::new(0));
                let monitor = Monitor::start(feed.clone(), {
                    let count = notifications.clone();
                    move || count.set(count.get() + 1)
                });
                wait_for(&feed, "initial paused timing", |f| paused_progress(f, 0.2)).await;
                assert!(!feed.borrow().moving(Instant::now()));
                assert!(
                    feed.borrow().sleeve_visible(),
                    "paused player lost its sleeve"
                );

                // Capture an old reply, then switch tracks while it is still in flight.
                fixture.delay.set(500);
                let calls = fixture.calls.get();
                fixture.seek(20_000_000);
                let deadline = Instant::now() + Duration::from_secs(4);
                while fixture.calls.get() == calls && Instant::now() < deadline {
                    glib::timeout_future(Duration::from_millis(10)).await;
                }
                assert!(
                    fixture.calls.get() > calls,
                    "fixture must receive the delayed query"
                );
                fixture.track("two", "Paused", 60_000_000, 120_000_000);
                fixture.changed();
                glib::timeout_future(Duration::from_millis(150)).await;
                assert!(
                    feed.borrow().snapshot.is_none(),
                    "old clock must be invalidated immediately"
                );
                wait_for(&feed, "stale old-track reply discarded", |f| {
                    paused_progress(f, 0.5)
                })
                .await;
                fixture.seek(30_000_000);
                wait_for(&feed, "seek", |f| paused_progress(f, 0.25)).await;
                fixture.track("two", "Paused", 30_000_000, 60_000_000);
                fixture.changed();
                wait_for(&feed, "duration update", |f| paused_progress(f, 0.5)).await;
                fixture.track("three", "Paused", 15_000_000, 300_000_000);
                fixture.changed();
                fixture.track("four", "Paused", 20_000_000, 200_000_000);
                fixture.changed();
                wait_for(&feed, "rapid skips", |f| paused_progress(f, 0.1)).await;

                fixture.track("four", "Playing", 40_000_000, 200_000_000);
                fixture
                    .data
                    .borrow_mut()
                    .insert("Rate".into(), 2.0f64.to_variant());
                fixture.changed();
                wait_for(&feed, "resume with rate", |f| f.moving(Instant::now())).await;
                let a = feed
                    .borrow()
                    .snapshot
                    .as_ref()
                    .unwrap()
                    .position_at(Instant::now());
                glib::timeout_future(Duration::from_millis(200)).await;
                let b = feed
                    .borrow()
                    .snapshot
                    .as_ref()
                    .unwrap()
                    .position_at(Instant::now());
                assert!(b.saturating_sub(a) >= Duration::from_millis(350));
                fixture.track("four", "Paused", 42_000_000, 200_000_000);
                fixture.changed();
                wait_for(&feed, "pause", |f| paused_progress(f, 0.21)).await;

                fixture.track("four", "Stopped", 42_000_000, 200_000_000);
                fixture.changed();
                wait_for(&feed, "stopped playback", |f| {
                    f.status == Status::Stopped && f.snapshot.is_some()
                })
                .await;
                assert!(feed.borrow().progress_at(Instant::now()).is_none());
                assert_eq!(feed.borrow().notice, "Playback stopped");
                assert!(!feed.borrow().sleeve_visible());

                fixture.track("live", "Playing", 0, 0);
                fixture.changed();
                wait_for(&feed, "unknown duration", |f| {
                    f.snapshot.as_ref().is_some_and(|s| s.duration.is_zero())
                })
                .await;
                assert!(feed.borrow().progress_at(Instant::now()).is_none());
                let calls = fixture.calls.get();
                assert!(
                    feed.borrow().sleeve_visible(),
                    "live stream lost its sleeve"
                );
                for _ in 0..10 {
                    fixture.changed();
                }
                glib::timeout_future(Duration::from_millis(300)).await;
                assert!(
                    fixture.calls.get() <= calls + 1,
                    "duplicate metadata must not cause request storms"
                );
                fixture
                    .data
                    .borrow_mut()
                    .insert("Rate".into(), f64::NAN.to_variant());
                fixture.changed();
                wait_for(&feed, "malformed rate", |f| {
                    f.snapshot.is_none() && f.notice == "Invalid playback rate"
                })
                .await;

                fixture.track("timeout", "Paused", 10_000_000, 100_000_000);
                fixture.delay.set(1800);
                fixture.changed();
                wait_for(&feed, "bounded timeout", |f| f.notice.contains("timed out")).await;
                fixture.track("recovery", "Paused", 80_000_000, 100_000_000);
                fixture.changed();
                wait_for(&feed, "recover from timeout", |f| paused_progress(f, 0.8)).await;
                glib::timeout_future(Duration::from_millis(650)).await;
                assert!(
                    paused_progress(&feed.borrow(), 0.8),
                    "late timed-out response must not roll back timing"
                );

                // A paused source yields to a playing source; explicit preference never does.
                let second_name = "org.mpris.MediaPlayer2.talis_fixture_second";
                let second = Fixture::new(second_name, "Playing", 20_000_000, 100_000_000).await;
                wait_for(&feed, "automatic playing-source selection", |f| {
                    f.active.as_deref() == Some(second_name) && f.moving(Instant::now())
                })
                .await;
                feed.borrow_mut().select(Some(name.into()));
                wait_for(&feed, "explicit paused-source selection", |f| {
                    f.active.as_deref() == Some(name) && paused_progress(f, 0.8)
                })
                .await;
                fixture.name("ReleaseName", name).await;
                wait_for(&feed, "selected player exits", |f| {
                    !f.players.iter().any(|p| p == name) && f.snapshot.is_none()
                })
                .await;
                fixture.track("reopened", "Paused", 5_000_000, 100_000_000);
                assert!(
                    !feed.borrow().sleeve_visible(),
                    "missing selected player retained a sleeve"
                );
                fixture.name("RequestName", name).await;
                wait_for(&feed, "selected player returns", |f| {
                    paused_progress(f, 0.05)
                })
                .await;

                feed.borrow_mut().set_enabled(false);
                assert!(!feed.borrow().sleeve_visible());
                glib::timeout_future(Duration::from_millis(350)).await;
                let calls = fixture.calls.get() + second.calls.get();
                glib::timeout_future(Duration::from_millis(400)).await;
                assert_eq!(
                    fixture.calls.get() + second.calls.get(),
                    calls,
                    "manual mode must stop D-Bus reads"
                );
                feed.borrow_mut().set_enabled(true);
                wait_for(&feed, "reenable", |f| paused_progress(f, 0.05)).await;
                // Losing the client connection must not terminate the process or stay stuck.
                let connection = monitor.0.borrow().connection.clone().unwrap();
                connection.close_future().await.unwrap();
                drop(connection);
                wait_for(&feed, "session connection lost", |f| f.snapshot.is_none()).await;
                wait_for(&feed, "session connection recovered", |f| {
                    paused_progress(f, 0.05)
                })
                .await;
                monitor.stop();
                assert!(
                    fixture.commands.borrow().is_empty() && second.commands.borrow().is_empty(),
                    "read-only monitoring sent transport commands"
                );
                let count = notifications.get();
                glib::timeout_future(Duration::from_millis(400)).await;
                assert_eq!(count, notifications.get(), "stop must remove callbacks");
            })
        })
        .unwrap();
}

#[test]
#[ignore = "Requires the owned temporary session-bus fixture; terminates only its own daemon"]
fn private_bus_daemon_restart_recovers_playback() {
    use crate::session_bus::tests::{Bus, wait};
    let mut bus = Bus::new();
    let context = glib::MainContext::default();
    context
        .with_thread_default(|| {
            context.block_on(async {
                let feed = Rc::new(RefCell::new(PlaybackFeed::default()));
                feed.borrow_mut().set_enabled(true);
                let monitor = Monitor::start(feed.clone(), || {});
                glib::timeout_future(Duration::from_millis(200)).await;
                assert!(monitor.0.borrow().connection.is_none());
                bus.start();
                let fixture = Fixture::new(
                    "org.mpris.MediaPlayer2.talis_bus_recovery",
                    "Paused",
                    20_000_000,
                    100_000_000,
                )
                .await;
                wait(|| paused_progress(&feed.borrow(), 0.2)).await;
                let old = monitor.0.borrow().connection.clone().unwrap();
                assert!(!old.property::<bool>("exit-on-close"));
                bus.stop();
                wait(|| old.is_closed() && monitor.0.borrow().connection.is_none()).await;
                assert!(feed.borrow().transport.is_none());
                drop(fixture);
                bus.start();
                let fixture = Fixture::new(
                    "org.mpris.MediaPlayer2.talis_bus_recovery",
                    "Paused",
                    70_000_000,
                    100_000_000,
                )
                .await;
                wait(|| paused_progress(&feed.borrow(), 0.7)).await;
                assert_ne!(monitor.0.borrow().connection.as_ref().unwrap(), &old);
                monitor.stop();
                drop(fixture);
            })
        })
        .unwrap();
}

#[test]
#[ignore = "requires private dbus-run-session; TALIS_MPRIS_TEST_BUS=1, no display"]
fn private_bus_previous_artwork_history_and_loading() {
    previous_artwork_scenario(false, false, false, false);
}

#[test]
#[ignore = "requires private dbus-run-session; TALIS_MPRIS_TEST_BUS=1, no display"]
fn private_bus_firefox_reused_track_id_and_memory_history() {
    previous_artwork_scenario(true, false, false, false);
}

#[test]
#[ignore = "requires private dbus-run-session; TALIS_MPRIS_TEST_BUS=1, no display"]
fn private_bus_missing_track_id_and_memory_history() {
    previous_artwork_scenario(true, true, false, false);
}

#[test]
#[ignore = "requires private dbus-run-session; TALIS_MPRIS_TEST_BUS=1, no display"]
fn private_bus_elisa_embedded_artwork_and_memory_history() {
    previous_artwork_scenario(false, false, true, false);
}

#[test]
#[ignore = "requires private dbus-run-session; TALIS_MPRIS_TEST_BUS=1, no display"]
fn private_bus_mislabeled_inline_artwork_and_memory_history() {
    previous_artwork_scenario(false, false, true, true);
}

fn previous_artwork_scenario(fallback: bool, missing_id: bool, embedded: bool, mislabeled: bool) {
    assert_eq!(std::env::var("TALIS_MPRIS_TEST_BUS").as_deref(), Ok("1"));
    let directory = std::process::Command::new("mktemp")
        .args(["-d", "/tmp/talis-history.XXXXXXXX"])
        .output()
        .unwrap();
    assert!(directory.status.success());
    let directory = std::path::PathBuf::from(String::from_utf8(directory.stdout).unwrap().trim());
    for (name, color) in [
        ("a", [245, 30, 20, 255]),
        ("b", [25, 230, 40, 255]),
        ("b-refresh", [25, 230, 40, 255]),
        ("d", [30, 40, 240, 255]),
    ] {
        let mut pixels = image::RgbaImage::from_pixel(128, 128, image::Rgba(color));
        if embedded {
            // Incompressible enough to exceed the old 8 KB URL limit; retain
            // the first pixel as the current/history identity assertion.
            let mut noise = if name == "b-refresh" {
                0x87654321_u32
            } else {
                0x12345678_u32
            };
            for pixel in pixels.pixels_mut().skip(1) {
                noise ^= noise << 13;
                noise ^= noise >> 17;
                noise ^= noise << 5;
                *pixel = image::Rgba([noise as u8, (noise >> 8) as u8, (noise >> 16) as u8, 255]);
            }
        }
        pixels.save(directory.join(format!("{name}.png"))).unwrap();
    }
    let sources: HashMap<&str, String> = ["a", "b", "b-refresh", "d"]
        .into_iter()
        .map(|id| {
            let path = directory.join(format!("{id}.png"));
            let url = if embedded {
                use base64::{Engine, engine::general_purpose::STANDARD};
                let url = format!(
                    "data:image/{};base64,{}",
                    if mislabeled { "jpeg" } else { "png" },
                    STANDARD.encode(std::fs::read(path).unwrap())
                );
                assert!(url.len() > 8192);
                url
            } else {
                glib::filename_to_uri(path, None).unwrap().to_string()
            };
            (id, url)
        })
        .collect();
    let context = glib::MainContext::default();
    context.with_thread_default(|| context.block_on(async {
        let name = "org.mpris.MediaPlayer2.talis_history_fixture";
        let fixture = Fixture::new(name,"Paused",0,0).await;
        let feed = Rc::new(RefCell::new(PlaybackFeed::default()));
        feed.borrow_mut().set_enabled(true);
        let loader = crate::artwork::Loader::new(|| {});
        let monitor = Monitor::start(feed.clone(), {
            let feed=feed.clone(); let loader=loader.clone();
            move || loader.sync(&feed.borrow())
        });
        let track = |id: &str, status: &str, art: bool| {
            // Zero duration deliberately: artwork/history must not need timing.
            fixture.track(id,status,0,0);
            let mut p=fixture.data.borrow_mut();
            let mut metadata=p["Metadata"].get::<Properties>().unwrap();
            if fallback {
                if missing_id { metadata.remove("mpris:trackid"); }
                else { metadata.insert("mpris:trackid".into(),glib::variant::ObjectPath::try_from("/org/mpris/MediaPlayer2/firefox").unwrap().to_variant()); }
                metadata.insert("xesam:title".into(),id.to_variant());
                metadata.insert("xesam:artist".into(),vec!["same artist"].to_variant());
                metadata.insert("xesam:album".into(),"same album".to_variant());
                // Browser page URL can remain the same while its music changes.
                metadata.insert("xesam:url".into(),"https://example.org/player".to_variant());
            }
            if art {
                metadata.insert("mpris:artUrl".into(),sources[id].to_variant());
            }
            p.insert("Metadata".into(),metadata.to_variant()); drop(p);
            fixture.changed();
        };
        let expected = || identity::Tracker::default().observe(&fixture.data.borrow()).unwrap();
        async fn image(loader: &crate::artwork::Loader, previous: bool, color: Option<[u8;4]>) {
            let deadline=Instant::now()+Duration::from_secs(3);
            loop {
                let actual=if previous { loader.previous_snapshot().1 } else { loader.snapshot().1 };
                let actual=actual.as_ref().map(|i| i.get_pixel(0,0).0);
                if actual==color { break; }
                assert!(Instant::now()<deadline,"history image: previous={previous} actual={actual:?} expected={color:?}");
                glib::timeout_future(Duration::from_millis(25)).await;
            }
        }
        track("a","Paused",true);
        let a=expected();
        wait_for(&feed,"A metadata",|f| f.artwork_track.as_ref()==Some(&a)).await;
        image(&loader,false,Some([245,30,20,255])).await;
        image(&loader,true,None).await;
        // Firefox removes old cover files. Promotion must reuse decoded RAM,
        // not require the previous source image to remain available on disk.
        std::fs::remove_file(directory.join("a.png")).unwrap();
        track("b","Playing",true);
        let b=expected(); assert_ne!(a,b);
        wait_for(&feed,"B metadata",|f| f.artwork_track.as_ref()==Some(&b)).await;
        image(&loader,true,Some([245,30,20,255])).await;
        image(&loader,false,Some([25,230,40,255])).await;
        let revision=loader.previous_snapshot().0;
        let refreshed=sources["b-refresh"].clone();
        assert_ne!(sources["b"],refreshed,"same-song refresh must exercise a changed source");
        {
            let mut p=fixture.data.borrow_mut();
            let mut m=p["Metadata"].get::<Properties>().unwrap();
            m.insert("mpris:artUrl".into(),refreshed.to_variant());
            p.insert("Metadata".into(),m.to_variant());
        }
        fixture.changed();
        wait_for(&feed,"same-song artwork refresh",|f| f.artwork.as_ref().is_some_and(|a| a.url.as_ref()==refreshed)).await;
        image(&loader,false,Some([25,230,40,255])).await;
        assert_eq!(loader.previous_snapshot().0,revision);
        for status in ["Paused","Playing","Paused"] {
            track("b",status,true);
            wait_for(&feed,"B status",|f| f.artwork_playback.is_some_and(|(s,_)| match status { "Playing"=>s==Status::Playing,"Stopped"=>s==Status::Stopped,_=>s==Status::Paused })).await;
            assert_eq!(loader.previous_snapshot().0,revision,"status changed history");
        }
        track("b","Stopped",true);
        wait_for(&feed,"stopped exposes the steel holder",|f| !f.sleeve_visible()).await;
        image(&loader,true,None).await;
        let revision=loader.previous_snapshot().0;
        glib::timeout_future(Duration::from_millis(200)).await;
        assert_eq!(loader.previous_snapshot().0,revision,"idle stop repeatedly updated artwork");
        track("b","Paused",true);
        wait_for(&feed,"resume with fresh history",|f| f.sleeve_visible()).await;
        image(&loader,true,None).await;
        assert_eq!(loader.previous_snapshot().0,revision,"resume restored pre-stop history");
        if fallback {
            let mut m=Properties::new();
            if !missing_id { m.insert("mpris:trackid".into(),glib::variant::ObjectPath::try_from("/org/mpris/MediaPlayer2/firefox").unwrap().to_variant()); }
            fixture.data.borrow_mut().insert("Metadata".into(),m.to_variant()); fixture.changed();
            wait_for(&feed,"blank browser metadata",|f| f.artwork_track.is_none()).await;
            assert_eq!(loader.previous_snapshot().0,revision);
            track("b","Paused",true);
            wait_for(&feed,"browser metadata restored",|f| f.artwork_track.as_ref()==Some(&b)).await;
            image(&loader,false,Some([25,230,40,255])).await;
            // Repeated GetAll polls must keep the same identity as signals.
            glib::timeout_future(Duration::from_millis(2150)).await;
            assert_eq!(loader.previous_snapshot().0,revision);
        }
        track("c","Paused",false);
        let c=expected();
        wait_for(&feed,"C without artwork",|f| f.artwork_track.as_ref()==Some(&c) && f.artwork.is_none()).await;
        image(&loader,true,Some([25,230,40,255])).await;
        track("d","Paused",true);
        let d=expected();
        wait_for(&feed,"D metadata",|f| f.artwork_track.as_ref()==Some(&d)).await;
        image(&loader,true,None).await;
        image(&loader,false,Some([30,40,240,255])).await;
        // Rapid observed switches, without waiting for the current art worker.
        for id in ["a","b","d"] {
            track(id,"Paused",true);
            let id=expected();
            wait_for(&feed,"rapid metadata",|f| f.artwork_track.as_ref()==Some(&id)).await;
        }
        image(&loader,true,Some([25,230,40,255])).await;
        image(&loader,false,Some([30,40,240,255])).await;
        glib::timeout_future(Duration::from_millis(200)).await;
        image(&loader,true,Some([25,230,40,255])).await;
        fixture.name("ReleaseName",name).await;
        wait_for(&feed,"player closed",|f| f.active.is_none() && !f.sleeve_visible()).await;
        image(&loader,true,None).await;
        let revision=loader.previous_snapshot().0;
        fixture.name("RequestName",name).await;
        wait_for(&feed,"player reopened",|f| f.active.as_deref()==Some(name) && f.sleeve_visible() && f.artwork_track.as_ref()==Some(&d)).await;
        image(&loader,true,None).await;
        assert_eq!(loader.previous_snapshot().0,revision,"reopen restored previous-session art");
        track("b","Paused",true);
        wait_for(&feed,"new history after reopen",|f| f.artwork_track.as_ref()==Some(&b)).await;
        image(&loader,true,Some([30,40,240,255])).await;
        feed.borrow_mut().select(Some(name.into()));
        loader.sync(&feed.borrow());
        image(&loader,true,None).await;
        monitor.stop(); loader.stop();
        assert!(fixture.commands.borrow().is_empty() && fixture.volume_writes.borrow().is_empty());
    })).unwrap();
    for name in ["b", "b-refresh", "d"] {
        std::fs::remove_file(directory.join(format!("{name}.png"))).unwrap();
    }
    std::fs::remove_dir(directory).unwrap();
}
