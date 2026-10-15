use super::*;
use crate::{
    config::{Config, PreviewMode, SceneVariant},
    preview::PreviewAsset,
    rendering::Renderer,
};
use glib::translate::ToGlibPtr;
use std::{
    rc::Rc,
    time::{Duration, Instant},
};

// Direct class snapshots on owned diagnostic widgets bypass GTK's outer
// render-node cache. Production always uses normal GTK snapshot traversal.
unsafe fn snapshot(area: &gtk::GLArea) -> gtk::gsk::RenderNode {
    let snapshot = gtk::Snapshot::new();
    unsafe {
        let instance = area.as_ptr() as *mut glib::gobject_ffi::GTypeInstance;
        let class = (*instance).g_class as *mut gtk::ffi::GtkWidgetClass;
        ((*class).snapshot.unwrap())(area.as_ptr().cast(), snapshot.to_glib_none().0);
    }
    snapshot.to_node().expect("Presenter has a render node")
}
fn pump(duration: Duration) {
    let until = Instant::now() + duration;
    let context = glib::MainContext::default();
    while Instant::now() < until {
        while context.pending() && Instant::now() < until {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn diagnostic_config() -> Config {
    std::env::var_os("TALIS_PRESENTATION_TEST_CONFIG")
        .map(|p| Config::load(std::path::Path::new(&p)).expect("explicit diagnostic profile"))
        .unwrap_or_default()
}
#[test]
#[ignore = "requires native X11/GLX; maps owned transparent diagnostic windows"]
fn separate_native_patch_matches_full_pixels_and_keeps_background_idle() {
    gtk::init().unwrap();
    let app = gtk::Application::builder()
        .application_id("io.github.azizstark.Talis.NativePatchTest")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gtk::gio::Cancellable>).unwrap();
    let (width, height) = if std::env::var_os("TALIS_PRESENTATION_TEST_4K").is_some() {
        (3840, 2160)
    } else if std::env::var_os("TALIS_PRESENTATION_TEST_1080P").is_some() {
        (1920, 1080)
    } else {
        (960, 540)
    };
    let scene = gtk::ApplicationWindow::builder()
        .application(&app)
        .default_width(width)
        .default_height(height)
        .build();
    scene.set_opacity(0.0);
    scene.set_focusable(false);
    let area = new();
    area.set_required_version(3, 3);
    scene.set_child(Some(&area));
    gtk::prelude::WidgetExt::realize(&scene);
    area.realize();
    let shared: SharedRenderer = Rc::new(RefCell::new(None));
    bind(&area, &shared);
    let r = shared.clone();
    let frames = Rc::new(Cell::new(0));
    let count = frames.clone();
    let gl_errors = Rc::new(RefCell::new(Vec::new()));
    let errors = gl_errors.clone();
    let progress = Rc::new(Cell::new(0.515625));
    let pose = progress.clone();
    area.connect_render(move |area, _| {
        count.set(count.get() + 1);
        let scale = area.scale_factor();
        let (w, h) = (area.width() * scale, area.height() * scale);
        let mut borrow = r.borrow_mut();
        let renderer = borrow.as_mut().unwrap();
        if let Some(region) = region(area) {
            if unsafe { renderer.render_regional(w, h, pose.get(), region).unwrap() } {
                refresh_background(area);
            }
        } else {
            unsafe {
                renderer.render(w, h, pose.get()).unwrap();
            }
        }
        if let Err(error) = unsafe { renderer.check_error() } {
            errors.borrow_mut().push(error.to_string());
        }
        glib::Propagation::Stop
    });
    let host = crate::desktop::DesktopHost::new(0).unwrap();
    super::super::native_patch::install(&area, &scene, &host).unwrap();
    let owner = area.downcast_ref::<RegionalArea>().unwrap();
    owner
        .imp()
        .native
        .borrow()
        .as_ref()
        .unwrap()
        .window
        .set_opacity(0.0);
    let reference_window = gtk::Window::new();
    let reference = gtk::GLArea::new();
    reference.set_required_version(3, 3);
    reference.set_auto_render(false);
    reference_window.set_child(Some(&reference));
    gtk::prelude::WidgetExt::realize(&reference_window);
    reference.realize();
    let normal: SharedRenderer = Rc::new(RefCell::new(None));
    let r = normal.clone();
    let pose = progress.clone();
    reference.connect_render(move |area, _| {
        let scale = area.scale_factor();
        unsafe {
            r.borrow_mut()
                .as_mut()
                .unwrap()
                .render(area.width() * scale, area.height() * scale, pose.get())
                .unwrap();
        }
        glib::Propagation::Stop
    });
    let art = image::RgbaImage::from_fn(256, 256, |x, y| image::Rgba([x as u8, y as u8, 80, 255]));
    for variant in [SceneVariant::Day, SceneVariant::Night] {
        let mut config = diagnostic_config();
        config.scene.mode = PreviewMode::Arm;
        config.scene.variant = variant;
        config.render.arm_interpolation = true;
        for (a, store) in [(&area, &shared), (&reference, &normal)] {
            a.make_current();
            if let Some(old) = store.borrow_mut().take() {
                unsafe {
                    old.destroy();
                }
            }
            let mut r = unsafe { Renderer::new(PreviewAsset::load(&config).unwrap()).unwrap() };
            unsafe {
                r.set_artwork(1, Some(&art), &config.artwork).unwrap();
            }
            *store.borrow_mut() = Some(r);
        }
        scene.set_visible(true);
        pump(Duration::from_millis(120));
        drop(unsafe { snapshot(&area) });
        pump(Duration::from_millis(100));
        assert!(
            gl_errors.borrow().is_empty(),
            "startup GL errors: {:?}",
            gl_errors.borrow()
        );
        let imp = owner.imp();
        assert!(!imp.failed.get());
        let animated = imp.animated.get().unwrap();
        assert_ne!(
            area.root(),
            animated.root(),
            "patch is still in the background toplevel"
        );
        let scale = area.scale_factor();
        let (w, h) = (area.width() * scale, area.height() * scale);
        reference.allocate(area.width(), area.height(), -1, None);
        let gsk = scene.renderer().unwrap();
        let pixels = |node: gtk::gsk::RenderNode| {
            let scaled = gtk::Snapshot::new();
            scaled.scale(scale as f32, scale as f32);
            scaled.append_node(&node);
            let texture = gsk.render_texture(
                scaled.to_node().unwrap(),
                Some(&gtk::graphene::Rect::new(0., 0., w as f32, h as f32)),
            );
            let mut data = vec![0; (w * h * 4) as usize];
            texture.download(&mut data, w as usize * 4);
            data
        };
        let mut background_draws = 0;
        for step in 0..6 {
            progress.set([0.0, 1.0, 0.37, 0.9, 0.02, 0.65][step]);
            for (a, store) in [(&area, &shared), (&reference, &normal)] {
                a.make_current();
                let mut r = store.borrow_mut();
                let r = r.as_mut().unwrap();
                r.set_artwork_angle(step as f32 * 0.5);
                r.set_button_progress([step as f32 / 5.0, 0.0, 0.5]);
                r.set_knob_angle(step as f32 * 0.1);
                if step == 3 {
                    unsafe {
                        r.set_previous_artwork(1, Some(&art)).unwrap();
                    }
                }
                if step == 4 {
                    r.set_sleeve_visible(false);
                }
                queue_render(a);
            }
            let patch = unsafe { snapshot(animated) };
            let background = unsafe { snapshot(&area) };
            let [x, y, _, rh] = region(&area).unwrap();
            let combined = gtk::Snapshot::new();
            combined.append_node(&background);
            combined.translate(&gtk::graphene::Point::new(
                x as f32 / scale as f32,
                (h - y - rh) as f32 / scale as f32,
            ));
            combined.append_node(&patch);
            let actual = pixels(combined.to_node().unwrap());
            let expected = pixels(unsafe { snapshot(&reference) });
            assert_eq!(actual.len(), expected.len());
            let mismatches = actual.iter().zip(&expected).filter(|(a, b)| a != b).count();
            assert_eq!(
                mismatches, 0,
                "native patch {variant:?} step {step} differs"
            );
            for (a, store) in [(&area, &shared), (&reference, &normal)] {
                a.make_current();
                unsafe {
                    store.borrow().as_ref().unwrap().check_error().unwrap();
                }
            }
            assert!(
                gl_errors.borrow().is_empty(),
                "render GL errors: {:?}",
                gl_errors.borrow()
            );
            if matches!(step, 1 | 2 | 5) {
                assert_eq!(imp.background_draws.get(), background_draws);
            }
            background_draws = imp.background_draws.get();
        }
        pump(Duration::from_millis(150));
        let settled = frames.get();
        pump(Duration::from_millis(200));
        assert_eq!(
            frames.get(),
            settled,
            "native patch starts an idle render loop"
        );
        eprintln!("Native separate patch {variant:?}: byte-exact, background reused, idle");
    }
    owner.imp().failed.set(true);
    queue_render(&area);
    drop(unsafe { snapshot(&area) });
    assert_eq!(region(&area), None);
    assert!(
        !owner
            .imp()
            .native
            .borrow()
            .as_ref()
            .unwrap()
            .window
            .is_visible()
    );
    for (a, store) in [(&area, &shared), (&reference, &normal)] {
        a.make_current();
        if let Some(r) = store.borrow_mut().take() {
            unsafe {
                r.destroy();
            }
        }
    }
    scene.destroy();
    reference_window.destroy();
}
#[test]
#[ignore = "requires isolated GTK/OpenGL; briefly maps an owned transparent test window"]
fn native_regional_snapshot_preserves_pixels_background_and_idle() {
    gtk::init().unwrap();
    let window = gtk::Window::new();
    window.set_default_size(960, 540);
    window.set_opacity(0.0);
    window.set_focusable(false);
    let area = new();
    area.set_required_version(3, 3);
    area.set_use_es(false);
    window.set_child(Some(&area));
    gtk::prelude::WidgetExt::realize(&window);
    area.realize();
    area.make_current();
    let shared: SharedRenderer = Rc::new(RefCell::new(None));
    bind(&area, &shared);
    let rendered = Rc::new(Cell::new(0));
    let count = rendered.clone();
    let r = shared.clone();
    area.connect_render(move |area, _| {
        count.set(count.get() + 1);
        let scale = area.scale_factor();
        let (w, h) = (area.width() * scale, area.height() * scale);
        let mut borrow = r.borrow_mut();
        let renderer = borrow.as_mut().unwrap();
        let changed = if let Some(region) = region(area) {
            unsafe { renderer.render_regional(w, h, 0.515625, region).unwrap() }
        } else {
            unsafe {
                renderer.render(w, h, 0.515625).unwrap();
            }
            false
        };
        if changed {
            refresh_background(area);
        }
        glib::Propagation::Stop
    });
    // Native GLArea baseline stays unmapped. It uses the same authored
    // pixels, artwork and arm pose but a separate context/GTK target.
    let reference_window = gtk::Window::new();
    let reference = gtk::GLArea::new();
    reference.set_required_version(3, 3);
    reference.set_auto_render(false);
    reference_window.set_child(Some(&reference));
    gtk::prelude::WidgetExt::realize(&reference_window);
    reference.realize();
    reference.make_current();
    let normal: SharedRenderer = Rc::new(RefCell::new(None));
    let r = normal.clone();
    reference.connect_render(move |area, _| {
        let scale = area.scale_factor();
        unsafe {
            r.borrow_mut()
                .as_mut()
                .unwrap()
                .render(area.width() * scale, area.height() * scale, 0.515625)
                .unwrap();
        }
        glib::Propagation::Stop
    });
    let artwork =
        image::RgbaImage::from_fn(256, 256, |x, y| image::Rgba([x as u8, y as u8, 80, 255]));
    for variant in [SceneVariant::Day, SceneVariant::Night] {
        let mut config = diagnostic_config();
        config.scene.mode = PreviewMode::Arm;
        config.scene.variant = variant;
        config.render.arm_interpolation = true;
        for (owner, store) in [(&area, &shared), (&reference, &normal)] {
            owner.make_current();
            if let Some(old) = store.borrow_mut().take() {
                unsafe {
                    old.destroy();
                }
            }
            let mut r = unsafe { Renderer::new(PreviewAsset::load(&config).unwrap()).unwrap() };
            unsafe {
                r.set_artwork(1, Some(&artwork), &config.artwork).unwrap();
            }
            *store.borrow_mut() = Some(r);
        }
        window.present();
        pump(Duration::from_millis(150));
        // A fully transparent diagnostic window may be skipped by GTK's
        // outer frame snapshot; explicitly exercise its class snapshot.
        drop(unsafe { snapshot(&area) });
        let scale = area.scale_factor();
        let (w, h) = (area.width() * scale, area.height() * scale);
        let imp = area.downcast_ref::<RegionalArea>().unwrap().imp();
        assert!(imp.region.get().is_some());
        assert_eq!(imp.animated.get().unwrap().context(), area.context());
        assert_eq!(imp.background.get().unwrap().context(), area.context());
        reference.allocate(area.width(), area.height(), -1, None);
        let gsk = window.renderer().unwrap();
        let viewport = gtk::graphene::Rect::new(0.0, 0.0, w as f32, h as f32);
        let pixels = |node: &gtk::gsk::RenderNode| {
            // render_texture has a 1× offscreen target. Reproduce the
            // window's physical scale, not a downsampled logical preview.
            let scaled = gtk::Snapshot::new();
            scaled.scale(scale as f32, scale as f32);
            scaled.append_node(node);
            let t = gsk.render_texture(scaled.to_node().unwrap(), Some(&viewport));
            let mut data = vec![0; t.width() as usize * t.height() as usize * 4];
            t.download(&mut data, t.width() as usize * 4);
            data
        };
        let mut held = Vec::new();
        let mut first_pixels = Vec::new();
        let mut background_draws = 0;
        for step in 0..6 {
            for (owner, store) in [(&area, &shared), (&reference, &normal)] {
                owner.make_current();
                let mut r = store.borrow_mut();
                let r = r.as_mut().unwrap();
                r.set_artwork_angle(step as f32 * 0.5);
                if step == 3 {
                    unsafe {
                        r.set_previous_artwork(1, Some(&artwork)).unwrap();
                    }
                }
                if step == 4 {
                    r.set_sleeve_visible(false);
                }
                owner.queue_render();
            }
            let actual = unsafe { snapshot(&area) };
            let baseline = unsafe { snapshot(&reference) };
            let difference = pixels(&actual)
                .iter()
                .zip(pixels(&baseline))
                .filter(|(a, b)| **a != *b)
                .count();
            assert_eq!(difference, 0, "GTK {variant:?} step {step}: output differs");
            if step == 0 {
                first_pixels = pixels(&actual);
            }
            if let Some(first) = held.first() {
                assert_eq!(
                    pixels(first),
                    first_pixels,
                    "GTK changed an already published texture"
                );
            }
            if matches!(step, 1 | 2 | 5) {
                assert_eq!(
                    imp.background_draws.get(),
                    background_draws,
                    "unchanged background repainted"
                );
            }
            background_draws = imp.background_draws.get();
            held.push(actual); // GTK must not overwrite previously exported texture storage.
        }
        eprintln!("Native GTK {variant:?} {w}x{h}: byte-exact, background reused");
        pump(Duration::from_millis(150));
        let settled = rendered.get();
        pump(Duration::from_millis(250));
        assert_eq!(
            rendered.get(),
            settled,
            "presenter starts an idle render loop"
        );
        assert!(!imp.failed.get());
        drop(held);
        // Inject failure to exercise the unchanged full-frame fallback.
        imp.failed.set(true);
        area.queue_render();
        reference.queue_render();
        assert_eq!(
            pixels(&unsafe { snapshot(&area) }),
            pixels(&unsafe { snapshot(&reference) })
        );
        assert!(region(&area).is_none());
        imp.failed.set(false);
    }
    for (owner, store) in [(&area, &shared), (&reference, &normal)] {
        owner.make_current();
        if let Some(r) = store.borrow_mut().take() {
            unsafe {
                r.destroy();
            }
        }
    }
    reference_window.destroy();
    window.destroy();
}
