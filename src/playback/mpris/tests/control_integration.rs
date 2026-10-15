//! Strictly private bus. No system player is discovered or controlled.
use super::super::*;
use super::integration::{Fixture, wait_for};
use crate::playback::transport::Command;

#[test]
#[ignore = "requires private dbus-run-session; TALIS_MPRIS_TEST_BUS=1"]
fn private_bus_transport_commands_races_capabilities_and_timeout() {
    assert_eq!(std::env::var("TALIS_MPRIS_TEST_BUS").as_deref(), Ok("1"));
    let context = glib::MainContext::default();
    context
        .with_thread_default(|| {
            context.block_on(async {
                let name = "org.mpris.MediaPlayer2.talis_transport_fixture";
                let fixture = Fixture::new(name, "Paused", 20_000_000, 100_000_000).await;
                let feed = Rc::new(RefCell::new(PlaybackFeed::default()));
                feed.borrow_mut().set_enabled(true);
                feed.borrow_mut().select(Some(name.into()));
                let monitor = Monitor::start(feed.clone(), || {});
                wait_for(&feed, "transport ready", |f| {
                    f.command_target(Command::PlayPause, Instant::now())
                        .is_some()
                })
                .await;
                assert!(
                    fixture.commands.borrow().is_empty(),
                    "monitor sent a write without a click"
                );
                monitor.command(Command::PlayPause).unwrap();
                assert!(feed.borrow().control_pending);
                assert!(
                    monitor.command(Command::Next).is_err(),
                    "duplicate command was queued"
                );
                wait_for(&feed, "actual play acknowledgement", |f| {
                    !f.control_pending
                        && f.transport
                            .as_ref()
                            .is_some_and(|t| t.status == Status::Playing)
                })
                .await;
                assert_eq!(*fixture.commands.borrow(), ["Play"]);
                monitor.command(Command::PlayPause).unwrap();
                wait_for(&feed, "actual pause acknowledgement", |f| {
                    !f.control_pending
                        && f.transport
                            .as_ref()
                            .is_some_and(|t| t.status == Status::Paused)
                })
                .await;
                for (command, method) in [(Command::Next, "Next"), (Command::Previous, "Previous")]
                {
                    monitor.command(command).unwrap();
                    wait_for(&feed, method, |f| {
                        !f.control_pending
                            && f.snapshot.as_ref().is_some_and(|s| s.position.is_zero())
                            && f.transport.is_some()
                    })
                    .await;
                }
                assert_eq!(
                    *fixture.commands.borrow(),
                    ["Play", "Pause", "Next", "Previous"]
                );

                fixture
                    .data
                    .borrow_mut()
                    .insert("CanGoNext".into(), false.to_variant());
                fixture.changed();
                wait_for(&feed, "unsupported next disabled", |f| {
                    f.transport.as_ref().is_some_and(|t| !t.capabilities.next)
                })
                .await;
                assert!(monitor.command(Command::Next).is_err());
                fixture.data.borrow_mut().remove("CanControl");
                fixture.changed();
                wait_for(&feed, "missing CanControl disabled", |f| {
                    f.transport
                        .as_ref()
                        .is_some_and(|t| !t.capabilities.control)
                })
                .await;
                assert!(monitor.command(Command::PlayPause).is_err());
                fixture.track("stream", "Paused", 0, 0);
                fixture.data.borrow_mut().remove("Position");
                fixture.changed();
                wait_for(&feed, "controls without duration or position", |f| {
                    f.snapshot.is_none()
                        && f.command_target(Command::PlayPause, Instant::now())
                            .is_some()
                })
                .await;
                monitor.command(Command::PlayPause).unwrap();
                wait_for(&feed, "stream plays without timing", |f| {
                    !f.control_pending
                        && f.snapshot.is_none()
                        && f.transport
                            .as_ref()
                            .is_some_and(|t| t.status == Status::Playing)
                })
                .await;

                let (old, revision) = {
                    let f = feed.borrow();
                    (f.transport.clone().unwrap(), f.selection_revision)
                };
                let second_name = "org.mpris.MediaPlayer2.talis_transport_second";
                let second = Fixture::new(second_name, "Playing", 0, 100_000_000).await;
                feed.borrow_mut().select(None);
                wait_for(&feed, "automatic selection retains playing stream", |f| {
                    f.snapshot.is_none()
                        && f.command_target(Command::Next, Instant::now())
                            .is_some_and(|t| t.player == name)
                })
                .await;
                fixture.track("stream", "Paused", 0, 0);
                fixture.data.borrow_mut().remove("Position");
                fixture.changed();
                wait_for(&feed, "paused stream yields to playing source", |f| {
                    f.command_target(Command::Next, Instant::now())
                        .is_some_and(|t| t.player == second_name)
                })
                .await;
                feed.borrow_mut().select(Some(second_name.into()));
                wait_for(&feed, "explicit second source", |f| {
                    f.command_target(Command::Next, Instant::now())
                        .is_some_and(|t| t.player == second_name)
                })
                .await;
                assert!(monitor.command_for(Command::Next, &old, revision).is_err());
                assert!(second.commands.borrow().is_empty());
                feed.borrow_mut().select(Some(name.into()));
                wait_for(&feed, "original source ready", |f| {
                    f.command_target(Command::Next, Instant::now())
                        .is_some_and(|t| t.player == name)
                })
                .await;
                // Selection changes before dispatch must cancel the request entirely.
                let count = fixture.commands.borrow().len();
                monitor.command(Command::Next).unwrap();
                feed.borrow_mut().select(Some(second_name.into()));
                wait_for(&feed, "preflight cancellation", |f| {
                    !f.control_pending
                        && f.transport
                            .as_ref()
                            .is_some_and(|t| t.player == second_name)
                })
                .await;
                assert_eq!(fixture.commands.borrow().len(), count);
                assert!(second.commands.borrow().is_empty());

                // A reply from a command already delivered to the original source
                // cannot set status or feedback on a newly selected player.
                feed.borrow_mut().select(Some(name.into()));
                wait_for(&feed, "ready for late reply", |f| {
                    f.command_target(Command::Next, Instant::now()).is_some()
                })
                .await;
                fixture.command_delay.set(700);
                let count = fixture.commands.borrow().len();
                monitor.command(Command::Next).unwrap();
                let deadline = Instant::now() + Duration::from_secs(4);
                while fixture.commands.borrow().len() == count && Instant::now() < deadline {
                    glib::timeout_future(Duration::from_millis(10)).await;
                }
                assert_eq!(fixture.commands.borrow().len(), count + 1);
                feed.borrow_mut().select(Some(second_name.into()));
                wait_for(&feed, "late reply stays with old player", |f| {
                    !f.control_pending
                        && f.transport
                            .as_ref()
                            .is_some_and(|t| t.player == second_name)
                })
                .await;
                assert!(feed.borrow().control_notice.is_none());
                assert!(second.commands.borrow().is_empty());

                feed.borrow_mut().select(Some(name.into()));
                wait_for(&feed, "ready before timeout", |f| {
                    f.command_target(Command::Next, Instant::now()).is_some()
                })
                .await;
                fixture.command_delay.set(1800);
                let count = fixture.commands.borrow().len();
                monitor.command(Command::Next).unwrap();
                wait_for(&feed, "bounded command timeout", |f| {
                    !f.control_pending
                        && f.control_notice
                            .as_ref()
                            .is_some_and(|n| n.contains("timed out"))
                })
                .await;
                glib::timeout_future(Duration::from_millis(800)).await;
                assert_eq!(
                    fixture.commands.borrow().len(),
                    count + 1,
                    "timed-out skip was retried"
                );
                wait_for(&feed, "recover after timeout", |f| {
                    f.command_target(Command::Next, Instant::now()).is_some()
                })
                .await;
                fixture.reject.set(true);
                monitor.command(Command::Next).unwrap();
                wait_for(&feed, "method error does not stick", |f| {
                    !f.control_pending
                        && f.control_notice
                            .as_ref()
                            .is_some_and(|n| n.contains("Fixture rejected"))
                })
                .await;
                fixture.reject.set(false);
                wait_for(&feed, "rejection recovered", |f| {
                    f.command_target(Command::Next, Instant::now()).is_some()
                })
                .await;

                let (old, revision) = {
                    let f = feed.borrow();
                    (f.transport.clone().unwrap(), f.selection_revision)
                };
                fixture.command_delay.set(700);
                let count = fixture.commands.borrow().len();
                monitor.command(Command::Next).unwrap();
                let deadline = Instant::now() + Duration::from_secs(4);
                while fixture.commands.borrow().len() == count && Instant::now() < deadline {
                    glib::timeout_future(Duration::from_millis(10)).await;
                }
                assert_eq!(fixture.commands.borrow().len(), count + 1);
                fixture.name("ReleaseName", name).await;
                let replacement = Fixture::new(name, "Paused", 0, 100_000_000).await;
                wait_for(&feed, "replacement unique owner", |f| {
                    !f.control_pending
                        && f.transport
                            .as_ref()
                            .is_some_and(|t| t.owner != old.owner && t.player == name)
                })
                .await;
                assert!(feed.borrow().control_notice.is_none());
                assert!(replacement.commands.borrow().is_empty());
                let count = fixture.commands.borrow().len();
                assert!(monitor.command_for(Command::Next, &old, revision).is_err());
                monitor.command(Command::Next).unwrap();
                wait_for(&feed, "new owner command", |f| {
                    !f.control_pending && f.command_target(Command::Next, Instant::now()).is_some()
                })
                .await;
                assert_eq!(
                    fixture.commands.borrow().len(),
                    count,
                    "old process got replacement's command"
                );
                assert_eq!(*replacement.commands.borrow(), ["Next"]);
                monitor.command(Command::Next).unwrap();
                feed.borrow_mut().set_enabled(false);
                glib::timeout_future(Duration::from_millis(400)).await;
                assert!(!feed.borrow().control_pending);
                assert_eq!(
                    *replacement.commands.borrow(),
                    ["Next"],
                    "disabled tracking still sent a command"
                );
                monitor.stop();
                assert!(monitor.command(Command::PlayPause).is_err());
            })
        })
        .unwrap();
}
