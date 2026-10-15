//! Compare physically packed assets against the unchanged wide masters.
use super::super::*;
use super::arm_cadence_tests::{owned_area, pixels};
use crate::config::{Config, PreviewMode, SceneVariant};
use gtk::prelude::*;
use std::path::PathBuf;

fn assets(variant: SceneVariant, root: Option<&std::path::Path>) -> PreviewAsset {
    let mut c = Config::default();
    c.scene.variant = variant;
    c.scene.mode = PreviewMode::Arm;
    c.render.arm_interpolation = true;
    if let Some(root) = root {
        if variant == SceneVariant::Day {
            c.scene.arm_poses = root.join("arm-poses/atlas.json");
            c.scene.arm_flow = root.join("arm-flow/atlas.json");
        } else {
            let p = c.scene.night.as_mut().unwrap();
            p.arm_poses = root.join("arm-poses/atlas.json");
            p.arm_flow = root.join("arm-flow/atlas.json");
        }
    }
    PreviewAsset::load(&c).unwrap()
}

#[test]
#[ignore = "requires owned GTK/OpenGL, wide installed masters and TALIS_ARM_PACK_ROOT staged packages"]
fn packed_arm_all_pairs_preserve_pixels_foreground_and_cadence() {
    let root = PathBuf::from(
        std::env::var_os("TALIS_ARM_PACK_ROOT").expect("Set staged arm package root"),
    );
    let masters = std::env::var_os("TALIS_ARM_MASTER_ROOT").map(PathBuf::from);
    let mut settings = Config::default().artwork;
    if std::env::var_os("TALIS_ARM_TEST_LARGE_ART").is_some() {
        settings.size_percent = 100.0;
    }
    let (_window, area) = owned_area();
    let cover =
        image::RgbaImage::from_fn(256, 256, |x, y| image::Rgba([x as u8, y as u8, 100, 255]));
    for scene in [SceneVariant::Day, SceneVariant::Night] {
        let master_path = masters.as_ref().map(|r| {
            if scene == SceneVariant::Day {
                r.clone()
            } else {
                r.join("night")
            }
        });
        let original = assets(scene, master_path.as_deref());
        let path = if scene == SceneVariant::Day {
            root.clone()
        } else {
            root.join("night")
        };
        let packed = assets(scene, Some(&path));
        let (crate::preview::PreviewScene::Arm(a), crate::preview::PreviewScene::Arm(b)) =
            (&original.scene, &packed.scene)
        else {
            panic!("Arm required")
        };
        assert_eq!(a.update_crop, b.update_crop);
        assert!(
            a.manifest.crop[2] > b.manifest.crop[2],
            "Comparison requires wide masters"
        );
        assert_eq!(
            a.flow.as_ref().unwrap().max_displacements,
            b.flow.as_ref().unwrap().max_displacements
        );
        let pairs = a.manifest.frames.len() - 1;
        let mut wide = unsafe { Renderer::new(original).unwrap() };
        let mut narrow = unsafe { Renderer::new(packed).unwrap() };
        eprintln!("PACK_DRIVER {}", unsafe {
            wide.gl.get_parameter_string(glow::RENDERER)
        });
        for r in [&mut wide, &mut narrow] {
            unsafe { r.set_artwork(1, Some(&cover), &settings).unwrap() };
        }
        for (w, h) in [(2560, 1440), (3840, 2160), (1919, 1079), (3440, 1440)] {
            area.allocate(w, h, -1, None);
            area.attach_buffers();
            let mut maximum = 0_u8;
            let mut fg_max = 0_u8;
            let mut changed = 0_u64;
            let mut total = 0_u64;
            let mut fg_error = 0_u64;
            let mut fg_samples = 0_u64;
            let selected: Vec<_> = if w == 2560 {
                (0..pairs).collect()
            } else {
                vec![0, 1, pairs / 2, pairs - 1]
            };
            for pair in selected {
                for t in [0.0, 0.01, 0.125, 0.375, 0.5, 0.875, 0.99, 1.0] {
                    let p = (pair as f32 + t) / pairs as f32;
                    for r in [&mut wide, &mut narrow] {
                        r.set_artwork_angle(p * 19.0);
                        r.set_button_progress([t, 0.25, 0.75]);
                        r.set_knob_angle(t);
                        unsafe {
                            r.render(w, h, p).unwrap();
                            r.check_error().unwrap()
                        };
                    }
                    let a = unsafe { pixels(&wide, w, h) };
                    let b = unsafe { pixels(&narrow, w, h) };
                    for (a, b) in a.iter().zip(&b) {
                        let d = a.abs_diff(*b);
                        maximum = maximum.max(d);
                        changed += u64::from(d != 0);
                        total += u64::from(d);
                    }
                    let a = unsafe { wide.source.test_foreground() };
                    let b = unsafe { narrow.source.test_foreground() };
                    for (a, b) in a.iter().zip(&b) {
                        let d = a.abs_diff(*b);
                        fg_max = fg_max.max(d);
                        fg_error += u64::from(d);
                        fg_samples += 1;
                    }
                }
            }
            eprintln!(
                "PACK_PIXELS {scene:?} {w}x{h} max={maximum}/255 foreground_max={fg_max}/255 changed={changed} total_error={total} foreground_error={fg_error}/{fg_samples} size={}",
                settings.size_percent
            );
            assert!(
                // Resizing normalized UV denominators can alter GPU edge
                // rounding by <=2 R8 levels, even with identical source pixels.
                // Bound the mean too, and compare final artwork/dust occlusion.
                maximum <= 1 && fg_max <= 2 && (fg_error as f64) / (fg_samples as f64) < 0.0001,
                "Packed sampling changed visible colour/coverage"
            );
        }
        unsafe {
            wide.destroy();
            narrow.destroy();
        }
    }
}
