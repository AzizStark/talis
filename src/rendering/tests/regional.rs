//! Owned/unmapped GPU comparisons. No music player or desktop manipulation.
use super::super::*;
use super::arm_cadence_tests::{owned_area, pixels};
use crate::config::{Config, PreviewMode, SceneVariant};
use gtk::prelude::*;
use std::time::{Duration, Instant};

unsafe fn target(
    gl: &glow::Context,
    w: i32,
    h: i32,
) -> (glow::NativeFramebuffer, glow::NativeTexture) {
    unsafe {
        let texture = gl.create_texture().unwrap();
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            w,
            h,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(None),
        );
        let framebuffer = gl.create_framebuffer().unwrap();
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
        gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(texture),
            0,
        );
        assert_eq!(
            gl.check_framebuffer_status(glow::FRAMEBUFFER),
            glow::FRAMEBUFFER_COMPLETE
        );
        (framebuffer, texture)
    }
}

#[test]
#[ignore = "requires owned unmapped GTK/OpenGL and day package"]
fn background_before_first_animation_does_not_use_an_incomplete_cache() {
    let (window, area) = owned_area();
    area.allocate(320, 180, -1, None);
    area.make_current();
    area.attach_buffers();
    let mut config = Config::default();
    config.scene.mode = PreviewMode::Arm;
    config.render.arm_interpolation = true;
    let mut renderer = unsafe { Renderer::new(PreviewAsset::load(&config).unwrap()).unwrap() };
    unsafe {
        renderer.check_error().unwrap();
        let draw = cache::framebuffer(&renderer.gl, glow::DRAW_FRAMEBUFFER_BINDING);
        let read = cache::framebuffer(&renderer.gl, glow::READ_FRAMEBUFFER_BINDING);
        // Separate native roots can snapshot the background before the first
        // animated frame has allocated and composed the retained framebuffer.
        renderer.present_background();
        let initial_error = renderer.check_error();
        let initial_pixels = pixels(&renderer, 320, 180);
        renderer.present_background();
        renderer.check_error().unwrap();
        assert_eq!(
            cache::framebuffer(&renderer.gl, glow::DRAW_FRAMEBUFFER_BINDING),
            draw
        );
        assert_eq!(
            cache::framebuffer(&renderer.gl, glow::READ_FRAMEBUFFER_BINDING),
            read
        );
        renderer.render(320, 180, 0.5).unwrap();
        let expected = pixels(&renderer, 320, 180);
        renderer.gl.clear_color(0.0, 0.0, 0.0, 0.0);
        renderer.gl.clear(glow::COLOR_BUFFER_BIT);
        renderer.present_background();
        let actual = pixels(&renderer, 320, 180);
        renderer.check_error().unwrap();
        renderer.destroy();
        window.destroy();
        initial_error.expect("background rendered before animation must be safe");
        assert!(initial_pixels.chunks_exact(4).all(|p| p == [0, 0, 0, 255]));
        assert_eq!(actual, expected);
    }
}

