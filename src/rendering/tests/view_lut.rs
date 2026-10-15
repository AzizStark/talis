//! Explicit owned/unmapped GL checks; no live desktop or player control.
use super::super::*;
use super::arm_cadence_tests::{owned_area, pixels};
use crate::config::{Config, PreviewMode, SceneVariant};
use gtk::prelude::*;
use sha2::Digest;
use std::collections::HashSet;

#[test]
#[ignore = "requires owned GTK/OpenGL context and day/night scene packages"]
fn shared_view_luts_preserve_pixels_and_scene_ownership() {
    let (window, area) = owned_area();
    let cover = image::RgbaImage::from_fn(256, 256, |x, y| {
        image::Rgba([
            x as u8,
            y as u8,
            if (x / 8 + y / 8) % 2 == 0 { 230 } else { 15 },
            255,
        ])
    });
    let previous = image::RgbaImage::from_fn(64, 64, |x, y| {
        image::Rgba([
            210,
            (x * 4) as u8,
            (y * 4) as u8,
            if x < 32 { 255 } else { 128 },
        ])
    });
    let mut day_key = None;
    for variant in [SceneVariant::Day, SceneVariant::Night] {
        let mut config = Config::default();
        config.scene.variant = variant;
        config.scene.mode = PreviewMode::Arm;
        config.render.arm_interpolation = true;
        let load = || PreviewAsset::load(&config).unwrap();
        let assets = load();
        let responses = [
            assets.stack_sleeve.as_ref().unwrap(),
            assets.sleeve.as_ref().unwrap(),
            assets.record_lighting.as_ref().unwrap(),
            &assets.vinyl_detail.as_ref().unwrap().response,
        ];
        for response in responses {
            assert_eq!(response.manifest.lut_edge, 96);
            assert_eq!(response.view_lut, responses[0].view_lut);
        }
        let digest = sha2::Sha256::digest(&responses[0].view_lut);
        if let Some(day) = day_key {
            assert_ne!(digest, day, "day and night must not be conflated");
        } else {
            day_key = Some(digest);
        }
        let mut shared = unsafe { Renderer::new(assets).unwrap() };
        let mut original = unsafe { Renderer::new_impl(load(), false).unwrap() };
        let shared_views = shared.source.test_views();
        let old_views = original.source.test_views();
        assert_eq!(shared_views.len(), 4);
        assert_eq!(old_views.len(), 4);
        assert_eq!(
            shared_views.iter().copied().collect::<HashSet<_>>().len(),
            1
        );
        assert_eq!(old_views.iter().copied().collect::<HashSet<_>>().len(), 4);
        assert!(
            !old_views.contains(&shared_views[0]),
            "renderers must own separate tables"
        );
        eprintln!(
            "VIEW_LUT_QA {variant:?}: four responses, 4 -> 1 texture, 10.125 MiB payload saved; {}",
            unsafe { shared.gl.get_parameter_string(glow::RENDERER) }
        );
        for (w, h) in [(2560, 1440), (3840, 2160), (1919, 1079)] {
            area.allocate(w, h, -1, None);
            area.make_current();
            area.attach_buffers();
            for step in 0..12 {
                let mut settings = config.artwork.clone();
                settings.size_percent = if step == 3 { 100.0 } else { 35.0 };
                settings.sleeve_shadow_lift = [0.0, 0.25, 1.0][step % 3];
                settings.vinyl_opacity = [0.0, 0.4, 1.0][step % 3];
                settings.vinyl_brightness = [0.0, 1.0, 2.0][step % 3];
                settings.vinyl_rotation = step != 8;
                for r in [&mut shared, &mut original] {
                    r.set_sleeve_visible(step != 6);
                    r.set_artwork_angle(step as f32 * 0.73);
                    r.set_button_progress([0.0, (step % 3) as f32 / 2.0, 1.0]);
                    r.set_knob_angle(step as f32 * 0.1);
                    unsafe {
                        r.set_artwork(step as u64, (step != 6).then_some(&cover), &settings)
                            .unwrap();
                        r.set_previous_artwork(step as u64, (step % 4 != 0).then_some(&previous))
                            .unwrap();
                    }
                }
                let progress = [0.0, 0.015625, 0.421875, 0.99, 1.0, 0.5][step % 6];
                unsafe {
                    original.render(w, h, progress).unwrap();
                    original.check_error().unwrap();
                }
                let reference = unsafe { pixels(&original, w, h) };
                unsafe {
                    shared.render(w, h, progress).unwrap();
                    shared.check_error().unwrap();
                }
                assert!(
                    reference == unsafe { pixels(&shared, w, h) },
                    "{variant:?} {w}x{h} step {step}: sharing changed pixels"
                );
                assert_eq!(shared.cache_update_counts(), original.cache_update_counts());
            }
        }
        // Dropping one renderer cannot invalidate the other, even during scene
        // switching's simultaneous old/new allocation and rollback lifetime.
        let gl = shared.gl.clone();
        // Fail late, after all paper responses acquired their LUT. Replacement
        // construction must roll back without touching the retained renderer.
        let mut broken = load();
        broken.buttons.as_mut().unwrap().manifest.crop[2] =
            unsafe { gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE) } as u32 + 1;
        let failure = match unsafe { Renderer::new(broken) } {
            Ok(value) => {
                unsafe {
                    value.destroy();
                }
                panic!("oversized replacement succeeded");
            }
            Err(error) => error,
        };
        assert!(failure.to_string().contains("Button atlas exceeds"));
        let mut invalid_lighting = load().record_lighting.unwrap();
        invalid_lighting.view_lut.pop();
        assert!(unsafe { shared.source.upload_record_lighting(invalid_lighting) }.is_err());
        assert_eq!(shared.source.test_views(), shared_views);
        unsafe {
            original.render(1919, 1079, 0.8).unwrap();
        }
        let rollback_reference = unsafe { pixels(&original, 1919, 1079) };
        unsafe {
            shared.render(1919, 1079, 0.8).unwrap();
            shared.check_error().unwrap();
        }
        assert_eq!(rollback_reference, unsafe { pixels(&shared, 1919, 1079) });
        let mut next_config = config.clone();
        next_config.scene.variant = match variant {
            SceneVariant::Day => SceneVariant::Night,
            SceneVariant::Night => SceneVariant::Day,
        };
        let mut next = unsafe { Renderer::new(PreviewAsset::load(&next_config).unwrap()).unwrap() };
        let next_views = next.source.test_views();
        assert_eq!(next_views.iter().copied().collect::<HashSet<_>>().len(), 1);
        assert_ne!(next_views[0], shared_views[0]);
        unsafe {
            original.destroy();
        }
        for texture in old_views {
            assert!(!unsafe { gl.is_texture(texture) });
        }
        assert!(unsafe { gl.is_texture(shared_views[0]) });
        unsafe {
            shared.render(1919, 1079, 0.8).unwrap();
            shared.check_error().unwrap();
            shared.destroy();
        }
        assert!(
            !unsafe { gl.is_texture(shared_views[0]) },
            "last renderer release leaked the shared LUT"
        );
        assert!(unsafe { gl.is_texture(next_views[0]) });
        unsafe {
            next.render(1919, 1079, 0.4).unwrap();
            next.check_error().unwrap();
            next.destroy();
        }
        assert!(!unsafe { gl.is_texture(next_views[0]) });
    }
    window.destroy();
}

