use super::*;

#[test]
fn missing_legacy_day_and_night_paths_migrate_but_existing_files_are_authoritative() {
    let fixture = crate::autostart::Fixture::new();
    for variant in ["day", "night"] {
        let file = fixture
            .root
            .join("assets/scenes/1440p")
            .join(variant)
            .join("beauty.png");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, []).unwrap();
    }
    let mut scene = SceneSettings {
        background: "assets/scenes/day/beauty.png".into(),
        ..Default::default()
    };
    scene.night.as_mut().unwrap().background = "assets/scenes/night/beauty.png".into();
    scene.migrate_bundled_paths(&fixture.root);
    assert_eq!(
        scene.background,
        fixture.root.join("assets/scenes/1440p/day/beauty.png")
    );
    assert_eq!(
        scene.night.unwrap().background,
        fixture.root.join("assets/scenes/1440p/night/beauty.png")
    );
    let legacy = fixture.root.join("assets/scenes/day/beauty.png");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(&legacy, []).unwrap();
    let mut scene = SceneSettings {
        background: legacy.clone(),
        ..Default::default()
    };
    scene.migrate_bundled_paths(&fixture.root);
    assert_eq!(scene.background, legacy);
}
#[test]
fn shipped_defaults_match_fresh_config_and_preserve_explicit_preferences() {
    let fresh: Config = toml::from_str("").unwrap();
    let shipped: Config = toml::from_str(include_str!("../../config/default.toml")).unwrap();
    assert_eq!(
        toml::to_string(&fresh).unwrap(),
        toml::to_string(&shipped).unwrap()
    );
    assert_eq!(fresh.render.fps, 60);
    assert!(fresh.render.pause_when_hidden && fresh.minimize_to_tray);
    assert_eq!(fresh.artwork.size_percent, 36.0);
    assert_eq!(fresh.artwork.rotation_rpm, 33.33);
    assert_eq!(fresh.artwork.vinyl_opacity, 1.0);
    assert_eq!(fresh.artwork.vinyl_brightness, 1.0);
    assert_eq!(fresh.artwork.sleeve_shadow_lift, 0.25);
    assert_eq!(
        (fresh.render.arm_range_start, fresh.render.arm_range_end),
        (0.0, 1.0)
    );
    let explicit: Config = toml::from_str("minimize_to_tray=false\n[render]\nfps=30\npause_when_hidden=false\n[artwork]\nsize_percent=35\nrotation_rpm=45\n").unwrap();
    assert!(!explicit.minimize_to_tray && !explicit.render.pause_when_hidden);
    assert_eq!(explicit.render.fps, 30);
    assert_eq!(explicit.artwork.size_percent, 35.0);
    assert_eq!(explicit.artwork.rotation_rpm, 45.0);
}
#[cfg(feature = "assets")]
#[test]
fn legacy_saved_config_resolves_the_organized_bundled_package() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let config = Config::load(&root.join("tests/fixtures/legacy-assets.toml")).unwrap();
    let package = config.scene.selected_package().unwrap();
    assert_eq!(
        package.background,
        root.join("assets/scenes/1440p/day/beauty.png")
    );
    assert_eq!(
        package.arm_poses,
        root.join("assets/scenes/1440p/day/arm-poses/atlas.json")
    );
    assert!(package.record_lighting.unwrap().is_file());
    assert!(config.render.arm_interpolation);
}
struct AssetFixture(PathBuf);
impl AssetFixture {
    fn new() -> Self {
        static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "talis-asset-path-test-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let package = ScenePackage::at(&root.join("assets/scenes/1440p/day"));
        for path in [
            Some(&package.background),
            Some(&package.arm_poses),
            Some(&package.arm_flow),
            package.sleeve.as_ref(),
            package.record_lighting.as_ref(),
            package.vinyl_detail.as_ref(),
            package.buttons.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"fixture").unwrap();
        }
        Self(root)
    }
}
impl Drop for AssetFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn missing_legacy_bundled_paths_migrate_in_memory_without_touching_custom_packages() {
    let fixture = AssetFixture::new();
    for absolute in [false, true] {
        let old = if absolute {
            fixture.0.join("assets/scenes")
        } else {
            PathBuf::from("assets/scenes")
        };
        let package = ScenePackage::at(&old);
        let mut scene = SceneSettings {
            background: package.background,
            arm_poses: package.arm_poses,
            arm_flow: package.arm_flow,
            sleeve: package.sleeve,
            record_lighting: package.record_lighting,
            vinyl_detail: package.vinyl_detail,
            buttons: package.buttons,
            ..Default::default()
        };
        let night = scene.night.clone().unwrap().background;
        scene.migrate_bundled_paths(&fixture.0);
        let expected = ScenePackage::at(&fixture.0.join("assets/scenes/1440p/day"));
        assert_eq!(scene.background, expected.background);
        assert_eq!(scene.arm_poses, expected.arm_poses);
        assert_eq!(scene.arm_flow, expected.arm_flow);
        assert_eq!(scene.sleeve, expected.sleeve);
        assert_eq!(scene.record_lighting, expected.record_lighting);
        assert_eq!(scene.vinyl_detail, expected.vinyl_detail);
        assert_eq!(scene.buttons, expected.buttons);
        assert_eq!(scene.night.unwrap().background, night);
    }
    let mut scene = SceneSettings {
        background: "custom.png".into(),
        arm_poses: "/some/other/checkout/assets/scenes/arm-poses/atlas.json".into(),
        ..Default::default()
    };
    scene.migrate_bundled_paths(&fixture.0);
    assert_eq!(scene.background, Path::new("custom.png"));
    assert_eq!(
        scene.arm_poses,
        Path::new("/some/other/checkout/assets/scenes/arm-poses/atlas.json")
    );
}
#[test]
fn existing_legacy_files_and_missing_replacements_are_not_redirected() {
    let fixture = AssetFixture::new();
    let old = fixture.0.join("assets/scenes/beauty.png");
    fs::write(&old, b"custom legacy file").unwrap();
    let mut scene = SceneSettings {
        background: old.clone(),
        ..Default::default()
    };
    scene.migrate_bundled_paths(&fixture.0);
    assert_eq!(scene.background, old);
    fs::remove_file(&old).unwrap();
    fs::remove_file(fixture.0.join("assets/scenes/1440p/day/beauty.png")).unwrap();
    scene.migrate_bundled_paths(&fixture.0);
    assert_eq!(scene.background, old);
}
#[test]
fn lighting_packages_are_explicit_and_old_configs_keep_day_paths() {
    let mut config: Config = toml::from_str("[scene]\nbackground='custom.png'\n").unwrap();
    assert_eq!(config.scene.variant, SceneVariant::Day);
    assert_eq!(
        config.scene.selected_package().unwrap().background,
        Path::new("custom.png")
    );
    config.scene.variant = SceneVariant::Night;
    assert_eq!(
        config.scene.selected_package().unwrap().background,
        Path::new("assets/scenes/1440p/night/beauty.png")
    );
    let restored: Config = toml::from_str(&toml::to_string(&config).unwrap()).unwrap();
    assert_eq!(restored.scene.variant, SceneVariant::Night);
    assert_eq!(restored.scene.background, Path::new("custom.png"));
    config.scene.night = None;
    assert!(config.validate().is_err());
    config.scene.variant = SceneVariant::Day;
    assert!(config.validate().is_ok());
}
#[test]
fn record_lighting_packages_are_optional_backwards_compatible_and_absolute() {
    let mut package: ScenePackage =
        toml::from_str("background='a.png'\narm_poses='p.json'\narm_flow='f.json'\n").unwrap();
    assert!(package.record_lighting.is_none());
    assert!(package.vinyl_detail.is_none());
    assert!(package.buttons.is_none());
    package = ScenePackage::at(Path::new("assets/scenes/1440p/night"));
    package.make_absolute(Path::new("/tmp/talis-test-repo"));
    assert!(package.vinyl_detail.as_ref().unwrap().is_absolute());
    assert!(package.buttons.as_ref().unwrap().is_absolute());
    assert_eq!(
        package.record_lighting.unwrap(),
        Path::new("/tmp/talis-test-repo/assets/scenes/1440p/night/record-lighting/atlas.json")
    );
    let config: Config = toml::from_str("[artwork]\nrecord_lighting=false\n").unwrap();
    assert!(!config.artwork.record_lighting);
    assert!(config.validate().is_ok());
    assert!(Config::default().artwork.record_lighting);
}
#[test]
fn partial_config_retains_animation_defaults() {
    let config: Config = toml::from_str("[render]\nfinish = 'gold'\n").unwrap();
    assert_eq!(config.render.finish, Finish::Gold);
    assert!(config.render.animate_arm);
    assert_eq!(config.render.fps, 60);
    assert!(!config.render.arm_interpolation);
    assert_eq!(config.render.arm_response_seconds, 0.6);
    assert_eq!(
        (config.render.arm_range_start, config.render.arm_range_end),
        (0.0, 1.0)
    );
}
#[test]
fn arm_ranges_are_validated_and_roundtrip() {
    let mut config = Config::default();
    for (start, end) in [(0.2, 0.8), (0.5, 0.5), (0.0, 1.0)] {
        config.render.arm_range_start = start;
        config.render.arm_range_end = end;
        config.validate().unwrap();
        let saved: Config = toml::from_str(&toml::to_string(&config).unwrap()).unwrap();
        assert_eq!(
            (saved.render.arm_range_start, saved.render.arm_range_end),
            (start, end)
        );
    }
    for (start, end) in [
        (-0.1, 1.0),
        (0.0, 1.1),
        (0.9, 0.2),
        (f32::NAN, 1.0),
        (0.0, f32::INFINITY),
    ] {
        config.render.arm_range_start = start;
        config.render.arm_range_end = end;
        assert!(config.validate().is_err());
    }
}
#[test]
fn invalid_frame_rate_and_misspelled_setting_are_rejected() {
    let mut config = Config::default();
    config.render.fps = 0;
    assert!(config.validate().is_err());
    assert!(toml::from_str::<Config>("[render]\nanimate_arms = true").is_err());
}
#[test]
fn fidelity_baseline_is_default_and_modes_are_explicit() {
    let defaults = Config::default();
    assert_eq!(defaults.scene.mode, PreviewMode::Beauty);
    let config: Config = toml::from_str("[scene]\nmode = 'arm'\n").unwrap();
    assert_eq!(config.scene.mode, PreviewMode::Arm);
    for removed in ["raster", "placeholder"] {
        assert!(toml::from_str::<Config>(&format!("[scene]\nmode = '{removed}'\n")).is_err());
    }
    assert!(toml::from_str::<Config>("[scene]\nmodel = 'old.glb'\n").is_err());
    assert!(toml::from_str::<Config>("[scene]\nmode = 'photoreal_3d'\n").is_err());
}
#[test]
fn hidden_pause_defaults_on_and_roundtrips_without_breaking_old_configs() {
    let mut config: Config = toml::from_str("[render]\nfps = 30\n").unwrap();
    assert!(config.render.pause_when_hidden);
    config.render.pause_when_hidden = false;
    let text = toml::to_string(&config).unwrap();
    assert!(
        !toml::from_str::<Config>(&text)
            .unwrap()
            .render
            .pause_when_hidden
    );
}
#[test]
fn interpolation_is_opt_in_and_requires_arm_mode() {
    let mut config = Config::default();
    config.render.arm_interpolation = true;
    assert!(config.validate().is_err());
    config.scene.mode = PreviewMode::Arm;
    assert!(config.validate().is_ok());
}
#[test]
fn mate_overlay_defaults_off_and_roundtrips() {
    let mut config: Config = toml::from_str("[render]\nfps = 30\n").unwrap();
    assert!(!config.render.experimental_mate_overlay);
    config.render.experimental_mate_overlay = true;
    let saved = toml::to_string(&config).unwrap();
    assert!(
        toml::from_str::<Config>(&saved)
            .unwrap()
            .render
            .experimental_mate_overlay
    );
}
#[test]
fn motion_response_is_bounded_and_nonfinite_settings_are_rejected() {
    for seconds in [0.0, -1.0, 0.19, 2.01, f64::NAN, f64::INFINITY] {
        let mut config = Config::default();
        config.render.arm_response_seconds = seconds;
        assert!(config.validate().is_err());
    }
    for seconds in [0.2, 0.6, 2.0] {
        let config: Config =
            toml::from_str(&format!("[render]\narm_response_seconds = {seconds}")).unwrap();
        assert!(config.validate().is_ok());
    }
}
