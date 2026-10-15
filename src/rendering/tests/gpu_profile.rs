//! Controlled, unmapped GPU diagnostics. No MPRIS or desktop mutation.
use super::super::*;
use super::arm_cadence_tests::{owned_area, pixels};
use crate::config::{Config, PreviewMode, SceneVariant};
use gtk::prelude::*;
use std::time::{Duration, Instant};

fn config(variant: SceneVariant) -> Config {
    let mut config = Config::default();
    config.scene.variant = variant;
    config.scene.mode = PreviewMode::Arm;
    config.render.arm_interpolation = true;
    config
}
fn cover() -> image::RgbaImage {
    image::RgbaImage::from_fn(256, 256, |x, y| image::Rgba([x as u8, y as u8, 80, 255]))
}
/// Diagnostic tests only: finish before polling so assertions are deterministic.
unsafe fn drain(r: &mut Renderer) -> Vec<GpuFrameTiming> {
    unsafe {
        r.gl.finish();
    }
    r.source.profiler.as_mut().unwrap().poll();
    r.take_gpu_timings()
}

#[test]
#[ignore = "requires owned, unmapped GTK/OpenGL and day/night assets"]
fn profiling_preserves_pixels_cadence_and_tags_and_restores_query_buffer() {
    let (_window, area) = owned_area();
    for variant in [SceneVariant::Day, SceneVariant::Night] {
        let config = config(variant);
        let mut exact = unsafe { Renderer::new(PreviewAsset::load(&config).unwrap()).unwrap() };
        assert!(
            exact.source.profiler.is_none(),
            "normal use allocates no timer queries"
        );
        unsafe extern "system" fn unavailable_counter(_: u32, _: u32, bits: *mut i32) {
            unsafe {
                *bits = 0;
            }
        }
        // An unsupported optional counter is rejected before allocation; the
        // unprofiled renderer still runs the full pixel/cadence comparison.
        assert!(
            unsafe { gpu_profile::GpuProfiler::new(exact.gl.clone(), unavailable_counter, true) }
                .is_err()
        );
        let mut timed = unsafe { Renderer::new(PreviewAsset::load(&config).unwrap()).unwrap() };
        unsafe {
            timed.enable_gpu_timing(true);
        }
        assert!(timed.source.profiler.is_some());
        let now = Instant::now();
        let cover = cover();
        for r in [&mut exact, &mut timed] {
            unsafe {
                r.set_artwork(1, Some(&cover), &config.artwork).unwrap();
            }
        }
        for (w, h) in [(1920, 1080), (2560, 1440), (3840, 2160)] {
            area.allocate(w, h, -1, None);
            area.attach_buffers();
            for frame in 0..12 {
                let p = 0.51 + frame as f32 / 5400.0;
                let timestamp = now + Duration::from_secs_f64(frame as f64 / 30.0);
                for r in [&mut exact, &mut timed] {
                    r.set_arm_cache_target(Some(p));
                    r.set_artwork_angle(frame as f32 * 0.116);
                    if frame == 4 {
                        r.set_button_progress([0.5, 0.25, 0.75]);
                        r.set_knob_angle(0.4);
                        let mut settings = config.artwork.clone();
                        settings.sleeve_shadow_lift = 0.4;
                        unsafe {
                            r.set_artwork(1, Some(&cover), &settings).unwrap();
                        }
                    }
                }
                unsafe {
                    exact.source.render_at(w, h, p, timestamp).unwrap();
                }
                let reference = unsafe { pixels(&exact, w, h) };
                unsafe {
                    timed.source.render_at(w, h, p, timestamp).unwrap();
                }
                assert_eq!(
                    reference,
                    unsafe { pixels(&timed, w, h) },
                    "profiling changed pixels"
                );
                assert_eq!(exact.cache_update_counts(), timed.cache_update_counts());
                assert_eq!(exact.button_update_count(), timed.button_update_count());
                assert_eq!(exact.knob_update_count(), timed.knob_update_count());
            }
        }
        let buffer = unsafe { timed.gl.create_buffer().unwrap() };
        unsafe {
            timed.gl.bind_buffer(glow::QUERY_BUFFER, Some(buffer));
            timed
                .gl
                .buffer_data_size(glow::QUERY_BUFFER, 64, glow::DYNAMIC_READ);
        }
        let samples = unsafe { drain(&mut timed) };
        assert_eq!(
            unsafe { timed.gl.get_parameter_i32(glow::QUERY_BUFFER_BINDING) } as u32,
            buffer.0.get()
        );
        unsafe {
            timed.gl.bind_buffer(glow::QUERY_BUFFER, None);
            timed.gl.delete_buffer(buffer);
        }
        assert_eq!(samples.len(), 36);
        assert!(samples.windows(2).all(|s| s[0].info.id < s[1].info.id));
        for stage in GpuStage::ALL {
            assert!(
                samples
                    .iter()
                    .any(|s| s.stages_ms[stage as usize].is_some()),
                "missing {}",
                stage.name()
            );
        }
        for sample in &samples {
            assert_eq!(sample.info.scene, variant);
            assert!(sample.stages_ms[GpuStage::Presentation as usize].is_some());
            assert!(sample.stages_ms[GpuStage::Record as usize].is_some());
            assert!(sample.stages_ms.iter().flatten().sum::<f64>() <= sample.total_ms + 0.0001);
        }
        assert_eq!(samples[0].info.dimensions, (1920, 1080));
        assert_eq!(samples[12].info.dimensions, (2560, 1440));
        assert_eq!(samples[24].info.dimensions, (3840, 2160));
        assert!(
            samples[0].info.transition
                && samples[12].info.transition
                && samples[24].info.transition
        );
        assert!(
            samples
                .iter()
                .any(|s| s.stages_ms[GpuStage::Arm as usize].is_none() && !s.info.transition)
        );
        let mut disabled = config.artwork.clone();
        disabled.enabled = false;
        disabled.vinyl_rotation = false;
        unsafe {
            timed.set_artwork(2, None, &disabled).unwrap();
            timed.render(3840, 2160, 0.515).unwrap();
        }
        let sample = unsafe { drain(&mut timed) }.pop().unwrap();
        assert!(sample.stages_ms[GpuStage::Record as usize].is_none());
        // Failed framebuffer allocation is tagged invalid; next frame recovers.
        unsafe { timed.source.render_at(i32::MAX, 2160, 0.515, now) }.unwrap_err();
        unsafe {
            timed.render(3840, 2160, 0.515).unwrap();
        }
        assert_eq!(unsafe { drain(&mut timed) }.len(), 1);
        assert_eq!(timed.source.profiler.as_ref().unwrap().invalid, 1);
        unsafe {
            timed.check_error().unwrap();
            exact.check_error().unwrap();
        }
        // Teardown with a pending sample must not wait or reuse the next scene's pool.
        unsafe {
            timed.render(3840, 2160, 0.516).unwrap();
            timed.destroy();
            exact.destroy();
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Workload {
    Playing,
    Artwork,
    Dust,
    PresentationOnly,
    Seeking,
}

fn describe(values: &[f64]) -> (f64, f64) {
    assert!(!values.is_empty());
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    (
        values.iter().sum::<f64>() / values.len() as f64,
        sorted[((values.len() - 1) as f64 * 0.95).ceil() as usize],
    )
}

/// Six real seconds at 30 FPS: two warm-up, four measured. This explicit test
/// cadence is not an application scheduler and never talks to the session bus.
unsafe fn paced_run(
    r: &mut Renderer,
    config: &Config,
    dimensions: (i32, i32),
    workload: Workload,
    timing: Option<bool>,
    outer: bool,
    repeat: usize,
) {
    let (w, h) = dimensions;
    r.source.profiler = None;
    if let Some(detailed) = timing {
        unsafe {
            r.enable_gpu_timing(detailed);
        }
        assert!(r.source.profiler.is_some());
    }
    let mut settings = config.artwork.clone();
    settings.enabled = matches!(
        workload,
        Workload::Playing | Workload::Artwork | Workload::Seeking
    );
    settings.vinyl_rotation = matches!(
        workload,
        Workload::Playing | Workload::Dust | Workload::Seeking
    );
    let cover = cover();
    unsafe {
        // Switching diagnostic workloads changes the image/None revision too;
        // otherwise a run starting with dust-only never uploads its later cover.
        r.set_artwork(
            1 + u64::from(settings.enabled),
            settings.enabled.then_some(&cover),
            &settings,
        )
        .unwrap();
    }
    let query = outer.then(|| unsafe { r.gl.create_query().unwrap() });
    let started = Instant::now();
    let mut motion = crate::arm::motion::ArmMotion::new(0.51, config.render.arm_response_seconds);
    let mut samples = Vec::new();
    let mut outside = Vec::new();
    let mut cpu = Vec::new();
    for frame in 0..180 {
        let deadline = started + Duration::from_secs_f64(frame as f64 / 30.0);
        std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
        let now = Instant::now();
        let (p, target) = match workload {
            Workload::Playing => {
                let p = 0.51 + frame as f32 / 5400.0;
                (p, Some(p))
            }
            Workload::Seeking => {
                let target = if frame / 30 % 2 == 0 { 0.9 } else { 0.1 };
                motion.retarget(target);
                (
                    motion.advance(Duration::from_secs_f64(1.0 / 30.0)),
                    Some(target),
                )
            }
            _ => (0.515, None),
        };
        r.set_arm_cache_target(target);
        r.set_artwork_angle(frame as f32 * 0.116355);
        if let Some(query) = query {
            unsafe {
                r.gl.begin_query(glow::TIME_ELAPSED, query);
            }
        }
        let submission = Instant::now();
        unsafe {
            r.source.render_at(w, h, p, now).unwrap();
        }
        if frame >= 60 {
            cpu.push(submission.elapsed().as_secs_f64() * 1000.0);
        }
        unsafe {
            if let Some(query) = query {
                r.gl.end_query(glow::TIME_ELAPSED);
                // Only the opt-in isolated OVERHEAD test waits for its outer
                // control timer. Live profiling and stage runs do not wait.
                r.gl.finish();
                let mut ns = 0_u64;
                r.gl.get_query_parameter_u64_with_offset(
                    query,
                    glow::QUERY_RESULT,
                    &mut ns as *mut u64 as usize,
                );
                if frame >= 60 {
                    outside.push(ns as f64 / 1_000_000.0);
                }
            } else {
                r.gl.flush();
            }
        }
        samples.extend(r.take_gpu_timings().into_iter().filter(|s| s.info.id > 60));
    }
    if r.source.profiler.is_some() {
        samples.extend(unsafe { drain(r) }.into_iter().filter(|s| s.info.id > 60));
    }
    if let Some(query) = query {
        unsafe {
            r.gl.delete_query(query);
        }
    }
    unsafe {
        r.check_error().unwrap();
    }
    let (cpu_avg, cpu_p95) = describe(&cpu);
    let detailed = timing == Some(true);
    if outer {
        let (avg, p95) = describe(&outside);
        println!(
            "GPU_OVERHEAD scene={:?} {w}x{h} repeat={repeat} mode={timing:?} frames={} avg_ms={avg:.6} p95_ms={p95:.6} cpu_avg_ms={cpu_avg:.6} cpu_p95_ms={cpu_p95:.6}",
            config.scene.variant,
            outside.len()
        );
    }
    if !samples.is_empty() {
        let profiler = r.source.profiler.as_ref().unwrap();
        let (avg, p95) = describe(&samples.iter().map(|s| s.total_ms).collect::<Vec<_>>());
        println!(
            "GPU_TOTAL scene={:?} {w}x{h} workload={workload:?} repeat={repeat} detailed={detailed} frames={} avg_ms={avg:.6} p95_ms={p95:.6} dropped={} invalid={} cpu_avg_ms={cpu_avg:.6} cpu_p95_ms={cpu_p95:.6}",
            config.scene.variant,
            samples.len(),
            profiler.dropped,
            profiler.invalid
        );
        assert_eq!(profiler.invalid, 0);
        assert_eq!(
            samples.len(),
            120,
            "incomplete samples: report saturation, don't silently extrapolate"
        );
        if detailed {
            for stage in GpuStage::ALL {
                let values: Vec<_> = samples
                    .iter()
                    .filter_map(|s| s.stages_ms[stage as usize])
                    .collect();
                let (avg, p95) = if values.is_empty() {
                    (0.0, 0.0)
                } else {
                    describe(&values)
                };
                let contribution =
                    values.iter().fold(0.0, |sum, value| sum + value) / samples.len() as f64;
                println!(
                    "GPU_STAGE scene={:?} {w}x{h} workload={workload:?} repeat={repeat} stage={} active={} avg_ms={avg:.6} p95_ms={p95:.6} contribution_ms_per_frame={contribution:.6} ms_per_second_at_30fps={:.6}",
                    config.scene.variant,
                    stage.name(),
                    values.len(),
                    contribution * 30.0
                );
            }
        }
    }
}

#[test]
#[ignore = "paced controlled GPU matrix (~3 min); unmapped, no live playback"]
fn gpu_stage_paced_matrix() {
    let (_window, area) = owned_area();
    for variant in [SceneVariant::Day, SceneVariant::Night] {
        let config = config(variant);
        let mut r = unsafe { Renderer::new(PreviewAsset::load(&config).unwrap()).unwrap() };
        println!("GPU_DEVICE {}", unsafe {
            r.gl.get_parameter_string(glow::RENDERER)
        });
        for (w, h) in [(2560, 1440), (1920, 1080), (3840, 2160)] {
            area.allocate(w, h, -1, None);
            area.attach_buffers();
            for repeat in 0..3 {
                unsafe {
                    paced_run(
                        &mut r,
                        &config,
                        (w, h),
                        Workload::Playing,
                        Some(true),
                        false,
                        repeat,
                    );
                }
            }
            if w == 2560 {
                for workload in [
                    Workload::Artwork,
                    Workload::Dust,
                    Workload::PresentationOnly,
                    Workload::Seeking,
                ] {
                    unsafe {
                        paced_run(&mut r, &config, (w, h), workload, Some(true), false, 0);
                    }
                }
            }
        }
        unsafe {
            r.destroy();
        }
    }
}

#[test]
#[ignore = "paced profiler A/B overhead (~1 min), unmapped; outer diagnostic timer waits"]
fn gpu_profiler_paced_overhead() {
    let (_window, area) = owned_area();
    let config = config(SceneVariant::Day);
    let mut r = unsafe { Renderer::new(PreviewAsset::load(&config).unwrap()).unwrap() };
    area.allocate(2560, 1440, -1, None);
    area.attach_buffers();
    // ABBA: compare detailed vs total-only and vs no internal queries, always
    // with the SAME external control query. Clocks remain uncontrolled.
    for (repeat, timing) in [
        None,
        Some(true),
        Some(true),
        None,
        Some(false),
        Some(true),
        Some(true),
        Some(false),
    ]
    .into_iter()
    .enumerate()
    {
        unsafe {
            paced_run(
                &mut r,
                &config,
                (2560, 1440),
                Workload::Playing,
                timing,
                true,
                repeat,
            );
        }
    }
    unsafe {
        r.destroy();
    }
}

#[test]
#[ignore = "paced original/trimmed arm ABBA (~2 min), owned unmapped GPU context"]
fn gpu_arm_region_paced_comparison() {
    let (_window, area) = owned_area();
    area.allocate(2560, 1440, -1, None);
    area.attach_buffers();
    for variant in [SceneVariant::Day, SceneVariant::Night] {
        let config = config(variant);
        let mut r = unsafe { Renderer::new(PreviewAsset::load(&config).unwrap()).unwrap() };
        println!("GPU_DEVICE {}", unsafe {
            r.gl.get_parameter_string(glow::RENDERER)
        });
        for workload in [Workload::Playing, Workload::Seeking] {
            for (repeat, width) in [960, 600, 600, 960].into_iter().enumerate() {
                unsafe {
                    r.source.test_arm_region([
                        1160.0 / 2560.0,
                        585.0 / 1440.0,
                        width as f32 / 2560.0,
                        630.0 / 1440.0,
                    ]);
                }
                println!(
                    "GPU_ARM_REGION scene={variant:?} workload={workload:?} repeat={repeat} crop_width={width}"
                );
                unsafe {
                    paced_run(
                        &mut r,
                        &config,
                        (2560, 1440),
                        workload,
                        Some(true),
                        false,
                        repeat,
                    );
                }
            }
        }
        unsafe {
            r.destroy();
        }
    }
}
