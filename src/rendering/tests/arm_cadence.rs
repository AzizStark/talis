//! Owned, unmapped GLArea checks; never controls real playback or desktop windows.
use super::super::*;
use crate::config::{Config, PreviewMode, SceneVariant};
use gtk::prelude::*;
use std::time::{Duration, Instant};

pub(super) unsafe fn pixels(r: &Renderer, w: i32, h: i32) -> Vec<u8> {
    let mut pixels = vec![0; w as usize * h as usize * 4];
    unsafe {
        r.gl.read_pixels(
            0,
            0,
            w,
            h,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut pixels)),
        );
    }
    pixels
}

pub(super) fn owned_area() -> (gtk::Window, gtk::GLArea) {
    gtk::init().unwrap();
    let window = gtk::Window::new();
    let area = gtk::GLArea::new();
    area.set_required_version(3, 3);
    area.set_auto_render(false);
    window.set_child(Some(&area));
    gtk::prelude::WidgetExt::realize(&window);
    area.realize();
    area.make_current();
    assert!(!window.is_mapped());
    assert!(area.error().is_none(), "{:?}", area.error());
    (window, area)
}

#[test]
#[ignore = "requires an owned GTK/OpenGL context and day/night packages"]
fn arm_cadence_preserves_retained_pixels_overlap_occlusion_and_exact_rest() {
    let (_window, area) = owned_area();
    let cover = image::RgbaImage::from_fn(256, 256, |x, y| {
        image::Rgba([
            x as u8,
            y as u8,
            if (x / 8 + y / 8) % 2 == 0 { 240 } else { 20 },
            255,
        ])
    });
    for variant in [SceneVariant::Day, SceneVariant::Night] {
        let mut config = Config::default();
        config.scene.variant = variant;
        config.scene.mode = PreviewMode::Arm;
        config.render.arm_interpolation = true;
        area.make_current();
        let assets = PreviewAsset::load(&config).unwrap();
        let (dimensions, bounds) = match &assets.scene {
            crate::preview::PreviewScene::Arm(arm) => (
                (arm.manifest.width, arm.manifest.height),
                arm.flow.as_ref().unwrap().max_displacements.clone(),
            ),
            _ => panic!("Expected interpolated arm assets"),
        };
        let mut r = unsafe { Renderer::new(assets).unwrap() };
        unsafe {
            r.set_artwork(1, Some(&cover), &config.artwork).unwrap();
        }
        for (w, h) in [(1920, 1080), (3440, 1440), (3840, 2160)] {
            area.allocate(w, h, -1, None);
            area.make_current();
            area.attach_buffers();
            let mut maximum = 0_u8;
            let mut error = 0_u64;
            let mut changed = 0_u64;
            for (pair, &bound) in bounds.iter().enumerate() {
                let p = (pair as f32 + 0.5) / bounds.len() as f32;
                let now = Instant::now();
                let viewport = fitted_viewport(w, h, r.scene_aspect());
                let scale = (f64::from(viewport.2) / f64::from(dimensions.0))
                    .max(f64::from(viewport.3) / f64::from(dimensions.1));
                // Exercise almost the full hold budget in EVERY flow interval,
                // not only one tiny request in the middle of a scene.
                let request = p
                    + (0.124 / (f64::from(bound.max(1.0)) * bounds.len() as f64 * scale * 2.0))
                        as f32;
                r.set_arm_cache_target(None);
                unsafe {
                    r.source.render_at(w, h, p, now).unwrap();
                }
                let reference = unsafe { pixels(&r, w, h) };
                let counts = r.cache_update_counts();
                r.set_arm_cache_target(Some(request));
                unsafe {
                    r.source
                        .render_at(w, h, request, now + Duration::from_millis(33))
                        .unwrap();
                }
                assert_eq!(
                    r.cache_update_counts(),
                    counts,
                    "subpixel arm wasn't reused"
                );
                let held = unsafe { pixels(&r, w, h) };
                assert!(
                    held == reference,
                    "retained RGB/mask/record changed at a fixed angle"
                );
                r.set_arm_cache_target(None);
                unsafe {
                    r.source
                        .render_at(w, h, request, now + Duration::from_millis(34))
                        .unwrap();
                }
                assert_eq!(r.cache_update_counts().1, counts.1 + 1);
                let exact = unsafe { pixels(&r, w, h) };
                for (a, b) in held.iter().zip(&exact) {
                    let delta = a.abs_diff(*b);
                    maximum = maximum.max(delta);
                    error += u64::from(delta);
                    changed += u64::from(delta != 0);
                }
                // Compare dirty overlapping patches to a fresh full composition
                // at the SAME retained pose, including foreground occlusion.
                unsafe {
                    r.source.render_at(w, h, p, now).unwrap();
                }
                r.set_arm_cache_target(Some(request));
                let mut settings = config.artwork.clone();
                settings.sleeve_shadow_lift = 0.3;
                unsafe {
                    r.set_artwork(1, Some(&cover), &settings).unwrap();
                }
                r.set_knob_angle(0.3);
                r.set_button_progress([0.5, 0.25, 0.75]);
                let counts = r.cache_update_counts();
                unsafe {
                    r.source
                        .render_at(w, h, request, now + Duration::from_millis(33))
                        .unwrap();
                }
                assert_eq!(r.cache_update_counts().1, counts.1);
                let partial = unsafe { pixels(&r, w, h) };
                area.allocate(w + 8, h + 8, -1, None);
                area.attach_buffers();
                unsafe {
                    r.source.render_at(w + 8, h + 8, p, now).unwrap();
                }
                area.allocate(w, h, -1, None);
                area.attach_buffers();
                unsafe {
                    r.source.render_at(w, h, p, now).unwrap();
                }
                assert!(
                    partial == unsafe { pixels(&r, w, h) },
                    "dirty patches used a different arm pose"
                );
                unsafe {
                    r.set_artwork(1, Some(&cover), &config.artwork).unwrap();
                }
                r.set_button_progress([0.0; 3]);
                r.set_knob_angle(0.0);
            }
            let mean = error as f64 / (f64::from(w) * f64::from(h) * 4.0 * bounds.len() as f64);
            eprintln!(
                "{variant:?} {w}x{h}: held-vs-exact max {maximum}/255, changed channels {changed}, mean channel error {mean}"
            );
            // A geometric hold affects sharp edge values; do not blur them.
            // Reject broad/large changes rather than requiring identical motion.
            assert!(maximum <= 16, "unexpected nonlinear flow discontinuity");
            assert!(mean < 0.001);
            r.set_arm_cache_target(Some(0.9));
            let counts = r.cache_update_counts();
            unsafe {
                r.source.render_at(w, h, 0.9211, Instant::now()).unwrap();
            }
            assert!(r.cache_update_counts().1 > counts.1);
            unsafe {
                r.check_error().unwrap();
            }
        }
        unsafe {
            r.destroy();
        }
    }
}

