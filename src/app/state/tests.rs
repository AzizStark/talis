use super::*;
#[test]
fn disabled_arm_holds_through_timing_range_visibility_and_connection_changes() {
    use crate::playback::Status;
    let now = Instant::now();
    let mut s = paused_music_state(0.7, 0.1, now);
    s.config.render.animate_arm = false;
    for (i, (seconds, duration, status)) in [
        (90, 100, Status::Playing),
        (0, 240, Status::Playing),
        (180, 200, Status::Paused),
        (10, 0, Status::Playing),
        (0, 100, Status::Stopped),
    ]
    .into_iter()
    .enumerate()
    {
        let at = now + Duration::from_secs(i as u64);
        {
            let mut feed = s.playback.borrow_mut();
            feed.status = status;
            let clock = feed.snapshot.as_mut().unwrap();
            clock.position = Duration::from_secs(seconds);
            clock.duration = Duration::from_secs(duration);
            clock.playing = status == Status::Playing;
            clock.observed_at = at;
        }
        tick(&mut s, at);
        assert_eq!(s.arm.position, 0.7);
        assert!(!s.arm_motion.has_target());
        assert_eq!(s.arm_cache_target(at), None);
        assert!(!s.needs_redraw_at(at));
    }
    s.set_arm_range(0.2, 0.3, now);
    assert_eq!(
        s.arm.position, 0.7,
        "range edits must not clamp a disabled arm"
    );
    s.playback.borrow_mut().invalidate("Disconnected");
    s.resume_at(now + Duration::from_secs(600));
    s.advance_arm(Duration::MAX);
    assert_eq!(s.arm.position, 0.7);
    s.playback.borrow_mut().set_enabled(false);
    s.set_arm_range(0.4, 0.4, now);
    assert_eq!(s.arm.position, 0.7, "manual range changes must also hold");
}
#[test]
fn disabling_inflight_arm_cancels_velocity_and_reenable_catches_up_smoothly() {
    let now = Instant::now();
    let mut s = paused_music_state(0.2, 0.9, now);
    tick(&mut s, now);
    tick(&mut s, now + Duration::from_millis(100));
    let pose = s.arm.position;
    assert!(pose > 0.2 && s.arm_motion.moving());
    s.config.render.animate_arm = false;
    // A forced frame must honor the flag even before a timing notification.
    s.advance_arm(Duration::MAX);
    assert_eq!(s.arm.position, pose);
    assert!(!s.arm_motion.has_target());
    assert!(!s.needs_redraw_at(now));
    s.benchmark = true;
    assert!(s.needs_redraw_at(now));
    tick(&mut s, now + Duration::from_secs(60));
    assert_eq!(s.arm.position, pose);
    s.benchmark = false;
    let resume = now + Duration::from_secs(120);
    s.playback
        .borrow_mut()
        .snapshot
        .as_mut()
        .unwrap()
        .observed_at = resume;
    s.config.render.animate_arm = true;
    s.sync_playback(resume);
    assert_eq!(s.previous_frame, resume);
    tick(&mut s, resume);
    assert_eq!(
        s.arm.position, pose,
        "reenable cannot teleport or consume the idle gap"
    );
    tick(&mut s, resume + Duration::from_millis(16));
    assert!(s.arm.position > pose && s.arm.position < pose + 0.01);
    settle(&mut s, resume + Duration::from_millis(16));
    assert!((s.arm.position - 0.9).abs() < 0.0001);
}
#[test]
fn disabled_arm_never_advances_demo_on_forced_frames_but_enabled_demo_does() {
    let now = Instant::now();
    let mut s = paused_music_state(0.3, 0.9, now);
    s.playback.borrow_mut().set_enabled(false);
    s.arm.play(true);
    s.config.render.animate_arm = false;
    s.sync_playback(now);
    assert!(!s.needs_redraw_at(now));
    for elapsed in [Duration::ZERO, Duration::from_secs(1), Duration::MAX] {
        s.advance_arm(elapsed);
        assert_eq!(s.arm.position, 0.3);
    }
    s.benchmark = true;
    assert!(s.needs_redraw_at(now));
    s.advance_arm(Duration::from_secs(1));
    assert_eq!(s.arm.position, 0.3);
    s.config.render.animate_arm = true;
    s.advance_arm(Duration::from_millis(100));
    assert!(s.arm.position > 0.3);
    s.mode = PreviewMode::Beauty;
    let pose = s.arm.position;
    s.advance_arm(Duration::from_secs(1));
    assert_eq!(s.arm.position, pose);
}
#[test]
fn disabled_arm_keeps_artwork_and_dust_rotation_independent() {
    use crate::playback::{ArtworkSource, Status};
    for (cover, dust) in [(true, false), (false, true), (true, true)] {
        let now = Instant::now();
        let mut s = paused_music_state(0.4, 0.9, now);
        s.config.render.animate_arm = false;
        s.artwork_ready = cover;
        s.vinyl_ready = dust;
        {
            let mut feed = s.playback.borrow_mut();
            feed.artwork = Some(ArtworkSource {
                track: "one".into(),
                url: "file:///cover.png".into(),
            });
            feed.artwork_playback = Some((Status::Playing, now));
        }
        assert!(s.needs_redraw_at(now));
        s.sync_playback(now);
        s.sync_artwork(now);
        let next = now + Duration::from_millis(33);
        s.advance_arm(Duration::from_millis(33));
        s.artwork_rotation.advance(next);
        assert_eq!(s.arm.position, 0.4);
        assert!(s.artwork_rotation.angle() > 0.0);
        assert_eq!(s.arm_cache_target(next), None);
        s.playback.borrow_mut().artwork_playback = Some((Status::Paused, next));
        s.sync_artwork(next);
        let angle = s.artwork_rotation.angle();
        s.artwork_rotation.advance(next + Duration::from_secs(1));
        assert_eq!(s.artwork_rotation.angle(), angle);
        assert!(!s.needs_redraw_at(next));
    }
}
#[test]
fn hidden_seek_resumes_from_visible_pose_without_charging_hidden_time() {
    let now = Instant::now();
    let mut s = paused_music_state(0.2, 0.2, now);
    s.sync_playback(now);
    let resume = now + Duration::from_secs(600);
    {
        let feed = s.playback.clone();
        let mut feed = feed.borrow_mut();
        let snapshot = feed.snapshot.as_mut().unwrap();
        snapshot.duration = Duration::from_secs(200);
        snapshot.position = Duration::from_secs(180);
        snapshot.observed_at = resume;
    }
    s.sync_playback(resume);
    assert_eq!(s.arm.position, 0.2);
    s.resume_at(resume);
    tick(&mut s, resume);
    assert_eq!(s.arm.position, 0.2, "first resumed frame teleported");
    tick(&mut s, resume + Duration::from_millis(16));
    assert!(s.arm.position > 0.2 && s.arm.position < 0.25);
    settle(&mut s, resume + Duration::from_millis(16));
    assert!((s.arm.position - 0.9).abs() < 0.0001);
    s.playback.borrow_mut().snapshot = None;
    s.resume_at(resume + Duration::from_secs(700));
    assert!(!s.arm_motion.has_target());
    assert!(s.arm.position.is_finite());
}
#[test]
fn vinyl_without_artwork_rotates_only_with_fresh_playback_and_enabled_surface() {
    use crate::playback::Status;
    let now = Instant::now();
    let mut s = paused_music_state(0.5, 0.5, now);
    s.vinyl_ready = true;
    s.playback.borrow_mut().artwork_playback = Some((Status::Playing, now));
    assert!(s.needs_redraw_at(now));
    s.sync_artwork(now);
    s.artwork_rotation.advance(now + Duration::from_millis(50));
    assert!(s.artwork_rotation.angle() > 0.0);
    let angle = s.artwork_rotation.angle();
    s.playback.borrow_mut().artwork_playback =
        Some((Status::Paused, now + Duration::from_millis(50)));
    s.sync_artwork(now + Duration::from_millis(50));
    s.artwork_rotation.advance(now + Duration::from_secs(1));
    assert_eq!(s.artwork_rotation.angle(), angle);
    assert!(!s.needs_redraw_at(now + Duration::from_secs(1)));
    s.playback.borrow_mut().artwork_playback = Some((Status::Playing, now));
    assert!(!s.needs_redraw_at(now + Duration::from_secs(7)));
    s.config.artwork.vinyl_rotation = false;
    assert!(!s.needs_redraw_at(now));
    s.config.artwork.vinyl_rotation = true;
    s.config.artwork.vinyl_opacity = 0.0;
    assert!(
        !s.needs_redraw_at(now),
        "hidden marks started a rotation timer"
    );
    s.sync_artwork(now);
    let frozen = s.artwork_rotation.angle();
    s.artwork_rotation.advance(now + Duration::from_millis(50));
    assert_eq!(s.artwork_rotation.angle(), frozen);
    s.config.artwork.vinyl_opacity = 0.5;
    assert!(s.needs_redraw_at(now));
    s.config.artwork.vinyl_brightness = 0.0;
    assert!(
        s.needs_redraw_at(now),
        "zero brightness leaves visible dark marks"
    );
    s.artwork_ready = true;
    s.playback.borrow_mut().artwork = Some(crate::playback::ArtworkSource {
        track: "one".into(),
        url: "file:///cover.png".into(),
    });
    s.config.artwork.vinyl_opacity = 0.0;
    assert!(
        s.needs_redraw_at(now),
        "hidden marks must not stop visible artwork"
    );
    s.artwork_ready = false;
    s.config.artwork.vinyl_opacity = 1.0;
    s.config.artwork.rotation_rpm = 0.0;
    assert!(!s.needs_redraw_at(now));
    s.config.artwork.rotation_rpm = 45.0;
    s.vinyl_ready = false;
    assert!(!s.needs_redraw_at(now));
}
#[test]
fn default_center_rotates_without_cover_timing_or_dust_and_obeys_playback() {
    use crate::playback::Status;
    let now = Instant::now();
    let mut state = paused_music_state(0.5, 0.5, now);
    state.artwork_ready = true;
    state.vinyl_ready = false;
    state.config.artwork.vinyl_rotation = false;
    {
        let mut feed = state.playback.borrow_mut();
        feed.active = Some("org.mpris.MediaPlayer2.test".into());
        feed.snapshot = None;
        feed.artwork = None;
        feed.artwork_playback = Some((Status::Playing, now));
    }
    state.sync_artwork(now);
    assert!(state.needs_redraw_at(now));
    state
        .artwork_rotation
        .advance(now + Duration::from_millis(50));
    let phase = state.artwork_rotation.angle();
    assert!(phase > 0.0);
    for status in [Status::Paused, Status::Stopped] {
        state.playback.borrow_mut().artwork_playback =
            Some((status, now + Duration::from_millis(50)));
        state.sync_artwork(now + Duration::from_millis(50));
        assert!(!state.needs_redraw_at(now + Duration::from_secs(1)));
        state.artwork_rotation.advance(now + Duration::from_secs(1));
        assert_eq!(state.artwork_rotation.angle(), phase);
    }
    state.playback.borrow_mut().artwork_playback = Some((Status::Playing, now));
    assert!(!state.needs_redraw_at(now + Duration::from_secs(7)));
    state.config.artwork.enabled = false;
    assert!(!state.needs_redraw_at(now));
    state.config.artwork.enabled = true;
    state.playback.borrow_mut().invalidate("No player");
    assert!(!state.needs_redraw_at(now));
}
#[test]
fn center_rotation_is_independent_of_timing_and_stops_for_pause_stale_or_unavailable_layer() {
    use crate::playback::{ArtworkSource, Status};
    let now = Instant::now();
    let mut s = paused_music_state(0.5, 0.5, now);
    s.artwork_ready = true;
    {
        let mut f = s.playback.borrow_mut();
        f.snapshot = None;
        f.artwork = Some(ArtworkSource {
            track: "one".into(),
            url: "file:///cover.png".into(),
        });
        f.artwork_playback = Some((Status::Playing, now));
    }
    assert!(
        s.needs_redraw_at(now),
        "unknown duration/position must not prevent rotating a playing cover"
    );
    s.sync_artwork(now);
    s.artwork_rotation.advance(now + Duration::from_millis(30));
    let angle = s.artwork_rotation.angle();
    assert!(angle > 0.0);
    // Playback position/range updates cannot reset the independent rotation phase.
    s.sync_playback(now + Duration::from_millis(40));
    assert_eq!(s.artwork_rotation.angle(), angle);
    s.playback.borrow_mut().artwork_playback =
        Some((Status::Paused, now + Duration::from_millis(50)));
    s.sync_artwork(now + Duration::from_millis(50));
    s.artwork_rotation.advance(now + Duration::from_secs(2));
    assert_eq!(s.artwork_rotation.angle(), angle);
    assert!(!s.needs_redraw_at(now + Duration::from_secs(2)));
    s.playback.borrow_mut().artwork_playback = Some((Status::Playing, now));
    assert!(
        !s.needs_redraw_at(now + Duration::from_secs(7)),
        "stale status must not rotate forever"
    );
    s.artwork_ready = false;
    assert!(!s.needs_redraw_at(now));
    s.artwork_ready = true;
    s.config.artwork.enabled = false;
    assert!(!s.needs_redraw_at(now));
    s.config.artwork.enabled = true;
    s.config.render.rotate_artwork = false;
    assert!(!s.needs_redraw_at(now));
    s.config.render.rotate_artwork = true;
    s.config.artwork.rotation_rpm = 0.0;
    assert!(
        !s.needs_redraw_at(now),
        "zero speed must not request rotation frames"
    );
    s.sync_artwork(now);
    let frozen = s.artwork_rotation.angle();
    s.artwork_rotation.advance(now + Duration::from_secs(1));
    assert_eq!(s.artwork_rotation.angle(), frozen);
    s.config.artwork.rotation_rpm = 45.0;
    assert!(s.needs_redraw_at(now));
    s.sync_artwork(now + Duration::from_secs(1));
    assert_eq!(s.artwork_rotation.angle(), frozen);
    s.playback.borrow_mut().set_enabled(false);
    assert!(!s.needs_redraw_at(now));
}
#[test]
fn static_beauty_and_paused_arm_have_no_animation_timer() {
    let now = Instant::now();
    let mut s = AppState {
        config: Config::default(),
        previous_frame: now,
        mode: PreviewMode::Beauty,
        benchmark: false,
        gpu_profile: false,
        shell_surface: false,
        arm: ArmState::default(),
        arm_motion: ArmMotion::new(0.0, 0.6),
        playback: Rc::new(RefCell::new(PlaybackFeed::default())),
        artwork_rotation: Default::default(),
        artwork_ready: false,
        vinyl_ready: false,
    };
    assert!(!s.needs_redraw());
    s.gpu_profile = true;
    assert!(!s.needs_redraw(), "profiling must not wake static scenes");
    s.benchmark = true;
    assert!(s.needs_redraw());
    s.benchmark = false;
    s.mode = PreviewMode::Arm;
    assert!(!s.needs_redraw());
    s.arm.play(true);
    assert!(s.needs_redraw());
    s.arm.play(false);
    assert!(!s.needs_redraw());
}
#[test]
fn arm_cache_reuse_requires_fresh_advancing_playback_and_not_diagnostics() {
    use crate::playback::Status;
    let now = Instant::now();
    let mut s = paused_music_state(0.5, 0.5, now);
    assert_eq!(s.arm_cache_target(now), None);
    s.playback.borrow_mut().status = Status::Playing;
    s.playback.borrow_mut().snapshot.as_mut().unwrap().playing = true;
    assert_eq!(s.arm_cache_target(now), Some(0.5));
    s.gpu_profile = true;
    assert_eq!(s.arm_cache_target(now), Some(0.5));
    assert_eq!(s.arm_cache_target(now + Duration::from_secs(7)), None);
    s.benchmark = true;
    assert_eq!(s.arm_cache_target(now), None);
    s.benchmark = false;
    s.mode = PreviewMode::Beauty;
    assert_eq!(s.arm_cache_target(now), None);
    s.mode = PreviewMode::Arm;
    s.playback.borrow_mut().status = Status::Stopped;
    assert_eq!(s.arm_cache_target(now), None);
    s.playback.borrow_mut().set_enabled(false);
    assert_eq!(s.arm_cache_target(now), None);
}
fn paused_music_state(position: f32, progress: f32, now: Instant) -> AppState {
    use crate::playback::{PlaybackSnapshot, Status};
    let mut feed = PlaybackFeed::default();
    feed.set_enabled(true);
    feed.status = Status::Paused;
    let mut clock = PlaybackSnapshot::demo(now);
    clock.playing = false;
    clock.duration = Duration::from_secs(100);
    clock.position = Duration::from_secs_f64(f64::from(progress) * 100.0);
    feed.snapshot = Some(clock);
    let mut arm = ArmState::default();
    arm.scrub(position);
    AppState {
        config: Config::default(),
        previous_frame: now,
        mode: PreviewMode::Arm,
        benchmark: false,
        gpu_profile: false,
        shell_surface: false,
        arm,
        arm_motion: ArmMotion::new(position, 0.6),
        playback: Rc::new(RefCell::new(feed)),
        artwork_rotation: Default::default(),
        artwork_ready: false,
        vinyl_ready: false,
    }
}
fn tick(s: &mut AppState, now: Instant) {
    s.sync_playback(now);
    let delta = now.saturating_duration_since(s.previous_frame);
    s.advance_arm(delta);
    s.previous_frame = now;
}
fn settle(s: &mut AppState, mut now: Instant) -> Instant {
    for _ in 0..180 {
        now += Duration::from_secs_f64(1.0 / 60.0);
        tick(s, now);
        if !s.needs_redraw_at(now) {
            break;
        }
    }
    assert!(!s.needs_redraw_at(now), "motion must become idle");
    now
}
#[test]
fn paused_seek_and_duration_change_start_smoothly_then_become_idle() {
    let now = Instant::now();
    let mut s = paused_music_state(0.2, 0.5, now);
    s.previous_frame = now - Duration::from_secs(60);
    s.arm.set_range(0.2, 0.8);
    s.arm.play(true);
    s.sync_playback(now);
    assert_eq!(s.arm.position, 0.2, "sync must not teleport");
    assert_eq!(
        s.previous_frame, now,
        "new motion must not consume the idle gap"
    );
    assert!(!s.arm.playing);
    assert!(s.needs_redraw_at(now), "paused seek still needs animation");
    tick(&mut s, now + Duration::from_millis(33));
    assert!(s.arm.position > 0.2 && s.arm.position < 0.3);
    let now = settle(&mut s, now + Duration::from_millis(33));
    assert!((s.arm.position - 0.5).abs() < 0.00001);
    s.playback.borrow_mut().snapshot.as_mut().unwrap().duration = Duration::from_secs(50);
    s.sync_playback(now);
    assert!((s.arm.position - 0.5).abs() < 0.00001);
    let now = settle(&mut s, now);
    assert!((s.arm.position - 0.8).abs() < 0.00001);
    assert!(!s.needs_redraw_at(now));
    tick(&mut s, now + Duration::from_secs(1));
    assert!(
        (s.arm.position - 0.8).abs() < 0.00001,
        "end of track must not reverse"
    );
}
#[test]
fn rapid_retarget_disconnect_stale_clock_and_mode_switches_preserve_pose() {
    let now = Instant::now();
    let mut s = paused_music_state(0.1, 0.9, now);
    s.sync_playback(now);
    tick(&mut s, now + Duration::from_millis(100));
    let pose = s.arm.position;
    for seconds in [60, 20, 80, 5] {
        s.playback.borrow_mut().snapshot.as_mut().unwrap().position = Duration::from_secs(seconds);
        s.sync_playback(now + Duration::from_millis(100));
        assert_eq!(s.arm.position, pose, "retarget must preserve position");
    }
    s.playback.borrow_mut().invalidate("Disconnected");
    s.sync_playback(now + Duration::from_millis(100));
    assert!(!s.needs_redraw_at(now));
    s.advance_arm(Duration::MAX);
    assert_eq!(
        s.arm.position, pose,
        "unavailable timing must freeze, not finish an old move"
    );
    let clock = paused_music_state(pose, 0.5, now)
        .playback
        .borrow()
        .snapshot
        .clone();
    s.playback.borrow_mut().snapshot = clock;
    s.sync_playback(now);
    assert!(s.needs_redraw_at(now));
    s.playback.borrow_mut().snapshot.as_mut().unwrap().duration = Duration::ZERO;
    s.sync_playback(now);
    assert!(!s.needs_redraw_at(now));
    assert_eq!(s.arm.position, pose);
    s.playback.borrow_mut().snapshot.as_mut().unwrap().duration = Duration::from_secs(100);
    s.sync_playback(now);
    s.sync_playback(now + Duration::from_secs(7));
    assert!(!s.needs_redraw_at(now + Duration::from_secs(7)));
    s.advance_arm(Duration::MAX);
    assert_eq!(s.arm.position, pose);
    s.playback.borrow_mut().set_enabled(false);
    s.sync_playback(now);
    s.arm.play(true);
    s.arm.advance(Duration::ZERO);
    assert_eq!(
        s.arm.position, pose,
        "returning to demo must preserve the current pose"
    );
}
#[test]
fn range_contraction_and_equal_limits_settle_instead_of_clamping() {
    let now = Instant::now();
    let mut s = paused_music_state(0.7, 0.5, now);
    s.set_arm_range(0.2, 0.3, now);
    assert_eq!(s.arm.position, 0.7);
    assert!(s.needs_redraw_at(now));
    let now = settle(&mut s, now);
    assert!((s.arm.position - 0.25).abs() < 0.00001);
    s.set_arm_range(0.4, 0.4, now);
    assert!((s.arm.position - 0.25).abs() < 0.00001);
    assert!(
        s.needs_redraw_at(now),
        "equal range must finish its transition"
    );
    let now = settle(&mut s, now);
    assert_eq!(s.arm.position, 0.4);
    assert!(!s.needs_redraw_at(now));
    s.playback.borrow_mut().set_enabled(false);
    s.set_arm_range(0.5, 0.5, now);
    assert_eq!(s.arm.position, 0.5, "manual range behavior stays unchanged");
}
#[test]
fn moving_song_and_final_boundary_keep_frames_until_visual_motion_settles() {
    use crate::playback::Status;
    let now = Instant::now();
    let mut s = paused_music_state(0.5, 0.5, now);
    s.playback.borrow_mut().status = Status::Playing;
    s.playback.borrow_mut().snapshot.as_mut().unwrap().playing = true;
    s.sync_playback(now);
    assert!(s.needs_redraw_at(now));
    for i in 1..=60 {
        tick(&mut s, now + Duration::from_secs_f64(f64::from(i) / 60.0));
    }
    assert!(s.arm.position > 0.5 && s.arm.position < 0.511);
    s.playback.borrow_mut().snapshot.as_mut().unwrap().position = Duration::from_secs(100);
    s.sync_playback(now + Duration::from_secs(1));
    assert!(
        s.needs_redraw_at(now + Duration::from_secs(1)),
        "track end does not cancel catch-up"
    );
    let now = settle(&mut s, now + Duration::from_secs(1));
    assert_eq!(s.arm.position, 1.0);
    assert!(!s.needs_redraw_at(now));
}
