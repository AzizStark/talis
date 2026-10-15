//! Actual GLSL ramp and owned, unmapped scene checks for sleeve-only shadow lift.
use super::super::*;
use crate::config::{Config, PreviewMode, SceneVariant};
use gtk::prelude::*;

unsafe fn check_shadow_ramp(renderer: &Renderer) {
    unsafe {
        let gl = &renderer.gl;
        let previous = gl.get_parameter_framebuffer(glow::DRAW_FRAMEBUFFER_BINDING);
        let texture = gl.create_texture().unwrap();
        let framebuffer = gl.create_framebuffer().unwrap();
        let vao = gl.create_vertex_array().unwrap();
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA32F as i32,
            256,
            3,
            0,
            glow::RGBA,
            glow::FLOAT,
            glow::PixelUnpackData::Slice(None),
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::NEAREST as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::NEAREST as i32,
        );
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
        let fragment = format!(
            "#version 330 core\nuniform sampler2D uArtwork;\n{}\n
            out vec4 fragColor;
            void main() {{
                float t = (gl_FragCoord.x-0.5)/255.0;
                float a = gl_FragCoord.y>1.0 && gl_FragCoord.y<2.0 ? 0.25 : 1.0;
                vec3 c = mix(vec3(0.08,0.10,0.16),uSleeveLightReference,t);
                if (gl_FragCoord.y>2.0) c = uSleeveLightReference*1.25;
                fragColor = vec4(sleeveLighting(vec4(c*a,a)),a);
            }}",
            include_str!("../../../shaders/sleeve.glsl")
        );
        let program =
            utils::compile(gl, include_str!("../../../shaders/beauty.vert"), &fragment).unwrap();
        let reference = [0.8, 0.7, 0.55];
        gl.use_program(Some(program));
        gl.uniform_3_f32(
            gl.get_uniform_location(program, "uSleeveLightReference")
                .as_ref(),
            reference[0],
            reference[1],
            reference[2],
        );
        gl.disable(glow::SCISSOR_TEST);
        gl.disable(glow::BLEND);
        gl.disable(glow::FRAMEBUFFER_SRGB);
        gl.viewport(0, 0, 256, 3);
        gl.bind_vertex_array(Some(vao));
        for lift in [0.0, 0.25, 0.5, 0.75, 1.0] {
            gl.uniform_1_f32(
                gl.get_uniform_location(program, "uSleeveShadowLift")
                    .as_ref(),
                lift,
            );
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            let mut bytes = vec![0; 256 * 3 * 4 * 4];
            gl.read_pixels(
                0,
                0,
                256,
                3,
                glow::RGBA,
                glow::FLOAT,
                glow::PixelPackData::Slice(Some(&mut bytes)),
            );
            let pixels: Vec<f32> = bytes
                .chunks_exact(4)
                .map(|b| f32::from_ne_bytes(b.try_into().unwrap()))
                .collect();
            for y in 0..3 {
                let alpha = if y == 1 { 0.25 } else { 1.0 };
                for x in 0..256 {
                    for c in 0..3 {
                        let value = pixels[(y * 256 + x) * 4 + c];
                        assert!(value.is_finite());
                        if x > 0 && y < 2 {
                            assert!(
                                value + 1e-6 >= pixels[(y * 256 + x - 1) * 4 + c],
                                "shadow gradient reversed at lift {lift}"
                            );
                        }
                        let t = x as f32 / 255.0;
                        let original = [0.08, 0.10, 0.16][c] * (1.0 - t) + reference[c] * t;
                        if lift == 0.0 && y < 2 {
                            assert!(
                                (value - original * alpha).abs() < 1e-5,
                                "zero lift changed authored light"
                            );
                        }
                        if lift == 1.0 && y < 2 {
                            assert!(
                                (value - reference[c] * alpha).abs() < 1e-5,
                                "full lift retained a cool shadow contour"
                            );
                        }
                        if y == 2 {
                            assert!(
                                (value - reference[c] * 1.25).abs() < 1e-5,
                                "lift altered bright illumination"
                            );
                        }
                    }
                }
            }
        }
        renderer.check_error().unwrap();
        gl.bind_vertex_array(None);
        gl.use_program(None);
        gl.bind_framebuffer(glow::FRAMEBUFFER, previous);
        gl.bind_texture(glow::TEXTURE_2D, None);
        gl.delete_program(program);
        gl.delete_vertex_array(vao);
        gl.delete_framebuffer(framebuffer);
        gl.delete_texture(texture);
    }
}

