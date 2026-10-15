//! Center-only default ink: no baked lighting in the rotating texture.
use super::super::*;
use crate::config::{Config, PreviewMode, SceneVariant};
use gtk::prelude::*;

#[test]
fn bundled_record_label_is_bounded_opaque_and_asymmetric() {
    let image = image::load_from_memory(include_bytes!(
        "../../../assets/artwork/fallbacks/record-label.png"
    ))
    .unwrap()
    .into_rgba8();
    assert_eq!(image.dimensions(), (512, 512));
    assert!(image.pixels().all(|pixel| pixel[3] == 255));
    assert!(image.pixels().any(|pixel| pixel[0] > 200));
    assert!(image.pixels().any(|pixel| pixel[0] < 130));
    assert_ne!(image, image::imageops::rotate180(&image));
}

#[test]
#[ignore = "Requires isolated GTK/OpenGL and complete day/night record packages"]
fn default_center_rotates_locally_without_static_redraws_or_reuploads() {
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
    let output = std::env::var_os("TALIS_FALLBACK_CAPTURE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| fixture.root.clone());
    std::fs::create_dir_all(&output).unwrap();
    for variant in [SceneVariant::Day, SceneVariant::Night] {
        let mut config = Config::default();
        config.scene.variant = variant;
        config.scene.mode = PreviewMode::Arm;
        config.render.arm_interpolation = true;
        let asset = PreviewAsset::load(&config).unwrap();
        let response = asset.record_lighting.as_ref().unwrap();
        let [left, top, width, height] = response.manifest.crop;
        let mut protected = Vec::new();
        for y in 1..height - 1 {
            for x in 1..width - 1 {
                if (y - 1..=y + 1).all(|yy| {
                    (x - 1..=x + 1).all(|xx| {
                        let index = ((yy * width + xx) * 8 + 6) as usize;
                        response.illumination[index..index + 2] == [0, 0]
                    })
                }) {
                    protected.push((left + x, top + y));
                }
            }
        }
        assert!(!protected.is_empty());
        let mut renderer = unsafe { Renderer::new(asset).unwrap() };
        let mut settings = config.artwork.clone();
        settings.vinyl_rotation = false;
        let capture = |renderer: &mut Renderer, name: &str| {
            let path = output.join(format!("{}-{name}.png", variant.id()));
            unsafe {
                renderer.render(2560, 1440, 0.0).unwrap();
                renderer.check_error().unwrap();
                renderer.capture(2560, 1440, &path).unwrap();
            }
            image::open(path).unwrap().into_rgba8()
        };
        unsafe {
            renderer.set_artwork(0, None, &settings).unwrap();
        }
        let baseline = capture(&mut renderer, "baseline");
        let counts = renderer.cache_update_counts();
        renderer.set_record_fallback_enabled(true);
        unsafe {
            renderer.set_artwork(0, None, &settings).unwrap();
        }
        let first = capture(&mut renderer, "default");
        assert_ne!(first, baseline);
        assert_eq!(renderer.source.test_artwork_uploads(), 1);
        for (index, angle) in [0.6, 1.7, 3.3].into_iter().enumerate() {
            renderer.set_record_fallback_enabled(true);
            unsafe {
                renderer.set_artwork(0, None, &settings).unwrap();
            }
            renderer.set_artwork_angle(angle);
            let rotated = capture(&mut renderer, &format!("rotated-{index}"));
            let mut changes = 0;
            for (x, y, pixel) in rotated.enumerate_pixels() {
                if pixel != first.get_pixel(x, y) {
                    changes += 1;
                    assert!(
                        x >= left && x < left + width && y >= top && y < top + height,
                        "default ink changed a non-record pixel ({x},{y})"
                    );
                }
            }
            assert!(changes > 100, "default art did not visibly rotate");
            for &(x, y) in &protected {
                assert_eq!(rotated.get_pixel(x, y), baseline.get_pixel(x, y));
            }
            assert_eq!(
                renderer.cache_update_counts(),
                counts,
                "rotation repainted static scene/stand"
            );
            assert_eq!(
                renderer.source.test_artwork_uploads(),
                1,
                "default art uploaded every frame"
            );
            assert_eq!(capture(&mut renderer, "held"), rotated);
        }
        let cover = image::RgbaImage::from_pixel(64, 64, image::Rgba([220, 180, 70, 255]));
        renderer.set_record_fallback_enabled(false);
        unsafe {
            renderer.set_artwork(1, Some(&cover), &settings).unwrap();
        }
        let fetched = capture(&mut renderer, "fetched");
        renderer.set_record_fallback_enabled(true);
        unsafe {
            renderer.set_artwork(2, None, &settings).unwrap();
        }
        assert_ne!(capture(&mut renderer, "default-restored"), fetched);
        assert_eq!(renderer.source.test_artwork_uploads(), 3);
        renderer.set_record_fallback_enabled(false);
        unsafe {
            renderer.set_artwork(2, None, &settings).unwrap();
        }
        assert_eq!(capture(&mut renderer, "stopped"), baseline);
        unsafe {
            renderer.destroy();
        }
    }
    window.destroy();
}
