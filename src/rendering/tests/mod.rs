//! Owned, unmapped GLArea checks: no real desktop/player is manipulated.

#[path = "arm_cadence.rs"]
mod arm_cadence_tests;
#[path = "arm_packing.rs"]
mod arm_packing_tests;
#[path = "fallback.rs"]
mod fallback_tests;
#[path = "gpu_profile.rs"]
mod gpu_profile_tests;
#[path = "knob.rs"]
mod knob_tests;
#[path = "regional.rs"]
mod regional_tests;
#[path = "sleeve.rs"]
mod sleeve_tests;
#[path = "stack.rs"]
mod stack_tests;
#[path = "stand.rs"]
mod stand_tests;
#[path = "view_lut.rs"]
mod view_lut_tests;
use super::*;
use crate::config::{Config, PreviewMode, SceneVariant};
use gtk::prelude::*;

#[test]
#[ignore = "requires isolated GTK/OpenGL and day/night scene packages"]
fn detailed_record_artwork_balances_minification_and_sharpness() {
    gtk::init().unwrap();
    let window = gtk::Window::new();
    let area = gtk::GLArea::new();
    area.set_required_version(3, 3);
    area.set_auto_render(false);
    window.set_child(Some(&area));
    gtk::prelude::WidgetExt::realize(&window);
    area.realize();
    let fixture = crate::autostart::Fixture::new();
    let frame = fixture.root.join("minification.png");
    // A one-texel checker must reduce to its mean, rather than moire, when
    // projected onto the label. Both inputs have the same dimensions/alpha.
    let checker = image::RgbaImage::from_fn(1024, 1024, |x, y| {
        let c = if (x + y) % 2 == 0 { 0 } else { 255 };
        image::Rgba([c, c, c, 255])
    });
    let mean = image::RgbaImage::from_pixel(1024, 1024, image::Rgba([128, 128, 128, 255]));
    for variant in [SceneVariant::Day, SceneVariant::Night] {
        let mut config = Config::default();
        config.scene.variant = variant;
        config.scene.mode = PreviewMode::Arm;
        config.render.arm_interpolation = true;
        area.make_current();
        let mut renderer = unsafe { Renderer::new(PreviewAsset::load(&config).unwrap()).unwrap() };
        let mut settings = config.artwork.clone();
        settings.record_lighting = false;
        for (width, height) in [(1920, 1080), (3840, 2160)] {
            area.allocate(width, height, -1, None);
            area.make_current();
            area.attach_buffers();
            for size in [5.0, 35.0, 100.0] {
                settings.size_percent = size;
                let transform = settings.transform().unwrap();
                let capture = |r: &mut Renderer| {
                    unsafe {
                        r.render(width, height, 0.515625).unwrap();
                        r.check_error().unwrap();
                        r.capture(width, height, &frame).unwrap();
                    }
                    image::open(&frame).unwrap().into_rgba8()
                };
                unsafe { renderer.set_artwork(1, Some(&mean), &settings).unwrap() };
                let reference = capture(&mut renderer);
                unsafe { renderer.set_artwork(2, Some(&checker), &settings).unwrap() };
                for angle in [0.0, 0.7, 2.1] {
                    let counts = renderer.cache_update_counts();
                    renderer.set_artwork_angle(angle);
                    let actual = capture(&mut renderer);
                    // The first capture after an upload updates the sleeve once.
                    if angle != 0.0 {
                        assert_eq!(renderer.cache_update_counts(), counts);
                    }
                    let mut checked = 0;
                    let mut close = 0;
                    let mut error = 0_u64;
                    for (x, y, pixel) in actual.enumerate_pixels() {
                        let sx = (x as f32 + 0.5) / width as f32;
                        let sy = (y as f32 + 0.5) / height as f32;
                        let w = transform[2] * sx + transform[5] * sy + transform[8];
                        let u = (transform[0] * sx + transform[3] * sy + transform[6]) / w;
                        let v = (transform[1] * sx + transform[4] * sy + transform[7]) / w;
                        if (u - 0.5).hypot(v - 0.5) < 0.4 {
                            checked += 1;
                            let expected = reference.get_pixel(x, y);
                            error += pixel[0].abs_diff(expected[0]) as u64;
                            if (0..3).all(|c| pixel[c].abs_diff(expected[c]) <= 2) {
                                close += 1;
                            }
                        }
                    }
                    assert!(checked > 10);
                    eprintln!(
                        "{variant:?} {width} size {size} angle {angle}: mean error {}",
                        error as f64 / checked as f64
                    );
                    // At maximum size/4K the minor axis approaches one texel per
                    // pixel. Hardware anisotropy is approximate there, so compare
                    // aliasing against unfiltered level-zero sampling, not an
                    // ideal constant that only stronger blur could guarantee.
                    if size == 100.0 && unsafe { renderer.source.test_record_filter(1.0, false) } {
                        let unfiltered = capture(&mut renderer);
                        let mut raw_error = 0_u64;
                        for (x, y, pixel) in unfiltered.enumerate_pixels() {
                            let sx = (x as f32 + 0.5) / width as f32;
                            let sy = (y as f32 + 0.5) / height as f32;
                            let w = transform[2] * sx + transform[5] * sy + transform[8];
                            let u = (transform[0] * sx + transform[3] * sy + transform[6]) / w;
                            let v = (transform[1] * sx + transform[4] * sy + transform[7]) / w;
                            if (u - 0.5).hypot(v - 0.5) < 0.4 {
                                raw_error += pixel[0].abs_diff(reference.get_pixel(x, y)[0]) as u64;
                            }
                        }
                        unsafe { renderer.source.test_record_filter(8.0, true) };
                        eprintln!(
                            "{variant:?} {width} angle {angle}: filtered {error} vs unfiltered {raw_error}"
                        );
                        assert!(
                            error * 100 < raw_error * 80,
                            "filter did not reduce near-Nyquist aliasing by at least20%"
                        );
                    } else {
                        assert!(
                            close * 100 >= checked * 99,
                            "{variant:?} {width} size {size} angle {angle}: only {close}/{checked} pixels match the filtered mean"
                        );
                    }
                }
            }
        }
        // Separately check detail preservation along the less-minified axis.
        // Isotropic mip selection uses the strongly foreshortened axis for both.
        area.allocate(1920, 1080, -1, None);
        area.make_current();
        area.attach_buffers();
        settings.size_percent = 35.0;
        let stripes = image::RgbaImage::from_fn(1024, 1024, |x, _| {
            let c = (128.0 + 100.0 * (x as f32 * std::f32::consts::TAU / 32.0).sin()) as u8;
            image::Rgba([c, c, c, 255])
        });
        let capture = |r: &mut Renderer| {
            unsafe {
                r.render(1920, 1080, 0.515625).unwrap();
                r.check_error().unwrap();
                r.capture(1920, 1080, &frame).unwrap();
            }
            image::open(&frame).unwrap().into_rgba8()
        };
        renderer.set_artwork_angle(0.0);
        unsafe { renderer.set_artwork(3, Some(&mean), &settings).unwrap() };
        let reference = capture(&mut renderer);
        unsafe { renderer.set_artwork(4, Some(&stripes), &settings).unwrap() };
        if unsafe { renderer.source.test_record_filter(1.0, true) } {
            let isotropic = capture(&mut renderer);
            let counts = renderer.cache_update_counts();
            // EXT guarantees at least2; this test works even on low-cap hardware.
            unsafe { renderer.source.test_record_filter(2.0, true) };
            let anisotropic = capture(&mut renderer);
            assert_eq!(renderer.cache_update_counts(), counts);
            let transform = settings.transform().unwrap();
            let mut iso_contrast = 0_u64;
            let mut aniso_contrast = 0_u64;
            for (x, y, pixel) in reference.enumerate_pixels() {
                let sx = (x as f32 + 0.5) / 1920.0;
                let sy = (y as f32 + 0.5) / 1080.0;
                let w = transform[2] * sx + transform[5] * sy + transform[8];
                let u = (transform[0] * sx + transform[3] * sy + transform[6]) / w;
                let v = (transform[1] * sx + transform[4] * sy + transform[7]) / w;
                if (u - 0.5).hypot(v - 0.5) < 0.35 {
                    iso_contrast += isotropic.get_pixel(x, y)[0].abs_diff(pixel[0]) as u64;
                    aniso_contrast += anisotropic.get_pixel(x, y)[0].abs_diff(pixel[0]) as u64;
                }
            }
            eprintln!(
                "{variant:?}: directional detail contrast {iso_contrast} isotropic -> {aniso_contrast} at2x"
            );
            assert!(
                aniso_contrast * 100 > iso_contrast * 125,
                "anisotropic filtering did not preserve directional detail"
            );
        } else {
            eprintln!("{variant:?}: anisotropy unavailable; mipmap fallback verified");
        }
        unsafe { renderer.destroy() };
    }
    window.destroy();
}

