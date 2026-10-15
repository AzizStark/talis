use super::*;
use std::fs;

#[test]
fn bundled_metadata_has_no_private_absolute_paths() {
    fn check_value(value: &serde_json::Value, path: &Path) {
        match value {
            serde_json::Value::String(value) => assert!(
                !value.starts_with('/'),
                "Bundled metadata contains an absolute path: {}: {value}",
                path.display()
            ),
            serde_json::Value::Array(values) => {
                for value in values {
                    check_value(value, path);
                }
            }
            serde_json::Value::Object(values) => {
                assert!(
                    !values.contains_key("stage_root"),
                    "Bake workspace is not release metadata"
                );
                for value in values.values() {
                    check_value(value, path);
                }
            }
            _ => (),
        }
    }
    fn check_directory(path: &Path) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                check_directory(&path);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                let value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                check_value(&value, &path);
            }
        }
    }
    check_directory(&Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/scenes"));
}

fn fixture() -> crate::autostart::Fixture {
    let f = crate::autostart::Fixture::new();
    let tier = f.root.join("assets/scenes/4k");
    for variant in ["day", "night"] {
        for name in PACKAGE_FILES {
            let file = tier.join(variant).join(name);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, []).unwrap();
        }
    }
    fs::write(
        tier.join("resolution.json"),
        r#"{"resolution":[3840,2160]}"#,
    )
    .unwrap();
    f
}

#[test]
fn selects_a_complete_pair_and_preserves_scene_preferences() {
    let f = fixture();
    let mut scene = SceneSettings {
        variant: super::super::SceneVariant::Night,
        ..Default::default()
    };
    assert!(scene.select_4k_at(&f.root));
    assert_eq!(scene.variant, super::super::SceneVariant::Night);
    assert_eq!(
        scene.background,
        f.root.join("assets/scenes/4k/day/beauty.png")
    );
    assert_eq!(
        scene.selected_package().unwrap().background,
        f.root.join("assets/scenes/4k/night/beauty.png")
    );
    assert!(
        !scene.select_4k_at(&f.root),
        "never rewrite a tier already selected"
    );
}

#[test]
fn custom_disabled_and_incomplete_packages_are_not_redirected() {
    let f = fixture();
    for field in ["background", "flow", "sleeve", "night"] {
        let mut s = SceneSettings::default();
        match field {
            "background" => s.background = "custom.png".into(),
            "flow" => s.arm_flow = "custom-flow.json".into(),
            "sleeve" => s.sleeve = None,
            "night" => s.night = None,
            _ => unreachable!(),
        }
        assert!(!s.select_4k_at(&f.root));
    }
    fs::remove_file(f.root.join("assets/scenes/4k/night/knob/atlas.json")).unwrap();
    let mut s = SceneSettings::default();
    assert!(!s.select_4k_at(&f.root));
    assert_eq!(s.background, SceneSettings::default().background);
}

#[test]
fn invalid_receipts_and_small_targets_stay_on_original_assets() {
    let f = fixture();
    let mut s = SceneSettings::default();
    for size in [
        (1920, 1080),
        (2560, 1440),
        (3440, 1440),
        (2160, 3840),
        (0, 0),
    ] {
        assert!(!s.resolve_at(&f.root, size.0, size.1).unwrap());
    }
    for data in ["broken", r#"{"resolution":[2560,1440]}"#] {
        fs::write(f.root.join("assets/scenes/4k/resolution.json"), data).unwrap();
        assert!(!s.select_4k_at(&f.root));
    }
}

#[test]
fn explicit_tiers_override_target_and_automatic_can_downgrade() {
    let f = fixture();
    let mut s = SceneSettings {
        resolution: SceneResolution::FourK,
        ..Default::default()
    };
    assert!(s.resolve_at(&f.root, 1280, 720).unwrap());
    assert!(s.matches_tier(&f.root, "4k"));
    s.resolution = SceneResolution::Qhd;
    assert!(s.resolve_at(&f.root, 3840, 2160).unwrap());
    assert!(s.matches_tier(&f.root, "1440p"));
    s.resolution = SceneResolution::Automatic;
    assert!(s.resolve_at(&f.root, 3840, 2160).unwrap());
    assert!(s.resolve_at(&f.root, 2560, 1440).unwrap());
    assert!(!s.resolve_at(&f.root, 2560, 1440).unwrap());
}

#[test]
fn missing_explicit_4k_is_an_error_without_path_changes() {
    let f = fixture();
    fs::remove_file(f.root.join("assets/scenes/4k/night/knob/atlas.json")).unwrap();
    let mut s = SceneSettings {
        resolution: SceneResolution::FourK,
        ..Default::default()
    };
    let before = s.clone();
    assert!(s.resolve_at(&f.root, 1920, 1080).is_err());
    assert_eq!(s, before);
    s.resolution = SceneResolution::Automatic;
    assert!(!s.resolve_at(&f.root, 3840, 2160).unwrap());
}

#[test]
fn full_hd_selects_both_variants_and_automatic_respects_physical_size() {
    let f = fixture();
    let tier = f.root.join("assets/scenes/1080p");
    for variant in ["day", "night"] {
        for name in PACKAGE_FILES {
            let file = tier.join(variant).join(name);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, []).unwrap();
        }
    }
    fs::write(
        tier.join("resolution.json"),
        r#"{"resolution":[1920,1080]}"#,
    )
    .unwrap();
    let mut s = SceneSettings {
        variant: super::super::SceneVariant::Night,
        ..Default::default()
    };
    for (size, expected) in [
        ((1920, 1080), "1080p"),
        ((1280, 720), "1080p"),
        ((2560, 1440), "1440p"),
        ((3840, 2160), "4k"),
        ((3440, 1440), "1440p"),
        ((0, 0), "1440p"),
    ] {
        s.resolve_at(&f.root, size.0, size.1).unwrap();
        assert!(s.matches_tier(&f.root, expected));
        assert_eq!(s.variant, super::super::SceneVariant::Night);
    }
    s.resolution = SceneResolution::FullHd;
    s.resolve_at(&f.root, 3840, 2160).unwrap();
    assert!(s.matches_tier(&f.root, "1080p"));
    fs::remove_file(tier.join("night/knob/atlas.json")).unwrap();
    let before = s.clone();
    assert!(s.resolve_at(&f.root, 1920, 1080).is_err());
    assert_eq!(s, before);
    s.resolution = SceneResolution::Automatic;
    s.resolve_at(&f.root, 1920, 1080).unwrap();
    assert!(
        s.matches_tier(&f.root, "1440p"),
        "missing optional tier falls back to 1440p"
    );
}