#[test]
#[ignore = "requires owned GTK/OpenGL; diagnostic timings are not CI speed assertions"]
fn arm_cadence_gpu_comparison() {
    let (_window, area) = owned_area();
    for variant in [SceneVariant::Day, SceneVariant::Night] {
        let mut config = Config::default();
        config.scene.variant = variant;
        config.scene.mode = PreviewMode::Arm;
        config.render.arm_interpolation = true;
        area.make_current();
        let mut r = unsafe { Renderer::new(PreviewAsset::load(&config).unwrap()).unwrap() };
        let cover =
            image::RgbaImage::from_fn(256, 256, |x, y| image::Rgba([x as u8, y as u8, 80, 255]));
        unsafe {
            r.set_artwork(1, Some(&cover), &config.artwork).unwrap();
        }
        for (w, h) in [(1920, 1080), (2560, 1440), (3840, 2160)] {
            area.allocate(w, h, -1, None);
            area.attach_buffers();
            // ABBA order reduces (not eliminates) clock/thermal-order bias.
            for reuse in [false, true, true, false] {
                let now = Instant::now();
                r.set_arm_cache_target(None);
                // Warm shaders/resources before timing. Do not lock hardware
                // clocks or claim this measures the desktop compositor.
                for frame in 0..24 {
                    unsafe {
                        r.source
                            .render_at(w, h, 0.51 + frame as f32 * 0.0002, now)
                            .unwrap();
                    }
                }
                unsafe {
                    r.source.render_at(w, h, 0.51, now).unwrap();
                    r.gl.finish();
                }
                let counts = r.cache_update_counts();
                let record_draws = r.source.test_record_draws();
                let query = unsafe { r.gl.create_query().unwrap() };
                let started = Instant::now();
                unsafe {
                    r.gl.begin_query(glow::TIME_ELAPSED, query);
                }
                for frame in 1..=90 {
                    let p = 0.51 + frame as f32 / (30.0 * 180.0);
                    r.set_arm_cache_target(reuse.then_some(p));
                    r.set_artwork_angle(frame as f32 * 0.116);
                    unsafe {
                        r.source
                            .render_at(
                                w,
                                h,
                                p,
                                now + Duration::from_secs_f64(f64::from(frame) / 30.0),
                            )
                            .unwrap();
                    }
                }
                unsafe {
                    r.gl.end_query(glow::TIME_ELAPSED);
                    r.gl.finish();
                }
                let ms = unsafe { r.gl.get_query_parameter_u32(query, glow::QUERY_RESULT) } as f64
                    / 1_000_000.0;
                unsafe {
                    r.gl.delete_query(query);
                    r.check_error().unwrap();
                }
                let updates = r.cache_update_counts().1 - counts.1;
                assert_eq!(r.cache_update_counts().0, counts.0);
                assert_eq!(
                    r.source.test_record_draws() - record_draws,
                    90,
                    "rotation lost frames"
                );
                if reuse {
                    assert!(updates <= 45);
                } else {
                    assert_eq!(updates, 90);
                }
                eprintln!(
                    "CADENCE_GPU {variant:?} {w}x{h} reuse={reuse} frames=90 arm_updates={updates} total_gpu_ms={ms:.3} gpu_ms_per_frame={:.3} wall_ms={:.3}",
                    ms / 90.0,
                    started.elapsed().as_secs_f64() * 1000.0
                );
            }
        }
        unsafe {
            r.destroy();
        }
    }
}