#[test]
#[ignore = "requires owned unmapped GTK/OpenGL and day/night packages"]
fn regional_pixels_match_full_output_and_static_background_is_reused() {
    let (window, area) = owned_area();
    let now = Instant::now();
    let cover =
        image::RgbaImage::from_fn(256, 256, |x, y| image::Rgba([x as u8, y as u8, 80, 255]));
    for variant in [SceneVariant::Day, SceneVariant::Night] {
        let mut config = Config::default();
        config.scene.variant = variant;
        config.scene.mode = PreviewMode::Arm;
        config.render.arm_interpolation = true;
        let mut exact = unsafe { Renderer::new(PreviewAsset::load(&config).unwrap()).unwrap() };
        let mut regional = unsafe { Renderer::new(PreviewAsset::load(&config).unwrap()).unwrap() };
        for (w, h, scale) in [(2560, 1440, 1), (3840, 2160, 2), (2560, 1080, 1)] {
            area.allocate(w, h, -1, None);
            area.make_current();
            area.attach_buffers();
            let full = unsafe { cache::framebuffer(&exact.gl, glow::DRAW_FRAMEBUFFER_BINDING) };
            for r in [&mut exact, &mut regional] {
                unsafe {
                    r.set_artwork(1, Some(&cover), &config.artwork).unwrap();
                    r.set_previous_artwork(0, None).unwrap();
                }
            }
            let rect = regional.regional_rect(w, h, scale).unwrap();
            let [x, y, rw, rh] = rect;
            assert!(
                rw * rh < w * h / 2,
                "animation bounds cover too much of the screen"
            );
            assert!(rect.iter().all(|v| v % scale == 0));
            let (patch, texture) = unsafe { target(&regional.gl, rw, rh) };
            let mut background = Vec::new();
            for step in 0..24 {
                let progress = match step {
                    0..=8 => 0.51 + step as f32 / 5400.0,
                    9..=17 => (step - 9) as f32 / 8.0,
                    _ => 1.0 - (step - 18) as f32 / 5.0,
                };
                let timestamp = now + Duration::from_secs_f64(step as f64 / 30.0);
                for r in [&mut exact, &mut regional] {
                    r.set_arm_cache_target((step <= 8).then_some(progress));
                    r.set_artwork_angle(step as f32 * 0.116);
                    if step == 3 {
                        r.set_button_progress([0.5, 0.25, 0.75]);
                        r.set_knob_angle(0.6);
                    }
                    if step == 5 {
                        unsafe {
                            r.set_previous_artwork(1, Some(&cover)).unwrap();
                        }
                    }
                    if step == 6 {
                        r.set_sleeve_visible(false);
                    }
                    if step == 7 {
                        r.set_sleeve_visible(true);
                    }
                    if step == 8 {
                        unsafe {
                            r.set_artwork(2, None, &config.artwork).unwrap();
                        }
                    }
                    if step == 9 {
                        unsafe {
                            r.set_artwork(3, Some(&cover), &config.artwork).unwrap();
                        }
                    }
                }
                unsafe {
                    exact.gl.bind_framebuffer(glow::FRAMEBUFFER, full);
                    exact.source.render_at(w, h, progress, timestamp).unwrap();
                }
                let reference = unsafe { pixels(&exact, w, h) };
                unsafe {
                    regional.gl.bind_framebuffer(glow::FRAMEBUFFER, Some(patch));
                }
                let changed = unsafe {
                    regional
                        .source
                        .render_target_at(w, h, progress, timestamp, Some(rect))
                        .unwrap()
                };
                let patch_pixels = unsafe { pixels(&regional, rw, rh) };
                assert_eq!(
                    changed,
                    matches!(step, 0 | 5 | 6 | 7 | 8 | 9),
                    "unexpected static refresh {variant:?} step {step}"
                );
                if changed {
                    unsafe {
                        regional.gl.bind_framebuffer(glow::FRAMEBUFFER, full);
                        regional.present_background();
                    }
                    background = unsafe { pixels(&regional, w, h) };
                }
                let mut actual = background.clone();
                for row in 0..rh as usize {
                    let dst = ((y as usize + row) * w as usize + x as usize) * 4;
                    let src = row * rw as usize * 4;
                    actual[dst..dst + rw as usize * 4]
                        .copy_from_slice(&patch_pixels[src..src + rw as usize * 4]);
                }
                let differences = reference
                    .iter()
                    .zip(&actual)
                    .filter(|(a, b)| a != b)
                    .count();
                if differences != 0 {
                    let mut bounds = [w, h, 0, 0];
                    let mut maximum = 0;
                    for (i, (a, b)) in reference.iter().zip(&actual).enumerate() {
                        if a != b {
                            let x = (i / 4) as i32 % w;
                            let y = (i / 4) as i32 / w;
                            bounds = [
                                bounds[0].min(x),
                                bounds[1].min(y),
                                bounds[2].max(x),
                                bounds[3].max(y),
                            ];
                            maximum = maximum.max(a.abs_diff(*b));
                        }
                    }
                    eprintln!(
                        "Difference bbox {bounds:?}, max channel delta {maximum}, region {rect:?}"
                    );
                }
                assert_eq!(
                    differences, 0,
                    "{variant:?} {w}x{h} step {step}: regional seams/pixels changed"
                );
                assert_eq!(exact.cache_update_counts(), regional.cache_update_counts());
                assert_eq!(exact.button_update_count(), regional.button_update_count());
                assert_eq!(exact.knob_update_count(), regional.knob_update_count());
            }
            eprintln!(
                "{variant:?} {w}x{h} region {rect:?}: 24 frames byte-exact, static background reused"
            );
            unsafe {
                regional.gl.bind_framebuffer(glow::FRAMEBUFFER, full);
                regional.gl.delete_framebuffer(patch);
                regional.gl.delete_texture(texture);
                regional.check_error().unwrap();
            }
        }
        unsafe {
            exact.destroy();
            regional.destroy();
        }
    }
    window.destroy();
}

