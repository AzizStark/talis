mod control_integration;
pub(super) mod integration;
mod volume_integration;
use super::*;
#[test]
fn playing_selection_is_independent_of_timing_but_expires() {
    let now = Instant::now();
    let mut entry = Entry::new(now);
    entry.artwork_playback = Some((Status::Playing, now));
    assert!(entry.sample.is_none());
    assert!(entry.playing(now));
    assert!(!entry.playing(now + Duration::from_secs(7)));
    entry.artwork_playback = Some((Status::Paused, now));
    assert!(!entry.playing(now));
    entry.artwork_playback = Some((Status::Stopped, now));
    assert!(!entry.playing(now));
}
#[test]
fn artwork_is_optional_typed_and_independent_of_timing() {
    let mut identity = identity::Tracker::default();
    let mut p = properties(0_i64.to_variant());
    assert!(artwork(&p, identity.observe(&p)).is_none());
    assert_eq!(identity.observe(&p).as_deref(), Some("/track/one"));
    let mut metadata = p["Metadata"].get::<Properties>().unwrap();
    metadata.insert(
        "mpris:artUrl".into(),
        "file:///tmp/cover%20art.png".to_variant(),
    );
    p.insert("Metadata".into(), metadata.clone().to_variant());
    p.remove("Position");
    p.insert("PlaybackStatus".into(), "Playing".to_variant());
    assert_eq!(artwork_playback(&p).unwrap().0, Status::Playing);
    assert!(decode(p.clone(), Instant::now()).is_err());
    assert_eq!(
        artwork(&p, identity.observe(&p)).unwrap().url.as_ref(),
        "file:///tmp/cover%20art.png"
    );
    metadata.insert("mpris:artUrl".into(), 123_i64.to_variant());
    p.insert("Metadata".into(), metadata.to_variant());
    assert!(artwork(&p, identity.observe(&p)).is_none());
    assert_eq!(identity.observe(&p).as_deref(), Some("/track/one"));
}
#[test]
fn inline_artwork_exceeds_old_url_limit_but_remains_bounded() {
    let mut p = properties(0_i64.to_variant());
    let mut metadata = p["Metadata"].get::<Properties>().unwrap();
    let url = format!("data:image/png;base64,{}", "A".repeat(9000));
    metadata.insert("mpris:artUrl".into(), url.to_variant());
    p.insert("Metadata".into(), metadata.clone().to_variant());
    let source = artwork(&p, Some("one".into())).unwrap();
    assert_eq!(source.url.as_ref(), url);
    assert!(std::sync::Arc::ptr_eq(&source.url, &source.clone().url));
    for url in [
        "https://example.org/".to_owned() + &"a".repeat(9000),
        "data:image/png;base64,".to_owned() + &"A".repeat(23 * 1024 * 1024),
    ] {
        metadata.insert("mpris:artUrl".into(), url.to_variant());
        p.insert("Metadata".into(), metadata.clone().to_variant());
        assert!(artwork(&p, Some("one".into())).is_none());
    }
}
fn properties(length: glib::Variant) -> Properties {
    let metadata: Properties = [
        (
            "mpris:trackid".into(),
            glib::variant::ObjectPath::try_from("/track/one")
                .unwrap()
                .to_variant(),
        ),
        ("mpris:length".into(), length),
    ]
    .into();
    [
        ("PlaybackStatus".into(), "Playing".to_variant()),
        ("Position".into(), 10_000_000i64.to_variant()),
        ("Rate".into(), 1.0f64.to_variant()),
        ("Metadata".into(), metadata.to_variant()),
    ]
    .into()
}
#[test]
fn accepts_standard_and_spotify_duration_types() {
    for length in [100_000_000i64.to_variant(), 100_000_000u64.to_variant()] {
        let sample = decode(properties(length), Instant::now()).unwrap();
        assert_eq!(sample.clock.duration, Duration::from_secs(100));
        assert_eq!(sample.clock.progress_at(sample.clock.observed_at), 0.1);
    }
}
#[test]
fn firefox_missing_rate_uses_normal_speed_and_preserves_artwork() {
    let now = Instant::now();
    let mut p = properties(231_000_000_i64.to_variant());
    p.remove("Rate");
    p.insert("Position".into(), 168_000_000_i64.to_variant());
    let mut metadata = p["Metadata"].get::<Properties>().unwrap();
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
        "file:///tmp/firefox.png".to_variant(),
    );
    p.insert("Metadata".into(), metadata.to_variant());
    let sample = decode(p.clone(), now).unwrap();
    assert_eq!(sample.clock.rate, 1.0);
    assert!((sample.clock.progress_at(now) - 168.0 / 231.0).abs() < 0.00001);
    assert_eq!(
        sample.clock.position_at(now + Duration::from_secs(2)),
        Duration::from_secs(170)
    );
    let mut identity = identity::Tracker::default();
    assert_eq!(
        artwork(&p, identity.observe(&p)).unwrap().url.as_ref(),
        "file:///tmp/firefox.png"
    );
}
#[test]
fn timing_does_not_require_a_track_identity_or_cover() {
    let now = Instant::now();
    for track in [
        None,
        Some("".to_variant()),
        Some(42_i64.to_variant()),
        Some(
            glib::variant::ObjectPath::try_from("/org/mpris/MediaPlayer2/TrackList/NoTrack")
                .unwrap()
                .to_variant(),
        ),
    ] {
        let mut p = properties(238_027_000_i64.to_variant());
        p.insert("Position".into(), 158_455_000_i64.to_variant());
        let mut metadata = p["Metadata"].get::<Properties>().unwrap();
        metadata.remove("mpris:trackid");
        if let Some(track) = track {
            metadata.insert("mpris:trackid".into(), track);
        }
        p.insert("Metadata".into(), metadata.to_variant());
        let sample = decode(p.clone(), now).unwrap();
        assert!((sample.clock.progress_at(now) - 158.455 / 238.027).abs() < 0.00001);
        let mut identity = identity::Tracker::default();
        assert!(identity.observe(&p).is_none());
        assert!(artwork(&p, identity.observe(&p)).is_none());
    }
}
#[test]
fn explicit_forward_and_reverse_rates_are_preserved() {
    let now = Instant::now();
    for (rate, expected_seconds) in [(2.0_f64, 14), (-1.0, 8)] {
        let mut p = properties(100_000_000_i64.to_variant());
        p.insert("Rate".into(), rate.to_variant());
        let sample = decode(p, now).unwrap();
        assert_eq!(sample.clock.rate, rate);
        assert_eq!(
            sample.clock.position_at(now + Duration::from_secs(2)),
            Duration::from_secs(expected_seconds)
        );
    }
}
#[test]
fn malformed_fields_and_nonfinite_rates_are_recoverable() {
    for rate in [0.0, f64::NAN, f64::INFINITY] {
        let mut p = properties(100i64.to_variant());
        p.insert("Rate".into(), rate.to_variant());
        assert!(decode(p, Instant::now()).is_err());
    }
    for key in ["PlaybackStatus", "Position", "Metadata", "Rate"] {
        let mut p = properties(100i64.to_variant());
        p.insert(key.into(), "wrong".to_variant());
        assert!(decode(p, Instant::now()).is_err());
    }
    let mut p = properties(100i64.to_variant());
    p.insert("Position".into(), (-1i64).to_variant());
    assert!(decode(p, Instant::now()).is_err());
}
#[test]
fn unknown_zero_negative_and_huge_duration_never_panics() {
    for length in [
        0i64.to_variant(),
        (-5i64).to_variant(),
        "unknown".to_variant(),
        u64::MAX.to_variant(),
    ] {
        let sample = decode(properties(length), Instant::now()).unwrap();
        assert!(sample.clock.progress_at(Instant::now()).is_finite());
    }
}
