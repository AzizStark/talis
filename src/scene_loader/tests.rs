use super::*;
use crate::config::SceneVariant;
use std::time::{Duration, Instant};
fn wait(loader: &SceneLoader) -> Result<(SceneSettings, PreviewAsset)> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(result) = loader.poll() {
            return result;
        }
        assert!(Instant::now() < deadline, "worker did not respond");
        thread::sleep(Duration::from_millis(2));
    }
}
fn image() -> PreviewAsset {
    PreviewAsset {
        variant: SceneVariant::Day,
        scene: crate::preview::PreviewScene::Beauty(image::RgbaImage::new(1, 1)),
        sleeve: None,
        record_lighting: None,
        vinyl_detail: None,
        buttons: None,
        knob: None,
        empty_stand: None,
        stack_sleeve: None,
    }
}
#[test]
fn cancelling_an_unconsumed_candidate_releases_its_memory_immediately() {
    let loader = SceneLoader::with_loader(|_| Ok(image())).unwrap();
    loader.request(Config::default()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while loader.results.lock().unwrap().is_none() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    loader.cancel();
    assert!(loader.results.lock().unwrap().is_none());
    assert!(loader.poll().is_none());
    assert!(!loader.pending());
    loader.request(Config::default()).unwrap();
    assert!(wait(&loader).is_ok());
}
#[test]
fn memory_limit_failure_does_not_kill_worker_or_poison_next_load() {
    let fixture = crate::autostart::Fixture::new();
    let beauty = fixture.root.join("beauty.png");
    image::RgbaImage::new(2, 2).save(&beauty).unwrap();
    let oversized = fixture.root.join("oversized.png");
    std::fs::File::create(&oversized)
        .unwrap()
        .set_len(crate::asset_validation::MAX_ASSET_BYTES + 1)
        .unwrap();
    let mut config = Config::default();
    config.scene.background = oversized;
    config.scene.sleeve = None;
    config.scene.record_lighting = None;
    config.scene.buttons = None;
    let loader = SceneLoader::new().unwrap();
    loader.request(config.clone()).unwrap();
    let error = match wait(&loader) {
        Ok(_) => panic!("oversized scene accepted"),
        Err(error) => error,
    };
    assert!(
        error
            .downcast_ref::<crate::asset_validation::ResourceLimit>()
            .is_some(),
        "{error:#}"
    );
    assert!(!loader.pending());
    config.scene.background = beauty;
    loader.request(config).unwrap();
    assert!(matches!(
        wait(&loader).unwrap().1.scene,
        crate::preview::PreviewScene::Beauty(_)
    ));
}
#[test]
fn newest_scene_wins_and_cancelled_loads_never_publish() {
    let (entered, started) = mpsc::channel();
    let (release, blocked) = mpsc::channel();
    let loader = SceneLoader::with_loader(move |config| {
        if config.scene.variant == SceneVariant::Night {
            entered.send(()).unwrap();
            blocked.recv().unwrap();
        }
        Ok(image())
    })
    .unwrap();
    let mut night = Config::default();
    night.scene.variant = SceneVariant::Night;
    loader.request(night.clone()).unwrap();
    started.recv_timeout(Duration::from_secs(1)).unwrap();
    loader.request(Config::default()).unwrap();
    release.send(()).unwrap();
    assert_eq!(wait(&loader).unwrap().0.variant, SceneVariant::Day);
    assert!(!loader.pending());
    loader.request(night).unwrap();
    started.recv_timeout(Duration::from_secs(1)).unwrap();
    loader.cancel();
    release.send(()).unwrap();
    loader.request(Config::default()).unwrap();
    assert_eq!(wait(&loader).unwrap().0.variant, SceneVariant::Day);
    assert!(loader.poll().is_none());
}
#[test]
fn resolution_and_scene_are_one_coalesced_request() {
    let (entered, started) = mpsc::channel();
    let (release, blocked) = mpsc::channel();
    let loader = SceneLoader::with_loader(move |config| {
        if config.scene.resolution == crate::config::SceneResolution::FourK {
            entered.send(()).unwrap();
            blocked.recv().unwrap();
        }
        Ok(image())
    })
    .unwrap();
    let mut high = Config::default();
    high.scene.resolution = crate::config::SceneResolution::FourK;
    loader.request(high).unwrap();
    started.recv_timeout(Duration::from_secs(1)).unwrap();
    let mut latest = Config::default();
    latest.scene.variant = SceneVariant::Night;
    latest.scene.resolution = crate::config::SceneResolution::Qhd;
    let expected = latest.scene.clone();
    loader.request(latest).unwrap();
    release.send(()).unwrap();
    assert_eq!(wait(&loader).unwrap().0, expected);
    assert!(loader.poll().is_none());
}
#[test]
fn failures_and_panics_are_reported_without_killing_worker() {
    let loader = SceneLoader::with_loader(|config| {
        if config.scene.variant == SceneVariant::Night {
            anyhow::bail!("bad night");
        }
        if config.render.fps == 1 {
            panic!("decoder test panic");
        }
        Ok(image())
    })
    .unwrap();
    let mut night = Config::default();
    night.scene.variant = SceneVariant::Night;
    loader.request(night).unwrap();
    assert!(wait(&loader).is_err());
    loader.request(Config::default()).unwrap();
    assert!(wait(&loader).is_ok());
    let mut panic = Config::default();
    panic.render.fps = 1;
    loader.request(panic).unwrap();
    assert!(wait(&loader).is_err());
    loader.request(Config::default()).unwrap();
    assert!(wait(&loader).is_ok());
}