#[test]
#[ignore = "requires isolated GTK/OpenGL and day/night button packages"]
fn button_depths_are_local_return_exactly_and_do_not_repaint_static_scene() {
    gtk::init().unwrap();
    let window = gtk::Window::new();
    let area = gtk::GLArea::new();
    area.set_required_version(3, 3);
    area.set_auto_render(false);
    window.set_child(Some(&area));
    gtk::prelude::WidgetExt::realize(&window);
    area.realize();
    let f = crate::autostart::Fixture::new();
    let output = std::env::var_os("TALIS_BUTTON_CAPTURE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or(f.root.clone());
    std::fs::create_dir_all(&output).unwrap();
    for variant in [SceneVariant::Day, SceneVariant::Night] {
        let mut config = std::env::var_os("TALIS_SCENE_TEST_CONFIG")
            .map(|p| Config::load(std::path::Path::new(&p)).unwrap())
            .unwrap_or_default();
        config.scene.variant = variant;
        config.scene.mode = PreviewMode::Arm;
        config.render.arm_interpolation = true;
        let asset = PreviewAsset::load(&config).unwrap();
        let m = &asset
            .buttons
            .as_ref()
            .expect("matched button package required")
            .manifest;
        let steps = m.steps;
        let crop = [
            m.crop[0] as f32 / m.width as f32,
            m.crop[1] as f32 / m.height as f32,
            m.crop[2] as f32 / m.width as f32,
            m.crop[3] as f32 / m.height as f32,
        ];
        area.make_current();
        let mut renderer = unsafe { Renderer::new(asset).unwrap() };
        assert!(renderer.buttons_supported());
        for (width, height) in [(2560, 1440), (3440, 1440), (3840, 2160)] {
            area.allocate(width, height, -1, None);
            area.make_current();
            area.attach_buffers();
            let capture = |r: &mut Renderer, name: &str| {
                let path = output.join(format!("{}-{width}-{name}.png", variant.id()));
                unsafe {
                    r.render(width, height, 0.515625).unwrap();
                    r.check_error().unwrap();
                    r.capture(width, height, &path).unwrap();
                }
                image::open(path).unwrap().into_rgba8()
            };
            renderer.set_button_progress([0.0; 3]);
            let normal = capture(&mut renderer, "normal");
            let counts = renderer.cache_update_counts();
            let [x, y, w, h] = super::cache::region(
                crop,
                fitted_viewport(width, height, renderer.scene_aspect()),
            );
            let top = height - y - h;
            for button in 0..3 {
                let mut previous = normal.clone();
                for step in 1..=steps {
                    let mut progress = [0.0; 3];
                    progress[button] = step as f32 / steps as f32;
                    assert!(renderer.set_button_progress(progress));
                    let image = capture(&mut renderer, &format!("{button}-{step}"));
                    assert_ne!(image, previous, "button depth has no visual change");
                    for (px, py, pixel) in image.enumerate_pixels() {
                        if pixel != normal.get_pixel(px, py) {
                            assert!(
                                px as i32 >= x
                                    && (px as i32) < x + w
                                    && py as i32 >= top
                                    && (py as i32) < top + h,
                                "button changed outside its patch at {px},{py}"
                            );
                        }
                    }
                    assert_eq!(
                        renderer.cache_update_counts(),
                        counts,
                        "button repainted a static/arm/sleeve region"
                    );
                    previous = image;
                }
                assert!(renderer.set_button_progress([0.0; 3]));
                assert_eq!(
                    capture(&mut renderer, "returned"),
                    normal,
                    "release accumulated stale button pixels"
                );
            }
            let updates = renderer.button_update_count();
            assert!(!renderer.set_button_progress([0.0; 3]));
            assert_eq!(capture(&mut renderer, "idle"), normal);
            assert_eq!(
                renderer.button_update_count(),
                updates,
                "idle redrew buttons"
            );
            println!(
                "{} {width}x{height}: 24 intermediate depths and releases verified; static counters {counts:?}",
                variant.id()
            );
        }
        area.make_current();
        unsafe { renderer.destroy() };
    }
    window.destroy();
}

#[test]
#[ignore = "requires isolated GTK/OpenGL and complete vinyl surface packages"]
fn vinyl_surface_rotates_without_repainting_static_scene() {
    gtk::init().unwrap();
    let window = gtk::Window::new();
    let area = gtk::GLArea::new();
    area.set_required_version(3, 3);
    area.set_auto_render(false);
    window.set_child(Some(&area));
    gtk::prelude::WidgetExt::realize(&window);
    area.realize();
    area.allocate(2560, 1440, -1, None);
    area.make_current();
    area.attach_buffers();
    assert!(area.error().is_none(), "{:?}", area.error());
    let fixture = crate::autostart::Fixture::new();
    let output = std::env::var_os("TALIS_VINYL_CAPTURE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| fixture.root.clone());
    std::fs::create_dir_all(&output).unwrap();
    for variant in [SceneVariant::Day, SceneVariant::Night] {
        let mut config = std::env::var_os("TALIS_SCENE_TEST_CONFIG")
            .map(|p| Config::load(std::path::Path::new(&p)).unwrap())
            .unwrap_or_default();
        config.scene.variant = variant;
        config.scene.mode = PreviewMode::Arm;
        config.render.arm_interpolation = true;
        let package = config.scene.selected_package().unwrap();
        let asset = PreviewAsset::load(&config).unwrap();
        let vinyl = asset
            .vinyl_detail
            .as_ref()
            .expect("matched vinyl calibration required");
        let fit = vinyl.response.transform;
        let crop = vinyl.response.manifest.crop;
        let baked_detail = vinyl.baked_detail;
        let (arm_crop, mask_a, mask_b) = match &asset.scene {
            crate::preview::PreviewScene::Arm(arm) => {
                let masks = &arm.flow.as_ref().unwrap().edges.as_ref().unwrap().masks;
                (arm.manifest.crop, masks[16].clone(), masks[17].clone())
            }
            _ => unreachable!(),
        };
        let mut renderer = unsafe { Renderer::new(asset).unwrap() };
        assert!(renderer.vinyl_supported());
        let capture = |r: &mut Renderer, name: &str| {
            let path = output.join(format!("{}-{name}.png", variant.id()));
            unsafe {
                r.render(2560, 1440, 0.515625).unwrap();
                r.check_error().unwrap();
                r.capture(2560, 1440, &path).unwrap();
            }
            image::open(path).unwrap().into_rgba8()
        };
        let mut settings = config.artwork.clone();
        settings.enabled = false;
        settings.vinyl_rotation = false;
        unsafe {
            renderer.set_artwork(1, None, &settings).unwrap();
        }
        let off = capture(&mut renderer, "off");
        settings.vinyl_rotation = true;
        unsafe {
            renderer.set_artwork(1, None, &settings).unwrap();
        }
        renderer.set_artwork_angle(0.0);
        let zero = capture(&mut renderer, "zero");
        if baked_detail {
            assert_eq!(zero, off, "baked zero-angle correction must be identity");
        } else {
            assert_ne!(zero, off, "new authored marks were not applied");
        }
        let counts = renderer.cache_update_counts();
        renderer.set_artwork_angle(1.2);
        let rotated = capture(&mut renderer, "rotated");
        assert_eq!(
            renderer.cache_update_counts(),
            counts,
            "rotation rebuilt scene patches"
        );
        let mut changed = 0;
        for ((x, y, a), (_, _, b)) in zero.enumerate_pixels().zip(rotated.enumerate_pixels()) {
            if a != b {
                changed += 1;
                let sx = (x as f32 + 0.5) / 2560.0;
                let sy = (y as f32 + 0.5) / 1440.0;
                let w = fit[2] * sx + fit[5] * sy + fit[8];
                let u = (fit[0] * sx + fit[3] * sy + fit[6]) / w;
                let v = (fit[1] * sx + fit[4] * sy + fit[7]) / w;
                assert!(
                    (u - 0.5).hypot(v - 0.5) < 0.494,
                    "surface changed outside record {x},{y}"
                );
                assert!(
                    (u - 0.5).hypot(v - 0.5) > 0.17,
                    "surface changed centre label/spindle {x},{y}"
                );
            }
        }
        assert!(changed > 100, "surface did not move: {changed}");
        let mut protected = 0;
        for y in 3..mask_a.height() - 3 {
            for x in 3..mask_a.width() - 3 {
                if (-3..=3).all(|dy| {
                    (-3..=3).all(|dx| {
                        let px = (x as i32 + dx) as u32;
                        let py = (y as i32 + dy) as u32;
                        mask_a.get_pixel(px, py)[0] == 255 && mask_b.get_pixel(px, py)[0] == 255
                    })
                }) {
                    protected += 1;
                    assert_eq!(
                        zero.get_pixel(x + arm_crop[0], y + arm_crop[1]),
                        rotated.get_pixel(x + arm_crop[0], y + arm_crop[1]),
                        "surface painted over arm"
                    );
                }
            }
        }
        assert!(protected > 20);
        assert_eq!(
            capture(&mut renderer, "paused"),
            rotated,
            "frozen phase was not stable"
        );
        renderer.set_artwork_angle(2.4);
        assert_ne!(capture(&mut renderer, "later"), rotated);
        renderer.set_artwork_angle(1.2);
        assert_eq!(capture(&mut renderer, "returned"), rotated);
        println!("VINYL_LOCAL variant={variant:?} changed_pixels={changed} cache={counts:?}");
        if let Some(root) = package.vinyl_detail.as_ref().and_then(|p| p.parent()) {
            let a = root.join("work/reference-zero.png");
            let b = root.join("work/reference-rotated.png");
            if a.exists() && b.exists() {
                let a = image::open(a).unwrap().into_rgb8();
                let b = image::open(b).unwrap().into_rgb8();
                let mut errors = Vec::new();
                for y in 0..crop[3] {
                    for x in 0..crop[2] {
                        let sx = (x + crop[0]) as f32 / 2560.0;
                        let sy = (y + crop[1]) as f32 / 1440.0;
                        let w = fit[2] * sx + fit[5] * sy + fit[8];
                        let u = (fit[0] * sx + fit[3] * sy + fit[6]) / w;
                        let v = (fit[1] * sx + fit[4] * sy + fit[7]) / w;
                        // Left annulus is clear of the arm and its shadow in both
                        // authored scenes. Compare motion DELTA, not different poses.
                        if u < 0.6 && (0.2..0.48).contains(&(u - 0.5).hypot(v - 0.5)) {
                            let p = zero.get_pixel(x + crop[0], y + crop[1]);
                            let q = rotated.get_pixel(x + crop[0], y + crop[1]);
                            let c = a.get_pixel(x, y);
                            let d = b.get_pixel(x, y);
                            if (0..3).any(|i| {
                                (d[i] as i16 - c[i] as i16).abs() > 2
                                    || (q[i] as i16 - p[i] as i16).abs() > 2
                            }) {
                                for i in 0..3 {
                                    errors.push(
                                        ((q[i] as i16 - p[i] as i16) - (d[i] as i16 - c[i] as i16))
                                            .unsigned_abs(),
                                    );
                                }
                            }
                        }
                    }
                }
                errors.sort_unstable();
                assert!(errors.len() > 100);
                let mean = errors.iter().map(|&v| v as f64).sum::<f64>() / errors.len() as f64;
                let p95 = errors[errors.len() * 95 / 100];
                println!(
                    "VINYL_REFERENCE variant={variant:?} samples={} mean={mean:.3} p95={p95} max={}",
                    errors.len(),
                    errors.last().unwrap()
                );
                assert!(
                    mean < 3.0 && p95 < 10,
                    "rotating marks differ from Blender reference"
                );
            }
        }
        // Live gains must affect only surface marks and never rebuild retained
        // room/arm/sleeve patches. Neutral defaults recover the exact old frame.
        for (opacity, brightness, name) in [
            (0.5, 1.0, "half-opacity"),
            (1.0, 2.0, "double-brightness"),
            (1.0, 0.0, "black-marks"),
            (0.0, 1.0, "hidden"),
        ] {
            settings.vinyl_opacity = opacity;
            settings.vinyl_brightness = brightness;
            unsafe {
                renderer.set_artwork(1, None, &settings).unwrap();
            }
            let adjusted = capture(&mut renderer, name);
            assert_ne!(adjusted, rotated, "{name} had no effect in {variant:?}");
            assert_eq!(
                renderer.cache_update_counts(),
                counts,
                "{name} rebuilt scene patches"
            );
            if opacity == 0.0 && !baked_detail {
                assert_eq!(adjusted, off, "zero opacity did not hide all marks");
            }
            for ((x, y, a), (_, _, b)) in
                adjusted.enumerate_pixels().zip(rotated.enumerate_pixels())
            {
                if a != b {
                    let sx = (x as f32 + 0.5) / 2560.0;
                    let sy = (y as f32 + 0.5) / 1440.0;
                    let w = fit[2] * sx + fit[5] * sy + fit[8];
                    let u = (fit[0] * sx + fit[3] * sy + fit[6]) / w;
                    let v = (fit[1] * sx + fit[4] * sy + fit[7]) / w;
                    let radius = (u - 0.5).hypot(v - 0.5);
                    assert!(
                        (0.17..0.494).contains(&radius),
                        "{name} changed non-surface pixel {x},{y}"
                    );
                }
            }
            for y in 3..mask_a.height() - 3 {
                for x in 3..mask_a.width() - 3 {
                    if (-3..=3).all(|dy| {
                        (-3..=3).all(|dx| {
                            let px = (x as i32 + dx) as u32;
                            let py = (y as i32 + dy) as u32;
                            mask_a.get_pixel(px, py)[0] == 255 && mask_b.get_pixel(px, py)[0] == 255
                        })
                    }) {
                        assert_eq!(
                            adjusted.get_pixel(x + arm_crop[0], y + arm_crop[1]),
                            rotated.get_pixel(x + arm_crop[0], y + arm_crop[1]),
                            "{name} changed the arm"
                        );
                    }
                }
            }
        }
        settings.vinyl_brightness = 2.0;
        unsafe {
            renderer.set_artwork(1, None, &settings).unwrap();
        }
        let hidden = capture(&mut renderer, "hidden-bright");
        renderer.set_artwork_angle(2.4);
        assert_eq!(
            capture(&mut renderer, "hidden-moved"),
            hidden,
            "zero-opacity marks depend on phase or brightness"
        );
        renderer.set_artwork_angle(1.2);
        settings.vinyl_opacity = 1.0;
        settings.vinyl_brightness = 1.0;
        unsafe {
            renderer.set_artwork(1, None, &settings).unwrap();
        }
        assert_eq!(capture(&mut renderer, "neutral-restored"), rotated);
        assert_eq!(renderer.cache_update_counts(), counts);

        // A real fetched centre image is also independent of dust appearance.
        let white = image::RgbaImage::from_pixel(64, 64, image::Rgba([255, 255, 255, 255]));
        settings.enabled = true;
        settings.vinyl_rotation = false;
        unsafe {
            renderer.set_artwork(2, Some(&white), &settings).unwrap();
        }
        let label = capture(&mut renderer, "label-no-dust");
        let counts = renderer.cache_update_counts();
        settings.vinyl_rotation = true;
        settings.vinyl_opacity = 0.25;
        settings.vinyl_brightness = 2.0;
        unsafe {
            renderer.set_artwork(2, Some(&white), &settings).unwrap();
        }
        let with_label = capture(&mut renderer, "label-adjusted-dust");
        assert_ne!(with_label, label);
        assert_eq!(renderer.cache_update_counts(), counts);
        for ((x, y, a), (_, _, b)) in label.enumerate_pixels().zip(with_label.enumerate_pixels()) {
            let sx = (x as f32 + 0.5) / 2560.0;
            let sy = (y as f32 + 0.5) / 1440.0;
            let w = fit[2] * sx + fit[5] * sy + fit[8];
            let u = (fit[0] * sx + fit[3] * sy + fit[6]) / w;
            let v = (fit[1] * sx + fit[4] * sy + fit[7]) / w;
            if (u - 0.5).hypot(v - 0.5) < 0.17 {
                assert_eq!(a, b, "surface gains changed fetched artwork/spindle");
            }
        }
        settings.vinyl_opacity = 1.0;
        settings.vinyl_brightness = 1.0;
        if std::env::var_os("TALIS_VINYL_BENCHMARK").is_some() {
            unsafe {
                renderer.enable_gpu_timing(false);
            }
            // Baseline includes the existing label pass, not just presentation.
            let white = image::RgbaImage::from_pixel(64, 64, image::Rgba([255, 255, 255, 255]));
            settings.enabled = true;
            unsafe {
                renderer.set_artwork(2, Some(&white), &settings).unwrap();
                renderer.render(2560, 1440, 0.515625).unwrap();
            }
            let counts = renderer.cache_update_counts();
            for enabled in [false, true] {
                settings.vinyl_rotation = enabled;
                unsafe {
                    renderer.set_artwork(2, Some(&white), &settings).unwrap();
                }
                let mut samples = Vec::new();
                for frame in 0..220 {
                    renderer.set_artwork_angle(frame as f32 * 0.07);
                    unsafe {
                        renderer.render(2560, 1440, 0.515625).unwrap();
                        renderer.gl.finish();
                    }
                    if frame >= 20 {
                        if let Some(ms) = renderer.gpu_ms {
                            samples.push(ms);
                        }
                    }
                }
                samples.sort_by(f64::total_cmp);
                assert!(samples.len() > 100);
                println!(
                    "VINYL_GPU variant={variant:?} enabled={enabled} median_ms={:.4} p95_ms={:.4}",
                    samples[samples.len() / 2],
                    samples[samples.len() * 95 / 100]
                );
                assert_eq!(renderer.cache_update_counts(), counts);
            }
        }
        unsafe {
            renderer.destroy();
        }
    }
    window.destroy();
}

#[test]
#[ignore = "requires a GTK/OpenGL display and both complete lighting packages"]
fn record_lighting_is_stationary_and_region_local() {
    gtk::init().unwrap();
    let window = gtk::Window::new();
    let area = gtk::GLArea::new();
    area.set_required_version(3, 3);
    area.set_auto_render(false);
    window.set_child(Some(&area));
    // Realize a native surface without mapping/showing a window. The benchmark
    // owns its framebuffer and does not involve the user's compositor/player.
    gtk::prelude::WidgetExt::realize(&window);
    area.realize();
    area.allocate(2560, 1440, -1, None);
    area.make_current();
    assert!(area.error().is_none(), "{:?}", area.error());
    area.attach_buffers();
    let fixture = crate::autostart::Fixture::new();
    let frame = fixture.root.join("frame.png");
    let white = image::RgbaImage::from_pixel(64, 64, image::Rgba([255, 255, 255, 255]));
    for variant in [SceneVariant::Day, SceneVariant::Night] {
        let mut config = std::env::var_os("TALIS_SCENE_TEST_CONFIG")
            .map(|path| Config::load(std::path::Path::new(&path)).unwrap())
            .unwrap_or_default();
        config.scene.variant = variant;
        config.scene.mode = PreviewMode::Arm;
        config.render.arm_interpolation = true;
        let asset = PreviewAsset::load(&config).unwrap();
        assert!(
            asset.record_lighting.is_some(),
            "matched calibration required"
        );
        let (arm_crop, arm_mask_a, arm_mask_b) = match &asset.scene {
            crate::preview::PreviewScene::Arm(arm) => {
                let edges = arm.flow.as_ref().unwrap().edges.as_ref().unwrap();
                (
                    arm.manifest.crop,
                    edges.masks[16].clone(),
                    edges.masks[17].clone(),
                )
            }
            _ => unreachable!(),
        };
        let mut renderer = unsafe { Renderer::new(asset).unwrap() };
        let mut settings = config.artwork.clone();
        settings.vinyl_rotation = false;
        let capture = |renderer: &mut Renderer| {
            unsafe {
                renderer.render(2560, 1440, 0.515625).unwrap();
                renderer.check_error().unwrap();
                renderer.capture(2560, 1440, &frame).unwrap();
            }
            image::open(&frame).unwrap().into_rgba8()
        };
        unsafe { renderer.set_artwork(1, Some(&white), &settings).unwrap() };
        let first = capture(&mut renderer);
        let initial = renderer.cache_update_counts();
        // Same physical scene/artwork, with and without the night-only trim.
        renderer.source.set_scene_variant(SceneVariant::Day);
        let full_brightness = capture(&mut renderer);
        if variant == SceneVariant::Day {
            assert_eq!(first, full_brightness, "day brightness must be unchanged");
        } else {
            let transform = settings.transform().unwrap();
            let mut darkened = 0;
            for (x, y, pixel) in first.enumerate_pixels() {
                let before = full_brightness.get_pixel(x, y);
                if pixel != before {
                    darkened += 1;
                    for channel in 0..3 {
                        assert!(
                            pixel[channel] <= before[channel],
                            "night trim brightened artwork"
                        );
                    }
                    let sx = (x as f32 + 0.5) / 2560.0;
                    let sy = (y as f32 + 0.5) / 1440.0;
                    let w = transform[2] * sx + transform[5] * sy + transform[8];
                    let u = (transform[0] * sx + transform[3] * sy + transform[6]) / w;
                    let v = (transform[1] * sx + transform[4] * sy + transform[7]) / w;
                    assert!(
                        (u - 0.5).hypot(v - 0.5) < 0.62,
                        "trim changed non-label pixel"
                    );
                }
            }
            assert!(
                darkened > 100,
                "night trim must visibly dim the center label"
            );
        }
        renderer.source.set_scene_variant(variant);
        assert_eq!(
            capture(&mut renderer),
            first,
            "brightness restoration must be exact"
        );
        assert_eq!(
            renderer.cache_update_counts(),
            initial,
            "trim repainted static patches"
        );
        for angle in [0.5, 1.5, 3.0, 5.9] {
            renderer.set_artwork_angle(angle);
            assert_eq!(
                capture(&mut renderer),
                first,
                "lighting/occlusion rotated in {variant:?}"
            );
            assert_eq!(
                renderer.cache_update_counts(),
                initial,
                "rotation invalidated static patches"
            );
        }
        for size in [5.0, 35.0, 100.0] {
            settings.size_percent = size;
            settings.record_lighting = false;
            unsafe { renderer.set_artwork(1, Some(&white), &settings).unwrap() };
            let unlit = capture(&mut renderer);
            let counts = renderer.cache_update_counts();
            settings.record_lighting = true;
            unsafe { renderer.set_artwork(1, Some(&white), &settings).unwrap() };
            let lit = capture(&mut renderer);
            assert_eq!(
                renderer.cache_update_counts(),
                counts,
                "record settings invalidated scene/sleeve"
            );
            assert_ne!(lit, unlit, "lighting did not apply at size {size}");
            if size == 100.0 {
                let mut checked = 0;
                for y in 4..arm_mask_a.height() - 4 {
                    for x in 4..arm_mask_a.width() - 4 {
                        let core = (-4..=4).all(|dy| {
                            (-4..=4).all(|dx| {
                                let px = (x as i32 + dx) as u32;
                                let py = (y as i32 + dy) as u32;
                                arm_mask_a.get_pixel(px, py)[0] == 255
                                    && arm_mask_b.get_pixel(px, py)[0] == 255
                            })
                        });
                        if core {
                            checked += 1;
                            assert_eq!(
                                lit.get_pixel(x + arm_crop[0], y + arm_crop[1]),
                                unlit.get_pixel(x + arm_crop[0], y + arm_crop[1]),
                                "label lighting painted over the moving arm"
                            );
                        }
                    }
                }
                assert!(
                    checked > 20,
                    "arm occlusion check must cover actual geometry"
                );
            }
            let transform = settings.transform().unwrap();
            for ((x, y, before), (_, _, after)) in
                unlit.enumerate_pixels().zip(lit.enumerate_pixels())
            {
                if before != after {
                    let sx = (x as f32 + 0.5) / 2560.0;
                    let sy = (y as f32 + 0.5) / 1440.0;
                    let w = transform[2] * sx + transform[5] * sy + transform[8];
                    let u = (transform[0] * sx + transform[3] * sy + transform[6]) / w;
                    let v = (transform[1] * sx + transform[4] * sy + transform[7]) / w;
                    assert!(
                        (u - 0.5).hypot(v - 0.5) < 0.62,
                        "lighting changed non-label pixel {x},{y}"
                    );
                }
            }
        }
        if std::env::var_os("TALIS_RECORD_BENCHMARK").is_some() {
            unsafe {
                renderer.enable_gpu_timing(false);
            }
            let driver = unsafe { renderer.gl.get_parameter_string(glow::RENDERER) };
            for size in [35.0, 100.0] {
                for enabled in [false, true] {
                    settings.size_percent = size;
                    settings.record_lighting = enabled;
                    unsafe { renderer.set_artwork(1, Some(&white), &settings).unwrap() };
                    let mut samples = Vec::new();
                    for frame in 0..220 {
                        renderer.set_artwork_angle(frame as f32 * 0.07);
                        unsafe {
                            renderer.render(2560, 1440, 0.515625).unwrap();
                            // Explicit diagnostic synchronization makes the
                            // previous timer available; never used in app playback.
                            renderer.gl.finish();
                        }
                        if frame >= 20 {
                            if let Some(ms) = renderer.gpu_ms {
                                samples.push(ms);
                            }
                        }
                    }
                    assert!(samples.len() > 100);
                    samples.sort_by(f64::total_cmp);
                    println!(
                        "RECORD_GPU variant={variant:?} size={size} lighting={enabled} median_ms={:.4} p95_ms={:.4} driver={driver}",
                        samples[samples.len() / 2],
                        samples[samples.len() * 95 / 100]
                    );
                    assert_eq!(renderer.cache_update_counts(), initial);
                }
            }
        }
        unsafe { renderer.destroy() };
    }
    window.destroy();
}
