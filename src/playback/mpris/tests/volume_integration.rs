//! Fake-player tests only. Never use the user's session bus.
use super::super::*;
use super::integration::{Fixture, wait_for};
use std::cell::Cell;

fn at(f: &PlaybackFeed, value: f64) -> bool {
    f.volume_target(Instant::now())
        .is_some_and(|t| (t.value - value).abs() < 1e-8)
}
#[test]
#[ignore = "requires private dbus-run-session; TALIS_MPRIS_TEST_BUS=1"]
fn private_bus_volume_ack_before_apply_preserves_hover_availability() {
    assert_eq!(std::env::var("TALIS_MPRIS_TEST_BUS").as_deref(), Ok("1"));
    let context = glib::MainContext::default();
    context
        .with_thread_default(|| {
            context.block_on(async {
                let name = "org.mpris.MediaPlayer2.talis_delayed_volume";
                let player = Fixture::new(name, "Paused", 0, 100_000_000).await;
                player.volume_changed(0.5);
                player.volume_apply_delay.set(350);
                let feed = Rc::new(RefCell::new(PlaybackFeed::default()));
                feed.borrow_mut().set_enabled(true);
                feed.borrow_mut().select(Some(name.into()));
                let lost_availability = Rc::new(Cell::new(false));
                let monitor = Monitor::start(feed.clone(), {
                    let feed = feed.clone();
                    let lost = lost_availability.clone();
                    move || {
                        if feed.borrow().active.as_deref() == Some(name)
                            && feed.borrow().volume_target(Instant::now()).is_none()
                        {
                            lost.set(true);
                        }
                    }
                });
                wait_for(&feed, "delayed player ready", |f| at(f, 0.5)).await;
                lost_availability.set(false);
                monitor.adjust_volume(0.05).unwrap();
                glib::timeout_future(Duration::from_millis(200)).await;
                assert!(
                    at(&feed.borrow(), 0.5),
                    "early acknowledgement must not disable hover or predict volume"
                );
                // Keep accepting new wheel input while the previous value has
                // not reached the player's property yet, including reversal.
                monitor.adjust_volume(0.05).unwrap();
                monitor.adjust_volume(-0.15).unwrap();
                wait_for(&feed, "delayed reversal confirmed", |f| at(f, 0.45)).await;
                glib::timeout_future(Duration::from_millis(150)).await;
                assert!(!lost_availability.get());
                let writes = player.volume_writes.borrow().clone();
                assert_eq!(writes.len(), 2);
                assert!((writes[0] - 0.55).abs() < 1e-8);
                assert!((writes[1] - 0.45).abs() < 1e-8);
                assert!(monitor.0.borrow().volume.is_none());

                // Also handle late publication without PropertiesChanged.
                player.volume_emit_signal.set(false);
                let reads = player.volume_reads.get();
                monitor.adjust_volume(0.05).unwrap();
                wait_for(&feed, "delayed property readback confirmed", |f| at(f, 0.5)).await;
                glib::timeout_future(Duration::from_millis(100)).await;
                assert!(!lost_availability.get());
                assert!(player.volume_reads.get() - reads <= 6);
                assert_eq!(player.volume_writes.borrow().len(), 3);
                let settled_reads = player.volume_reads.get();
                glib::timeout_future(Duration::from_millis(200)).await;
                assert_eq!(player.volume_reads.get(), settled_reads);

                // A genuine acknowledged no-op is still bounded, never
                // replayed, and must not animate the unconfirmed destination.
                player.volume_noop.set(true);
                let start = Instant::now();
                let reads = player.volume_reads.get();
                monitor.adjust_volume(0.05).unwrap();
                glib::timeout_future(Duration::from_millis(200)).await;
                assert!(at(&feed.borrow(), 0.5));
                wait_for(&feed, "unchanged volume eventually disabled", |f| {
                    f.volume.is_none()
                })
                .await;
                assert!(start.elapsed() >= Duration::from_millis(1200));
                assert!(start.elapsed() < Duration::from_secs(2));
                assert!(player.volume_reads.get() - reads <= 13);
                glib::timeout_future(Duration::from_millis(200)).await;
                assert_eq!(player.volume_writes.borrow().len(), 4);
                assert!(
                    player.data.borrow()["Volume"]
                        .get::<f64>()
                        .is_some_and(|v| (v - 0.5).abs() < 1e-8)
                );
                monitor.stop();
            })
        })
        .unwrap();
}
#[test]
#[ignore = "requires private dbus-run-session; TALIS_MPRIS_TEST_BUS=1"]
fn private_bus_volume_source_and_owner_switch_during_confirmation() {
    assert_eq!(std::env::var("TALIS_MPRIS_TEST_BUS").as_deref(), Ok("1"));
    let context = glib::MainContext::default();
    context
        .with_thread_default(|| {
            context.block_on(async {
                let first_name = "org.mpris.MediaPlayer2.talis_confirm_first";
                let other_name = "org.mpris.MediaPlayer2.talis_confirm_other";
                let first = Fixture::new(first_name, "Paused", 0, 100_000_000).await;
                let other = Fixture::new(other_name, "Paused", 0, 100_000_000).await;
                first.volume_changed(0.5);
                other.volume_changed(0.25);
                first.volume_apply_delay.set(350);
                let feed = Rc::new(RefCell::new(PlaybackFeed::default()));
                feed.borrow_mut().set_enabled(true);
                feed.borrow_mut().select(Some(first_name.into()));
                let monitor = Monitor::start(feed.clone(), || {});
                wait_for(&feed, "first ready", |f| at(f, 0.5)).await;
                monitor.adjust_volume(0.05).unwrap();
                glib::timeout_future(Duration::from_millis(200)).await;
                assert_eq!(first.volume_writes.borrow().len(), 1);
                assert!(monitor.0.borrow().volume.is_some());
                feed.borrow_mut().select(Some(other_name.into()));
                wait_for(&feed, "source changed during readback", |f| at(f, 0.25)).await;
                glib::timeout_future(Duration::from_millis(400)).await;
                assert!(at(&feed.borrow(), 0.25));
                assert!(other.volume_writes.borrow().is_empty());
                assert!(monitor.0.borrow().volume.is_none());

                other.volume_apply_delay.set(350);
                monitor.adjust_volume(0.05).unwrap();
                glib::timeout_future(Duration::from_millis(200)).await;
                assert_eq!(other.volume_writes.borrow().len(), 1);
                assert!(monitor.0.borrow().volume.is_some());
                other.name("ReleaseName", other_name).await;
                let replacement = Fixture::new(other_name, "Paused", 0, 100_000_000).await;
                replacement.volume_changed(0.6);
                wait_for(&feed, "owner changed during readback", |f| at(f, 0.6)).await;
                glib::timeout_future(Duration::from_millis(400)).await;
                assert!(at(&feed.borrow(), 0.6));
                assert!(replacement.volume_writes.borrow().is_empty());
                assert_eq!(other.volume_writes.borrow().len(), 1);
                assert!(monitor.0.borrow().volume.is_none());
                monitor.stop();
            })
        })
        .unwrap();
}
#[test]
#[ignore = "requires private dbus-run-session; TALIS_MPRIS_TEST_BUS=1"]
fn private_bus_volume_coalescing_switches_unsupported_and_timeouts() {
    assert_eq!(std::env::var("TALIS_MPRIS_TEST_BUS").as_deref(), Ok("1"));
    let context = glib::MainContext::default();
    context
        .with_thread_default(|| {
            context.block_on(async {
                let name = "org.mpris.MediaPlayer2.talis_volume_fixture";
                let first = Fixture::new(name, "Paused", 20_000_000, 100_000_000).await;
                first.volume_changed(0.5);
                let f = Rc::new(RefCell::new(PlaybackFeed::default()));
                f.borrow_mut().set_enabled(true);
                f.borrow_mut().select(Some(name.into()));
                let monitor = Monitor::start(f.clone(), || {});
                wait_for(&f, "volume ready", |f| at(f, 0.5)).await;
                assert!(
                    first.volume_writes.borrow().is_empty(),
                    "monitor wrote without input"
                );
                let timing = f.borrow().snapshot.as_ref().unwrap().observed_at;
                let note = f.borrow().notice.clone();
                monitor.adjust_volume(0.05).unwrap();
                wait_for(&f, "increased player volume", |f| at(f, 0.55)).await;
                assert_eq!(
                    f.borrow().snapshot.as_ref().unwrap().observed_at,
                    timing,
                    "volume invalidated arm timing"
                );
                assert_eq!(f.borrow().notice, note);
                assert!(f.borrow().control_notice.is_none());
                assert!(!f.borrow().control_pending);
                first.volume_changed(0.4);
                wait_for(&f, "external volume synchronized", |f| at(f, 0.4)).await;
                first.delay.set(350);
                monitor.0.borrow_mut().entries.get_mut(name).unwrap().due = Instant::now();
                Monitor::tick(&monitor.0);
                glib::timeout_future(Duration::from_millis(60)).await;
                first.volume_changed(0.4 + 0.05);
                wait_for(&f, "new volume signal beats stale GetAll", |f| at(f, 0.45)).await;
                glib::timeout_future(Duration::from_millis(400)).await;
                assert!(
                    at(&f.borrow(), 0.45),
                    "delayed GetAll restored stale volume"
                );
                assert!(
                    f.borrow().snapshot.is_some(),
                    "volume revision discarded timing reply"
                );
                first.volume_changed(0.4);
                wait_for(&f, "burst baseline", |f| at(f, 0.4)).await;
                let count = first.volume_writes.borrow().len();
                for _ in 0..8 {
                    monitor.adjust_volume(0.05).unwrap();
                }
                wait_for(&f, "wheel burst accumulated", |f| at(f, 0.8)).await;
                glib::timeout_future(Duration::from_millis(150)).await;
                assert_eq!(
                    first.volume_writes.borrow().len(),
                    count + 1,
                    "burst was not coalesced"
                );
                monitor.adjust_volume(1.0).unwrap();
                wait_for(&f, "maximum", |f| at(f, 1.0)).await;
                monitor.adjust_volume(-1.0).unwrap();
                wait_for(&f, "muted", |f| at(f, 0.0)).await;
                let count = first.volume_writes.borrow().len();
                monitor.adjust_volume(-0.05).unwrap();
                glib::timeout_future(Duration::from_millis(150)).await;
                assert_eq!(
                    first.volume_writes.borrow().len(),
                    count,
                    "bound generated extra write"
                );

                first.volume_changed(0.5);
                wait_for(&f, "reset", |f| at(f, 0.5)).await;
                first.volume_delay.set(350);
                monitor.adjust_volume(0.05).unwrap();
                glib::timeout_future(Duration::from_millis(180)).await;
                monitor.adjust_volume(0.05).unwrap();
                monitor.adjust_volume(-0.15).unwrap();
                wait_for(&f, "in-flight reversal kept latest intent", |f| at(f, 0.45)).await;
                glib::timeout_future(Duration::from_millis(150)).await;
                assert!(monitor.0.borrow().volume.is_none());

                let second_name = "org.mpris.MediaPlayer2.talis_volume_other";
                let second = Fixture::new(second_name, "Playing", 0, 0).await;
                second.volume_changed(0.25);
                let count = first.volume_writes.borrow().len();
                monitor.adjust_volume(0.05).unwrap();
                f.borrow_mut().select(Some(second_name.into()));
                wait_for(&f, "selection switched", |f| at(f, 0.25)).await;
                glib::timeout_future(Duration::from_millis(150)).await;
                assert_eq!(
                    first.volume_writes.borrow().len(),
                    count,
                    "queued write survived selection switch"
                );
                assert!(
                    second.volume_writes.borrow().is_empty(),
                    "old intent retargeted another player"
                );
                second.data.borrow_mut().remove("Position");
                second.changed();
                wait_for(&f, "volume without timing", |f| {
                    f.snapshot.is_none() && at(f, 0.25)
                })
                .await;
                monitor.adjust_volume(0.05).unwrap();
                wait_for(&f, "stream volume", |f| at(f, 0.3)).await;

                second.volume_reject.set(true);
                monitor.adjust_volume(0.05).unwrap();
                wait_for(&f, "read-only volume disabled", |f| f.volume.is_none()).await;
                assert!(monitor.adjust_volume(0.05).is_err());
                assert!(f.borrow().control_notice.is_none());
                assert!(!f.borrow().control_pending);
                let count = second.volume_writes.borrow().len();
                glib::timeout_future(Duration::from_millis(200)).await;
                assert_eq!(
                    second.volume_writes.borrow().len(),
                    count,
                    "rejected write retried itself"
                );

                // New owner resets unsupported/cooldown state, without replaying input.
                second.name("ReleaseName", second_name).await;
                let replacement = Fixture::new(second_name, "Paused", 0, 100_000_000).await;
                replacement.volume_changed(0.6);
                wait_for(&f, "replacement owner", |f| at(f, 0.6)).await;
                assert!(replacement.volume_writes.borrow().is_empty());
                replacement.volume_noop.set(true);
                monitor.adjust_volume(0.05).unwrap();
                wait_for(&f, "acknowledged no-op disabled", |f| f.volume.is_none()).await;
                assert_eq!(replacement.data.borrow()["Volume"].get::<f64>(), Some(0.6));

                replacement.name("ReleaseName", second_name).await;
                let timeout = Fixture::new(second_name, "Paused", 0, 100_000_000).await;
                timeout.volume_changed(0.4);
                wait_for(&f, "timeout owner ready", |f| at(f, 0.4)).await;
                timeout.volume_delay.set(1500);
                monitor.adjust_volume(0.05).unwrap();
                wait_for(&f, "timeout stopped volume stream", |f| f.volume.is_none()).await;
                glib::timeout_future(Duration::from_millis(500)).await;
                assert_eq!(
                    timeout.volume_writes.borrow().len(),
                    1,
                    "timed-out intent retried"
                );
                assert!(f.borrow().control_notice.is_none());
                let race_name = "org.mpris.MediaPlayer2.talis_volume_race";
                let old_owner = Fixture::new(race_name, "Paused", 0, 100_000_000).await;
                old_owner.volume_changed(0.7);
                f.borrow_mut().select(Some(race_name.into()));
                wait_for(&f, "owner race baseline", |f| at(f, 0.7)).await;
                monitor.adjust_volume(0.05).unwrap();
                old_owner.name("ReleaseName", race_name).await;
                let new_owner = Fixture::new(race_name, "Paused", 0, 100_000_000).await;
                new_owner.volume_changed(0.2);
                wait_for(&f, "owner replaced during queued wheel input", |f| {
                    at(f, 0.2)
                })
                .await;
                glib::timeout_future(Duration::from_millis(150)).await;
                assert!(old_owner.volume_writes.borrow().is_empty());
                assert!(
                    new_owner.volume_writes.borrow().is_empty(),
                    "old wheel intent reached replacement owner"
                );
                f.borrow_mut().set_enabled(false);
                monitor.stop();
                assert!(monitor.0.borrow().volume.is_none());
                assert!(
                    first.commands.borrow().is_empty()
                        && second.commands.borrow().is_empty()
                        && replacement.commands.borrow().is_empty()
                        && timeout.commands.borrow().is_empty()
                );
            })
        })
        .unwrap();
}
