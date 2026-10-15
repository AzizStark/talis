//! Owned, unmapped GPU regressions: no real desktop window or music player changes.
use super::super::*;
use super::arm_cadence_tests;
use crate::config::{Config, PreviewMode, SceneVariant};
use gtk::prelude::*;
use std::{path::PathBuf, time::Instant};

unsafe fn capture(
    r: &mut Renderer,
    area: &gtk::GLArea,
    w: i32,
    h: i32,
    p: f32,
) -> image::RgbaImage {
    area.allocate(w, h, -1, None);
    area.make_current();
    area.attach_buffers();
    unsafe {
        r.source.render_at(w, h, p, Instant::now()).unwrap();
        r.check_error().unwrap();
        let bottom_up = arm_cadence_tests::pixels(r, w, h);
        image::imageops::flip_vertical(
            &image::RgbaImage::from_raw(w as u32, h as u32, bottom_up).unwrap(),
        )
    }
}
#[test]
#[ignore = "requires owned GTK/OpenGL context and both scene packages"]
fn previous_sleeve_is_lit_cached_and_never_overwritten_by_arm_or_rotation() {
    let (_window, area) = arm_cadence_tests::owned_area();
    let output = std::env::var_os("TALIS_STACK_CAPTURE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("talis-stack-qa-{}", std::process::id()))
        });
    std::fs::create_dir_all(&output).unwrap();
    for variant in [SceneVariant::Day, SceneVariant::Night] {
        let mut config = Config::default();
        config.scene.variant = variant;
        config.scene.mode = PreviewMode::Arm;
        config.render.arm_interpolation = true;
        let asset = PreviewAsset::load(&config).unwrap();
        let stack = asset.stack_sleeve.as_ref().expect("matched stack lighting");
        let bounds = stack.manifest.crop;
        let normalized = [
            bounds[0] as f32 / 2560.0,
            bounds[1] as f32 / 1440.0,
            bounds[2] as f32 / 2560.0,
            bounds[3] as f32 / 1440.0,
        ];
        let arm = match &asset.scene {
            crate::preview::PreviewScene::Arm(a) => a,
            _ => panic!(),
        };
        assert_eq!(arm.update_crop, Some([1160, 585, 600, 630]));
        let mut r = unsafe { Renderer::new(asset).unwrap() };
        let cover = image::RgbaImage::from_fn(512, 512, |x, y| {
            image::Rgba([
                if x < 256 { 245 } else { 30 },
                if y < 256 { 45 } else { 225 },
                70,
                255,
            ])
        });
        unsafe {
            r.set_artwork(1, Some(&cover), &config.artwork).unwrap();
        }
        for (w, h) in [(2560, 1440), (3840, 2160), (2560, 1080)] {
            // Neutral logo fallback replaces Thriller even before any history exists.
            unsafe {
                r.set_previous_artwork(10, None).unwrap();
            }
            let neutral = unsafe { capture(&mut r, &area, w, h, 0.0) };
            let viewport = fitted_viewport(w, h, 16.0 / 9.0);
            let [sx, sy, sw, sh] = cache::region(normalized, viewport);
            let top = h - sy - sh;
            let roi = |image: &image::RgbaImage| {
                image::imageops::crop_imm(image, sx as u32, top as u32, sw as u32, sh as u32)
                    .to_image()
            };
            unsafe {
                r.set_previous_artwork(11, Some(&cover)).unwrap();
            }
            let first = unsafe { capture(&mut r, &area, w, h, 0.0) };
            assert!(
                roi(&neutral) != roi(&first),
                "previous image was not displayed"
            );
            let baseline = roi(&first);
            let count = r.source.stack_updates();
            // Every authored pose and three fractional intermediates per pair.
            for step in 0..=128 {
                r.set_artwork_angle(step as f32 * 0.19);
                let current = unsafe { capture(&mut r, &area, w, h, step as f32 / 128.0) };
                assert!(
                    roi(&current) == baseline,
                    "{variant:?} {w}x{h} stack changed at {step}"
                );
                assert_eq!(
                    r.source.stack_updates(),
                    count,
                    "static stack was recomposed"
                );
            }
            // The loader resets history on stop: steel holder + neutral stack.
            r.set_sleeve_visible(false);
            unsafe {
                r.set_artwork(2, None, &config.artwork).unwrap();
                r.set_previous_artwork(12, None).unwrap();
            }
            let stopped = unsafe { capture(&mut r, &area, w, h, 0.5) };
            if w == 2560 && h == 1440 {
                stopped
                    .save(output.join(format!("{}-idle.png", variant.id())))
                    .unwrap();
            }
            assert!(
                roi(&stopped) == roi(&neutral),
                "{variant:?} {w}x{h} stop retained the previous cover"
            );
            let stopped_count = r.source.stack_updates();
            unsafe {
                r.set_previous_artwork(12, None).unwrap();
            }
            let idle = unsafe { capture(&mut r, &area, w, h, 0.5) };
            assert_eq!(idle, stopped, "idle frame changed");
            assert_eq!(
                r.source.stack_updates(),
                stopped_count,
                "idle stack redrawn"
            );
            r.set_sleeve_visible(true);
            unsafe {
                r.set_artwork(3, Some(&cover), &config.artwork).unwrap();
            }
            let reopened = unsafe { capture(&mut r, &area, w, h, 0.5) };
            assert!(
                roi(&reopened) == roi(&neutral),
                "reopen restored old stack art"
            );
            // A subsequent song switch supplies a new previous cover.
            unsafe {
                r.set_previous_artwork(13, Some(&cover)).unwrap();
            }
            let advanced = unsafe { capture(&mut r, &area, w, h, 0.5) };
            assert!(
                roi(&advanced) == baseline,
                "new history did not restore stack art"
            );
            // Scene-switch equivalent: a fresh renderer receives both snapshots.
            let mut fresh = unsafe { Renderer::new(PreviewAsset::load(&config).unwrap()).unwrap() };
            unsafe {
                fresh.set_artwork(3, Some(&cover), &config.artwork).unwrap();
                fresh.set_previous_artwork(13, Some(&cover)).unwrap();
            }
            let rebuilt = unsafe { capture(&mut fresh, &area, w, h, 0.0) };
            assert!(
                roi(&rebuilt) == baseline,
                "{variant:?} {w}x{h} rebuild changed the previous cover"
            );
            if w == 2560 && h == 1440 {
                rebuilt
                    .save(output.join(format!("{}-previous.png", variant.id())))
                    .unwrap();
                neutral
                    .save(output.join(format!("{}-neutral.png", variant.id())))
                    .unwrap();
                // Compare white/black runtime calibration to actual Blender renders.
                if let Some(bake) = std::env::var_os("TALIS_STACK_BAKE_DIR") {
                    for (i, value, name) in [(0, 255, "white"), (1, 0, "black")] {
                        let solid = image::RgbaImage::from_pixel(
                            32,
                            32,
                            image::Rgba([value, value, value, 255]),
                        );
                        unsafe {
                            fresh.set_previous_artwork(20 + i, Some(&solid)).unwrap();
                        }
                        let actual = unsafe { capture(&mut fresh, &area, w, h, 0.0) };
                        let reference = image::open(
                            PathBuf::from(&bake)
                                .join(variant.id())
                                .join("work")
                                .join(format!("{name}.png")),
                        )
                        .unwrap()
                        .to_rgba8();
                        let mut differences = Vec::new();
                        let illumination = &stack_pixels(variant);
                        for y in 4..bounds[3] - 4 {
                            for x in 4..bounds[2] - 4 {
                                let offset = ((y * bounds[2] + x) * 8 + 6) as usize;
                                if u16::from_le_bytes([
                                    illumination[offset],
                                    illumination[offset + 1],
                                ]) != 0x3c00
                                {
                                    continue;
                                }
                                let a = actual.get_pixel(bounds[0] + x, bounds[1] + y);
                                let b = reference.get_pixel(x, y);
                                differences
                                    .push((0..3).map(|c| a[c].abs_diff(b[c])).max().unwrap());
                            }
                        }
                        differences.sort_unstable();
                        assert!(differences.len() > 10000);
                        let p99 = differences[(differences.len() - 1) * 99 / 100];
                        eprintln!(
                            "{variant:?} stack {name}: p99={p99}/255 max={}",
                            differences.last().unwrap()
                        );
                        assert!(p99 <= 4, "stack paper transfer disagrees with Blender");
                    }
                }
            }
            unsafe {
                fresh.destroy();
            }
        }
        unsafe {
            r.destroy();
        }
    }
    eprintln!("Stack QA captures: {}", output.display());
}
fn stack_pixels(variant: SceneVariant) -> Vec<u8> {
    let root = if variant == SceneVariant::Day {
        "assets/scenes/1440p/day"
    } else {
        "assets/scenes/1440p/night"
    };
    std::fs::read(format!("{root}/stack-sleeve/illumination.rgba16f")).unwrap()
}