#[test]
#[ignore = "paced owned/unmapped GPU benchmark, not a CI speed assertion"]
fn regional_presentation_paced_abba_benchmark() {
    let (window, area) = owned_area();
    let artwork =
        image::RgbaImage::from_fn(256, 256, |x, y| image::Rgba([x as u8, y as u8, 80, 255]));
    for variant in [SceneVariant::Day, SceneVariant::Night] {
        let mut config = Config::default();
        config.scene.variant = variant;
        config.scene.mode = PreviewMode::Arm;
        config.render.arm_interpolation = true;
        let mut r = unsafe { Renderer::new(PreviewAsset::load(&config).unwrap()).unwrap() };
        unsafe {
            r.set_artwork(1, Some(&artwork), &config.artwork).unwrap();
        }
        for (w, h) in [(2560, 1440), (3840, 2160)] {
            area.allocate(w, h, -1, None);
            area.make_current();
            area.attach_buffers();
            let full = unsafe { cache::framebuffer(&r.gl, glow::DRAW_FRAMEBUFFER_BINDING) };
            let rect = r.regional_rect(w, h, 1).unwrap();
            let (patch, texture) = unsafe { target(&r.gl, rect[2], rect[3]) };
            for (run, regional) in [false, true, true, false].into_iter().enumerate() {
                unsafe {
                    r.enable_gpu_timing(true);
                }
                let started = Instant::now();
                let mut samples = Vec::new();
                for frame in 0..180 {
                    let deadline = started + Duration::from_secs_f64(frame as f64 / 30.0);
                    std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
                    let now = Instant::now();
                    let p = 0.51 + frame as f32 / 5400.0;
                    r.set_arm_cache_target(Some(p));
                    r.set_artwork_angle(frame as f32 * 0.116355);
                    unsafe {
                        r.gl.bind_framebuffer(
                            glow::FRAMEBUFFER,
                            if regional { Some(patch) } else { full },
                        );
                        let changed = r
                            .source
                            .render_target_at(w, h, p, now, regional.then_some(rect))
                            .unwrap();
                        if regional && changed {
                            r.gl.bind_framebuffer(glow::FRAMEBUFFER, full);
                            r.present_background();
                        }
                        r.gl.flush();
                    }
                    samples.extend(r.take_gpu_timings().into_iter().filter(|s| s.info.id > 60));
                }
                unsafe {
                    r.gl.finish();
                }
                r.source.profiler.as_mut().unwrap().poll();
                samples.extend(r.take_gpu_timings().into_iter().filter(|s| s.info.id > 60));
                assert_eq!(samples.len(), 120);
                let summary = |values: Vec<f64>| {
                    let avg = values.iter().sum::<f64>() / values.len() as f64;
                    let mut sorted = values;
                    sorted.sort_by(f64::total_cmp);
                    (
                        avg,
                        sorted[((sorted.len() - 1) as f64 * 0.95).ceil() as usize],
                    )
                };
                let (total, p95) = summary(samples.iter().map(|s| s.total_ms).collect());
                let (copy, copy_p95) = summary(
                    samples
                        .iter()
                        .map(|s| s.stages_ms[GpuStage::Presentation as usize].unwrap())
                        .collect(),
                );
                eprintln!(
                    "REGIONAL_ABBA {variant:?} {w}x{h} run={run} regional={regional} samples=120 region={rect:?} total_avg_ms={total:.6} total_p95_ms={p95:.6} copy_avg_ms={copy:.6} copy_p95_ms={copy_p95:.6}"
                );
            }
            unsafe {
                r.gl.bind_framebuffer(glow::FRAMEBUFFER, full);
                r.gl.delete_framebuffer(patch);
                r.gl.delete_texture(texture);
                r.check_error().unwrap();
            }
        }
        unsafe {
            r.destroy();
        }
    }
    window.destroy();
}
