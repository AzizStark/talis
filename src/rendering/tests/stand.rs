//! Matched cover-OFF images on an isolated, unmapped GLArea.
use super::super::*;
use crate::{
    config::{Config, PreviewMode, SceneVariant},
    playback::{PlaybackFeed, Status},
    stand_assets::StandAssets,
};
use gtk::prelude::*;

#[test]
#[ignore = "requires isolated GTK/OpenGL and matched day/night empty-stand packages"]
fn empty_stand_stop_pause_disconnect_and_restore_are_cached() {
    gtk::init().unwrap();
    let window = gtk::Window::new();
    let area = gtk::GLArea::new();
    area.set_required_version(3, 3);
    area.set_auto_render(false);
    window.set_child(Some(&area));
    gtk::prelude::WidgetExt::realize(&window);
    area.realize();
    let fixture = crate::autostart::Fixture::new();
    let output = std::env::var_os("TALIS_STAND_CAPTURE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| fixture.root.clone());
    std::fs::create_dir_all(&output).unwrap();
    let art = image::RgbaImage::from_pixel(64, 64, image::Rgba([50, 130, 220, 255]));
    for variant in [SceneVariant::Day, SceneVariant::Night] {
        let mut config = Config::default();
        config.scene.variant = variant;
        config.scene.mode = PreviewMode::Arm;
        config.render.arm_interpolation = true;
        let mut asset = PreviewAsset::load(&config).unwrap();
        if let Some(root) = std::env::var_os("TALIS_STAND_TEST_ROOT") {
            let package = config.scene.selected_package().unwrap();
            asset.empty_stand = Some(
                StandAssets::load(
                    &std::path::PathBuf::from(root)
                        .join(variant.id())
                        .join("atlas.json"),
                    &package.background,
                    (2560, 1440),
                )
                .unwrap(),
            );
        }
        let reference = asset
            .empty_stand
            .as_ref()
            .expect("matched empty stand required")
            .image
            .clone();
        // Existing button tiles deliberately replace their own beauty pixels,
        // even at neutral depth. Check the entire stand, stopping before that
        // independent turntable patch rather than comparing different bakes.
        let stand_right = asset.buttons.as_ref().unwrap().manifest.crop[0];
        assert!(stand_right > 760, "button patch overlaps the sleeve region");
        area.make_current();
        let mut renderer = unsafe { Renderer::new(asset).unwrap() };
        let mut settings = config.artwork.clone();
        settings.vinyl_rotation = false;
        let mut feed = PlaybackFeed {
            enabled: true,
            active: Some("org.mpris.MediaPlayer2.stand_fixture".into()),
            status: Status::Playing,
            ..Default::default()
        };
        unsafe {
            renderer.set_artwork(1, Some(&art), &settings).unwrap();
        }
        for (width, height) in [(2560, 1440), (3440, 1440), (3840, 2160)] {
            area.allocate(width, height, -1, None);
            area.make_current();
            area.attach_buffers();
            assert!(area.error().is_none(), "{:?}", area.error());
            let capture = |r: &mut Renderer, name: &str| {
                let path = output.join(format!("{}-{width}-{name}.png", variant.id()));
                unsafe {
                    r.render(width, height, 0.5).unwrap();
                    r.check_error().unwrap();
                    r.capture(width, height, &path).unwrap();
                }
                image::open(path).unwrap().into_rgba8()
            };
            feed.status = Status::Playing;
            renderer.set_sleeve_visible(feed.sleeve_visible());
            let covered = capture(&mut renderer, "playing");
            let counts = renderer.cache_update_counts();
            feed.status = Status::Paused;
            renderer.set_sleeve_visible(feed.sleeve_visible());
            assert_eq!(capture(&mut renderer, "paused"), covered);
            assert_eq!(
                renderer.cache_update_counts(),
                counts,
                "pause rebuilt the scene"
            );
            feed.status = Status::Stopped;
            renderer.set_sleeve_visible(feed.sleeve_visible());
            let empty = capture(&mut renderer, "stopped");
            assert_ne!(empty, covered, "stopped retained the sleeve");
            assert_eq!(
                renderer.cache_update_counts(),
                (counts.0 + 1, counts.1, counts.2)
            );
            if width == 2560 {
                // The whole exposed stand/background must match its authored
                // bake, not a black/transparent card or a partial erase mask.
                for y in 0..1440 {
                    for x in 0..stand_right {
                        let actual = empty.get_pixel(x, y);
                        let expected = reference.get_pixel(x, y);
                        for c in 0..3 {
                            assert!(
                                actual[c].abs_diff(expected[c]) <= 1,
                                "empty stand differs from bake at {x},{y}"
                            );
                        }
                    }
                }
            }
            let stopped_counts = renderer.cache_update_counts();
            feed.active = None;
            renderer.set_sleeve_visible(feed.sleeve_visible());
            assert_eq!(capture(&mut renderer, "no-player"), empty);
            assert_eq!(renderer.cache_update_counts(), stopped_counts);
            feed.active = Some("org.mpris.MediaPlayer2.stand_fixture".into());
            feed.status = Status::Paused;
            renderer.set_sleeve_visible(feed.sleeve_visible());
            assert_eq!(capture(&mut renderer, "resumed-paused"), covered);
            assert_eq!(
                renderer.cache_update_counts(),
                (counts.0 + 2, counts.1, counts.2)
            );
        }
        unsafe {
            renderer.destroy();
        }
    }
    window.destroy();
}