#[test]
#[ignore = "requires owned GTK/OpenGL context and both scene packages"]
fn trimmed_arm_preserves_motion_and_has_only_tiny_frozen_lamp_side_differences() {
    let (_window, area) = arm_cadence_tests::owned_area();
    for variant in [SceneVariant::Day, SceneVariant::Night] {
        let mut config = Config::default();
        config.scene.variant = variant;
        config.scene.mode = PreviewMode::Arm;
        config.render.arm_interpolation = true;
        let mut r = unsafe { Renderer::new(PreviewAsset::load(&config).unwrap()).unwrap() };
        let original = [
            1160.0 / 2560.0,
            585.0 / 1440.0,
            960.0 / 2560.0,
            630.0 / 1440.0,
        ];
        let trimmed = [
            1160.0 / 2560.0,
            585.0 / 1440.0,
            600.0 / 2560.0,
            630.0 / 1440.0,
        ];
        for p in [
            0.0, 0.0078125, 0.25, 0.5078125, 0.75, 0.9921875, 1.0, 0.1, 0.9,
        ] {
            unsafe {
                r.source.test_arm_region(original);
            }
            let full = unsafe { capture(&mut r, &area, 2560, 1440, p) };
            unsafe {
                r.source.test_arm_region(trimmed);
            }
            let local = unsafe { capture(&mut r, &area, 2560, 1440, p) };
            let mut error = 0;
            for y in 585..1215 {
                for x in 1160..2120 {
                    let a = full.get_pixel(x, y);
                    let b = local.get_pixel(x, y);
                    let d = (0..3).map(|c| a[c].abs_diff(b[c])).max().unwrap();
                    if x < 1744 {
                        assert_eq!(d, 0, "moving area changed at {x},{y} progress {p}");
                    } else {
                        error = error.max(d);
                    }
                }
            }
            eprintln!("{variant:?} trimmed region progress {p}: excluded/seam max={error}/255");
            assert!(
                error <= 10,
                "unexpected moving shadow beyond chosen boundary"
            );
        }
        unsafe {
            r.destroy();
        }
    }
}
