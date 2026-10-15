use super::*;
use std::fs;
#[test]
fn flow_and_mask_group_budgets_reject_before_source_hash_reads() {
    let fixture = crate::autostart::Fixture::new();
    let (poses, mut manifest) = manifests();
    let path = fixture.root.join("flow.json");
    let check = |interpolate, manifest: &FlowManifest| {
        fs::write(&path, serde_json::to_vec(manifest).unwrap()).unwrap();
        let mut budget = crate::asset_validation::SceneBudget::default();
        budget
            .reserve(crate::asset_validation::MAX_SCENE_BYTES - 1, "other layers")
            .unwrap();
        let error = match ArmFlowAssets::load_budgeted(
            &path,
            &fixture.root.join("missing-poses.json"),
            &poses,
            interpolate,
            &mut budget,
        ) {
            Ok(_) => panic!("oversized group accepted"),
            Err(error) => error,
        };
        assert!(
            error
                .downcast_ref::<crate::asset_validation::ResourceLimit>()
                .is_some(),
            "{error:#}"
        );
    };
    check(true, &manifest);
    let file = || FlowPair {
        file: "mask.png".into(),
        sha256: "0".repeat(64),
    };
    manifest.edge_layers = Some(EdgeLayersManifest {
        masks: vec![file(), file()],
        background: file(),
    });
    check(false, &manifest);
}
fn manifests() -> (PoseManifest, FlowManifest) {
    let poses = PoseManifest {
        format_version: 1,
        width: 8,
        height: 8,
        crop: [0, 0, 8, 8],
        base_image_sha256: "0".repeat(64),
        frames: vec![
            crate::arm_assets::PoseFrame {
                file: "a.png".into(),
                progress: 0.0,
                angle_degrees: 0.0,
            },
            crate::arm_assets::PoseFrame {
                file: "b.png".into(),
                progress: 1.0,
                angle_degrees: 1.0,
            },
        ],
    };
    let flow = FlowManifest {
        format_version: 1,
        width: 4,
        height: 4,
        crop_width: 8,
        crop_height: 8,
        encoding: "rgba16_unorm_bias32768_scale256".into(),
        pose_manifest_sha256: "0".repeat(64),
        pose_images_sha256: vec!["0".repeat(64); 2],
        pairs: vec![FlowPair {
            file: "flow.png".into(),
            sha256: "0".repeat(64),
        }],
        edge_layers: None,
    };
    (poses, flow)
}
#[test]
fn encoding_keeps_direction_and_subpixel_precision() {
    assert_eq!(decode_component(32768), 0.0);
    assert_eq!(decode_component(33024), 1.0);
    assert_eq!(decode_component(32384), -1.5);
    assert_eq!(decode_component(32769), 1.0 / 256.0);
}
#[test]
fn displacement_bound_includes_both_directions_and_diagonal_motion() {
    let mut field = image::ImageBuffer::from_pixel(2, 1, image::Rgba([32768_u16; 4]));
    field.put_pixel(0, 0, image::Rgba([33536, 33792, 32768, 32768]));
    assert_eq!(max_displacement(&field), 5.0);
    field.put_pixel(1, 0, image::Rgba([32768, 32768, 32768, 31232]));
    assert_eq!(max_displacement(&field), 6.0);
}
#[test]
fn loader_binds_cache_to_manifest_pose_images_and_field_bytes() {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("talis-flow-test-{}-{stamp}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let (poses, mut flow) = manifests();
    let pose_path = root.join("poses.json");
    let flow_path = root.join("flow.json");
    fs::write(&pose_path, b"pose cache identity").unwrap();
    let image = image::RgbaImage::from_pixel(8, 8, image::Rgba([10, 20, 30, 255]));
    image.save(root.join("a.png")).unwrap();
    image.save(root.join("b.png")).unwrap();
    let field = image::ImageBuffer::from_pixel(4, 4, image::Rgba([32768u16; 4]));
    field.save(root.join("flow.png")).unwrap();
    let hash = |name: &str| sha256(&fs::read(root.join(name)).unwrap());
    flow.pose_manifest_sha256 = hash("poses.json");
    flow.pose_images_sha256 = vec![hash("a.png"), hash("b.png")];
    flow.pairs[0].sha256 = hash("flow.png");
    fs::write(&flow_path, serde_json::to_vec(&flow).unwrap()).unwrap();
    assert!(ArmFlowAssets::load(&flow_path, &pose_path, &poses).is_ok());
    fs::write(root.join("a.png"), b"changed pose").unwrap();
    assert!(ArmFlowAssets::load(&flow_path, &pose_path, &poses).is_err());
    image.save(root.join("a.png")).unwrap();
    fs::write(root.join("flow.png"), b"corrupt flow").unwrap();
    assert!(ArmFlowAssets::load(&flow_path, &pose_path, &poses).is_err());
    field.save(root.join("flow.png")).unwrap();
    fs::write(&pose_path, b"changed manifest").unwrap();
    assert!(ArmFlowAssets::load(&flow_path, &pose_path, &poses).is_err());
    fs::write(&pose_path, b"pose cache identity").unwrap();
    let mask = image::GrayImage::from_pixel(8, 8, image::Luma([128]));
    mask.save(root.join("mask.png")).unwrap();
    image.save(root.join("background.png")).unwrap();
    flow.edge_layers = Some(EdgeLayersManifest {
        masks: (0..2)
            .map(|_| FlowPair {
                file: "mask.png".into(),
                sha256: hash("mask.png"),
            })
            .collect(),
        background: FlowPair {
            file: "background.png".into(),
            sha256: hash("background.png"),
        },
    });
    fs::write(&flow_path, serde_json::to_vec(&flow).unwrap()).unwrap();
    let loaded = ArmFlowAssets::load(&flow_path, &pose_path, &poses).unwrap();
    let mut rgb_poses = vec![image.clone(), image.clone()];
    loaded.edges.unwrap().apply_masks(&mut rgb_poses).unwrap();
    assert!(
        rgb_poses
            .iter()
            .all(|pose| pose.pixels().all(|pixel| pixel.0 == [10, 20, 30, 128]))
    );
    fs::write(root.join("mask.png"), b"corrupt mask").unwrap();
    assert!(ArmFlowAssets::load(&flow_path, &pose_path, &poses).is_err());
    // A valid hash cannot turn a color image into an accepted coverage mask.
    image.save(root.join("mask.png")).unwrap();
    for entry in &mut flow.edge_layers.as_mut().unwrap().masks {
        entry.sha256 = hash("mask.png");
    }
    fs::write(&flow_path, serde_json::to_vec(&flow).unwrap()).unwrap();
    assert!(ArmFlowAssets::load(&flow_path, &pose_path, &poses).is_err());
    // This directory was exclusively created by this test, never an input path.
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn rejects_bad_dimensions_counts_encoding_and_paths() {
    let (poses, mut flow) = manifests();
    assert!(flow.validate(&poses).is_ok());
    flow.width = 9;
    assert!(flow.validate(&poses).is_err());
    flow.width = 4;
    flow.pairs[0].file = "../flow.png".into();
    assert!(flow.validate(&poses).is_err());
    flow.pairs[0].file = "flow.png".into();
    flow.encoding = "rgba8".into();
    assert!(flow.validate(&poses).is_err());
    flow.encoding = "rgba16_unorm_bias32768_scale256".into();
    flow.pose_images_sha256.pop();
    assert!(flow.validate(&poses).is_err());
}
#[test]
fn rejects_edge_layers_with_missing_masks_or_external_paths() {
    let (poses, mut flow) = manifests();
    flow.edge_layers = Some(EdgeLayersManifest {
        masks: Vec::new(),
        background: FlowPair {
            file: "background.png".into(),
            sha256: "0".repeat(64),
        },
    });
    assert!(flow.validate(&poses).is_err());
    let edges = flow.edge_layers.as_mut().unwrap();
    edges.masks = (0..2)
        .map(|_| FlowPair {
            file: "mask.png".into(),
            sha256: "0".repeat(64),
        })
        .collect();
    edges.background.file = "../background.png".into();
    assert!(flow.validate(&poses).is_err());
}