#[test]
#[ignore = "requires owned GTK/OpenGL context"]
fn view_lut_pool_releases_last_owner_and_never_merges_different_content() {
    let (window, _area) = owned_area();
    let mut config = Config::default();
    config.scene.mode = PreviewMode::Beauty;
    let r = unsafe { Renderer::new(PreviewAsset::load(&config).unwrap()).unwrap() };
    let gl = r.gl.clone();
    let mut pool = view_lut::ViewLutPool::new(gl.clone());
    let a = vec![7; 2 * 2 * 2 * 4];
    let mut b = a.clone();
    b[12] = 8;
    unsafe {
        let first = pool.acquire(2, &a, 7).unwrap();
        let second = pool.acquire(2, &a, 10).unwrap();
        assert!(std::rc::Rc::ptr_eq(&first, &second));
        let texture = first.texture;
        let different = pool.acquire(2, &b, 7).unwrap();
        assert_ne!(different.texture, texture);
        let different_size = pool.acquire(3, &[7; 3 * 3 * 3 * 4], 10).unwrap();
        assert_ne!(different_size.texture, texture);
        assert!(pool.acquire(2, &[0; 31], 7).is_err());
        drop(first);
        assert!(gl.is_texture(texture));
        drop(second);
        assert!(
            !gl.is_texture(texture),
            "weak cache retained an unused table"
        );
        let fresh = pool.acquire(2, &a, 10).unwrap();
        assert!(gl.is_texture(fresh.texture));
        let mut independent = view_lut::ViewLutPool::new(gl.clone());
        let separate = independent.acquire(2, &a, 7).unwrap();
        assert_ne!(fresh.texture, separate.texture);
        let owned = fresh.texture;
        drop(pool);
        assert!(
            gl.is_texture(owned),
            "dropping the cache freed a live response"
        );
        drop(fresh);
        assert!(!gl.is_texture(owned));
        drop(different);
        drop(different_size);
        drop(separate);
        drop(independent);
        r.check_error().unwrap();
        r.destroy();
    }
    window.destroy();
}