#[test]
#[ignore = "requires isolated GTK/OpenGL and matched day/night sleeve packages"]
fn sleeve_shadow_lift_is_monotonic_tint_safe_and_region_local() {
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
    let output = std::env::var_os("TALIS_SLEEVE_LIFT_CAPTURE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| fixture.root.clone());
    std::fs::create_dir_all(&output).unwrap();
    let white = image::RgbaImage::from_pixel(64, 64, image::Rgba([255, 255, 255, 255]));
    let black = image::RgbaImage::from_pixel(64, 64, image::Rgba([0, 0, 0, 255]));
    for variant in [SceneVariant::Day, SceneVariant::Night] {
        let mut config = Config::default();
        config.scene.variant = variant;
        config.scene.mode = PreviewMode::Arm;
        config.render.arm_interpolation = true;
        let asset = PreviewAsset::load(&config).unwrap();
        let crop = asset.sleeve.as_ref().unwrap().manifest.crop;
        eprintln!(
            "{variant:?} bright-paper RGB {:?}",
            asset.sleeve.as_ref().unwrap().light_reference
        );
        let mut renderer = unsafe { Renderer::new(asset).unwrap() };
        if variant == SceneVariant::Day {
            unsafe { check_shadow_ramp(&renderer) };
        }
        let mut settings = config.artwork.clone();
        settings.enabled = true;
        settings.vinyl_rotation = false;
        settings.sleeve_shadow_lift = 0.0;
        let capture = |r: &mut Renderer, name: &str| {
            let path = output.join(format!("{}-{name}.png", variant.id()));
            unsafe {
                r.render(2560, 1440, 0.515625).unwrap();
                r.check_error().unwrap();
                r.capture(2560, 1440, &path).unwrap();
            }
            image::open(path).unwrap().into_rgba8()
        };
        unsafe { renderer.set_artwork(1, Some(&white), &settings).unwrap() };
        let original = capture(&mut renderer, "lift-0");
        let initial = renderer.cache_update_counts();
        for (step, lift) in [0.25, 0.5, 0.75, 1.0].into_iter().enumerate() {
            settings.sleeve_shadow_lift = lift;
            unsafe { renderer.set_artwork(1, Some(&white), &settings).unwrap() };
            let lifted = capture(&mut renderer, &format!("lift-{lift}"));
            assert!(
                lifted != original,
                "lift had no visible effect on the sleeve"
            );
            for (x, y, pixel) in lifted.enumerate_pixels() {
                if x < crop[0] || x >= crop[0] + crop[2] || y < crop[1] || y >= crop[1] + crop[3] {
                    assert_eq!(
                        pixel,
                        original.get_pixel(x, y),
                        "shadow lift changed other scene pixels"
                    );
                }
            }
            let counts = renderer.cache_update_counts();
            assert_eq!(counts, (initial.0, initial.1, initial.2 + step as u64 + 1));
            assert!(
                capture(&mut renderer, "cached") == lifted,
                "cached frame changed"
            );
            assert_eq!(renderer.cache_update_counts(), counts);
        }
        settings.sleeve_shadow_lift = 0.0;
        unsafe { renderer.set_artwork(1, Some(&white), &settings).unwrap() };
        assert!(
            capture(&mut renderer, "restored") == original,
            "zero lift did not restore authored shading"
        );
        unsafe { renderer.set_artwork(2, Some(&black), &settings).unwrap() };
        let dark = capture(&mut renderer, "black-0");
        settings.sleeve_shadow_lift = 1.0;
        unsafe { renderer.set_artwork(2, Some(&black), &settings).unwrap() };
        assert!(
            capture(&mut renderer, "black-1") == dark,
            "lift changed black ink/residual highlights"
        );
        unsafe { renderer.destroy() };
    }
    window.destroy();
}
