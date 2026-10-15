//! Real shader captures on an isolated, unmapped GLArea.
use super::super::*;
use crate::config::{Config, PreviewMode, SceneVariant};
use gtk::prelude::*;
#[test]
#[ignore = "requires isolated GTK/OpenGL and day/night knob packages"]
fn marker_rotates_locally_with_fixed_body_and_idle_cache() {
    gtk::init().unwrap();
    let window = gtk::Window::new();
    let area = gtk::GLArea::new();
    area.set_required_version(3, 3);
    area.set_auto_render(false);
    window.set_child(Some(&area));
    gtk::prelude::WidgetExt::realize(&window);
    area.realize();
    let fixture = crate::autostart::Fixture::new();
    let output = std::env::var_os("TALIS_KNOB_CAPTURE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or(fixture.root.clone());
    std::fs::create_dir_all(&output).unwrap();
    for variant in [SceneVariant::Day, SceneVariant::Night] {
        let mut config = Config::default();
        config.scene.variant = variant;
        config.scene.mode = PreviewMode::Arm;
        config.render.arm_interpolation = true;
        let asset = PreviewAsset::load(&config).unwrap();
        let knob = asset.knob.as_ref().expect("matched knob required");
        let transform = knob.transform;
        let m = &knob.manifest;
        let crop = [
            m.crop[0] as f32 / m.width as f32,
            m.crop[1] as f32 / m.height as f32,
            m.crop[2] as f32 / m.width as f32,
            m.crop[3] as f32 / m.height as f32,
        ];
        area.make_current();
        let mut renderer = unsafe { Renderer::new(asset).unwrap() };
        assert!(renderer.knob_supported());
        for (width, height) in [(2560, 1440), (3440, 1440), (3840, 2160)] {
            area.allocate(width, height, -1, None);
            area.make_current();
            area.attach_buffers();
            let capture = |r: &mut Renderer, name: &str| {
                let path = output.join(format!("{}-{width}-{name}.png", variant.id()));
                unsafe {
                    r.render(width, height, 0.5).unwrap();
                    r.check_error().unwrap();
                    r.capture(width, height, &path).unwrap();
                }
                let image = image::open(path).unwrap().into_rgba8();
                if width == 2560 {
                    let [x, y, w, h] =
                        cache::region(crop, fitted_viewport(width, height, r.scene_aspect()));
                    let patch = image::imageops::crop_imm(
                        &image,
                        x as u32,
                        (height - y - h) as u32,
                        w as u32,
                        h as u32,
                    )
                    .to_image();
                    image::imageops::resize(
                        &patch,
                        patch.width() * 4,
                        patch.height() * 4,
                        image::imageops::FilterType::Nearest,
                    )
                    .save(output.join(format!("{}-knob-{name}.png", variant.id())))
                    .unwrap();
                }
                image
            };
            renderer.set_knob_angle(0.0);
            let initial = capture(&mut renderer, "minimum");
            let counts = renderer.cache_update_counts();
            let buttons = renderer.button_update_count();
            let viewport = fitted_viewport(width, height, renderer.scene_aspect());
            let [x, y, w, h] = cache::region(crop, viewport);
            let top = height - y - h;
            for (index, angle) in [
                std::f32::consts::FRAC_PI_2,
                std::f32::consts::PI,
                std::f32::consts::PI * 1.5,
            ]
            .iter()
            .enumerate()
            {
                assert!(renderer.set_knob_angle(*angle));
                let image = capture(&mut renderer, &format!("angle-{index}"));
                let mut changed = 0;
                for (px, py, pixel) in image.enumerate_pixels() {
                    if pixel != initial.get_pixel(px, py) {
                        changed += 1;
                        assert!(
                            px as i32 >= x
                                && (px as i32) < x + w
                                && py as i32 >= top
                                && (py as i32) < top + h,
                            "marker leaked outside crop at {px},{py}"
                        );
                        let sx = (px as f32 + 0.5 - viewport.0 as f32) / viewport.2 as f32;
                        let sy = (py as f32 + 0.5 - (height - viewport.1 - viewport.3) as f32)
                            / viewport.3 as f32;
                        let u = transform[0] * sx + transform[3] * sy + transform[6];
                        let v = transform[1] * sx + transform[4] * sy + transform[7];
                        let z = transform[2] * sx + transform[5] * sy + transform[8];
                        let radius =
                            ((u / z * 2.0 - 1.0).powi(2) + (v / z * 2.0 - 1.0).powi(2)).sqrt();
                        assert!(
                            radius < 0.8,
                            "knob body/highlight changed at radius {radius}"
                        );
                    }
                }
                assert!(changed > 0, "marker invisible");
                assert_eq!(renderer.cache_update_counts(), counts);
                assert_eq!(renderer.button_update_count(), buttons);
            }
            assert!(renderer.set_knob_angle(std::f32::consts::TAU));
            assert_eq!(
                capture(&mut renderer, "maximum"),
                initial,
                "max volume did not return to the camera-facing position"
            );
            assert!(!renderer.set_knob_angle(std::f32::consts::TAU));
            assert!(renderer.set_knob_angle(0.0));
            assert_eq!(
                capture(&mut renderer, "returned"),
                initial,
                "old marker trails accumulated"
            );
            let updates = renderer.knob_update_count();
            assert!(!renderer.set_knob_angle(0.0));
            assert!(!renderer.set_knob_angle(f32::NAN));
            assert_eq!(capture(&mut renderer, "idle"), initial);
            assert_eq!(renderer.knob_update_count(), updates);
            println!(
                "{} {width}x{height}: marker-only changes, unchanged body/static caches, exact return, idle verified",
                variant.id()
            );
        }
        area.make_current();
        drop(renderer);
    }
    window.destroy();
}
