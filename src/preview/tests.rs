use super::*;
#[test]
fn oversized_optional_package_is_not_silently_dropped() {
    let fixture = crate::autostart::Fixture::new();
    let beauty = fixture.root.join("beauty.png");
    image::RgbaImage::new(2, 2).save(&beauty).unwrap();
    let manifest = fixture.root.join("oversized.json");
    std::fs::File::create(&manifest)
        .unwrap()
        .set_len(crate::asset_validation::MAX_MANIFEST_BYTES + 1)
        .unwrap();
    let mut config = Config::default();
    config.scene.background = beauty;
    config.scene.sleeve = None;
    config.scene.record_lighting = None;
    config.scene.buttons = Some(manifest);
    let error = match PreviewAsset::load(&config) {
        Ok(_) => panic!("oversized optional manifest accepted"),
        Err(error) => error,
    };
    assert!(
        error
            .downcast_ref::<crate::asset_validation::ResourceLimit>()
            .is_some(),
        "{error:#}"
    );
}

#[test]
#[ignore = "decodes a staged package; set TALIS_STAGED_SCENE_ROOT explicitly"]
fn staged_package_fits_budget_without_dropping_supported_layers() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init();
    let root = std::path::PathBuf::from(
        std::env::var_os("TALIS_STAGED_SCENE_ROOT").expect("explicit staged package path"),
    );
    for (mode, interpolate) in [
        (PreviewMode::Beauty, false),
        (PreviewMode::Arm, false),
        (PreviewMode::Arm, true),
    ] {
        let package = crate::config::ScenePackage::at(&root);
        let mut config = Config::default();
        config.scene.mode = mode;
        config.scene.background = package.background;
        config.scene.arm_poses = package.arm_poses;
        config.scene.arm_flow = package.arm_flow;
        config.scene.sleeve = package.sleeve;
        config.scene.record_lighting = package.record_lighting;
        config.scene.vinyl_detail = package.vinyl_detail;
        config.scene.buttons = package.buttons;
        config.render.arm_interpolation = interpolate;
        config.validate().unwrap();
        let assets = PreviewAsset::load(&config).unwrap();
        assert!(assets.sleeve.is_some());
        assert!(assets.record_lighting.is_some());
        assert!(assets.buttons.is_some());
        assert!(assets.empty_stand.is_some());
        assert!(assets.stack_sleeve.is_some());
        if let PreviewScene::Arm(arm) = &assets.scene {
            assert_eq!(arm.poses.len(), 33);
            assert!(arm.flow.as_ref().unwrap().edges.is_some());
            assert_eq!(arm.flow.as_ref().unwrap().fields.is_empty(), !interpolate);
            assert!(assets.vinyl_detail.is_some());
            assert!(assets.knob.is_some());
        }
        println!(
            "Accepted staged {mode:?}/interpolate={interpolate}; all expected layers retained"
        );
    }
}

#[test]
#[ignore = "decodes the full bundled day/night scene packages"]
fn bundled_packages_fit_budget_without_dropping_supported_layers() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init();
    for variant in [
        crate::config::SceneVariant::Day,
        crate::config::SceneVariant::Night,
    ] {
        for mode in [PreviewMode::Beauty, PreviewMode::Arm] {
            for interpolate in [false, true] {
                if mode == PreviewMode::Beauty && !interpolate {
                    continue;
                }
                let mut config = Config::load(std::path::Path::new("config/default.toml")).unwrap();
                config.scene.variant = variant;
                config.scene.mode = mode;
                config.render.arm_interpolation = interpolate;
                let assets = PreviewAsset::load(&config).unwrap();
                assert!(assets.sleeve.is_some());
                assert!(assets.record_lighting.is_some());
                assert!(assets.buttons.is_some());
                assert!(assets.empty_stand.is_some());
                assert!(assets.stack_sleeve.is_some());
                if let PreviewScene::Arm(arm) = &assets.scene {
                    assert_eq!(arm.poses.len(), 33);
                    assert!(arm.flow.as_ref().unwrap().edges.is_some());
                    assert_eq!(arm.flow.as_ref().unwrap().fields.is_empty(), !interpolate);
                    assert!(assets.vinyl_detail.is_some());
                    assert!(assets.knob.is_some());
                }
                println!(
                    "Accepted {variant:?}/{mode:?}/interpolate={interpolate}; all expected layers retained"
                );
            }
        }
    }
}
#[test]
fn missing_or_corrupt_empty_stand_keeps_the_original_scene_available() {
    let fixture = crate::autostart::Fixture::new();
    let beauty = fixture.root.join("beauty.png");
    image::RgbaImage::from_pixel(16, 9, image::Rgba([12, 34, 56, 255]))
        .save(&beauty)
        .unwrap();
    let mut config = Config::default();
    config.scene.background = beauty;
    config.scene.sleeve = None;
    config.scene.record_lighting = None;
    config.scene.vinyl_detail = None;
    config.scene.buttons = None;
    assert!(PreviewAsset::load(&config).unwrap().empty_stand.is_none());
    let folder = fixture.root.join("empty-stand");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("atlas.json"), b"broken").unwrap();
    let loaded = PreviewAsset::load(&config).unwrap();
    assert!(loaded.empty_stand.is_none());
    assert!(matches!(loaded.scene, PreviewScene::Beauty(_)));
}
#[test]
fn invalid_optional_record_lighting_keeps_the_original_scene_available() {
    let fixture = crate::autostart::Fixture::new();
    let beauty = fixture.root.join("beauty.png");
    image::RgbaImage::from_pixel(16, 9, image::Rgba([12, 34, 56, 255]))
        .save(&beauty)
        .unwrap();
    let mut config = Config::default();
    config.scene.background = beauty;
    config.scene.sleeve = None;
    config.scene.record_lighting = Some(fixture.root.join("broken.json"));
    config.scene.vinyl_detail = Some(fixture.root.join("broken.json"));
    config.scene.buttons = Some(fixture.root.join("broken.json"));
    std::fs::write(config.scene.record_lighting.as_ref().unwrap(), b"broken").unwrap();
    let loaded = PreviewAsset::load(&config).unwrap();
    assert!(loaded.record_lighting.is_none());
    assert!(loaded.vinyl_detail.is_none());
    assert!(loaded.buttons.is_none());
    assert!(matches!(loaded.scene, PreviewScene::Beauty(_)));
    config.artwork.record_lighting = false;
    assert!(
        PreviewAsset::load(&config)
            .unwrap()
            .record_lighting
            .is_none()
    );
}
#[test]
fn missing_beauty_is_an_error_not_a_silent_quality_downgrade() {
    let mut config = Config::default();
    config.scene.background = "this-beauty-does-not-exist.png".into();
    assert!(PreviewAsset::load(&config).is_err());
}
