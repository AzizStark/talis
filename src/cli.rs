//! Command-line parsing and validation, independent of launch orchestration.
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about = "Talis — a music-aware Linux desktop turntable")]
pub(crate) struct Args {
    /// Load a TOML configuration; otherwise use built-in defaults.
    #[arg(long)]
    pub(crate) config: Option<PathBuf>,
    /// Ignore saved preferences and disable autosaving for this launch.
    #[arg(long)]
    pub(crate) no_user_settings: bool,
    /// Owned login launch baseline; current preferences override its visual values.
    #[arg(long, hide = true, requires = "desktop", conflicts_with = "config")]
    pub(crate) startup_config: Option<PathBuf>,
    #[arg(long, hide = true, requires = "startup_config", value_parser = parse_player)]
    pub(crate) startup_player: Option<String>,
    /// Select the authored lighting package.
    #[arg(long, value_parser = ["day", "night"])]
    pub(crate) scene: Option<String>,
    /// Validate settings and report the session without opening a window.
    #[arg(long)]
    pub(crate) check: bool,
    /// Fullscreen preview only, not desktop-layer placement.
    #[arg(long)]
    pub(crate) fullscreen: bool,
    /// Desktop placement; X11 by default, Wayland requires --experimental-wayland.
    #[arg(long, conflicts_with_all = ["fullscreen", "size"])]
    pub(crate) desktop: bool,
    /// Explicit experimental layer-shell desktop; requires optional wayland-desktop build.
    #[arg(long, requires = "desktop", conflicts_with_all = ["native_patch", "regional_presenter"])]
    pub(crate) experimental_wayland: bool,
    /// Desktop monitor index (0-based); defaults to the first GDK monitor.
    #[arg(long, requires = "desktop")]
    pub(crate) monitor: Option<u32>,
    /// Do not open the separate controls window (also useful for benchmarks).
    #[arg(long)]
    pub(crate) scene_only: bool,
    /// Start the settings window minimized (used for desktop login startup).
    #[arg(long, conflicts_with = "scene_only")]
    pub(crate) start_minimized: bool,
    /// Preview window size in GTK logical pixels, e.g. 2560x1440.
    #[arg(long, value_parser = parse_size)]
    pub(crate) size: Option<(i32, i32)>,
    /// Automatically close the preview for a rendering smoke check.
    #[arg(long)]
    pub(crate) smoke_seconds: Option<u32>,
    /// Load an already display-encoded Blender beauty PNG (static fidelity baseline).
    #[arg(long)]
    pub(crate) background: Option<PathBuf>,
    /// Photorealistic arm animation using prepared scene poses.
    #[arg(long, conflicts_with = "background")]
    pub(crate) arm: bool,
    /// Generate intermediate arm images live using cached optical-flow maps.
    #[arg(long, requires = "arm")]
    pub(crate) arm_flow: bool,
    /// Static background plus regional GTK presentation (Arm mode).
    #[arg(long, requires = "arm", conflicts_with = "capture")]
    pub(crate) regional_presenter: bool,
    /// Separate native X11 animated patch over a static desktop.
    #[arg(long, requires_all = ["arm", "desktop"], conflicts_with_all = ["capture", "regional_presenter"])]
    pub(crate) native_patch: bool,
    /// Start the reversible arm demo immediately (no music API).
    #[arg(long, requires = "arm")]
    pub(crate) arm_demo: bool,
    /// Initial normalized arm position: 0 outer groove, 1 inner groove.
    #[arg(long, requires = "arm", value_parser = parse_progress)]
    pub(crate) arm_progress: Option<f32>,
    /// Follow MPRIS timing and enable explicit turntable playback controls.
    #[arg(long, requires = "arm", conflicts_with_all = ["arm_demo", "arm_progress"])]
    pub(crate) now_playing: bool,
    /// Preferred MPRIS player (e.g. spotify), otherwise prefer the playing source.
    #[arg(long, requires = "now_playing", value_parser = parse_player)]
    pub(crate) player: Option<String>,
    /// Continuously redraw the scene to measure active rendering cost.
    #[arg(long)]
    pub(crate) benchmark: bool,
    /// Log asynchronous per-stage GPU timings without forcing redraws or exact arm updates.
    #[arg(long)]
    pub(crate) gpu_profile: bool,
    /// Requested rendering frame-rate cap (1..120).
    #[arg(long)]
    pub(crate) fps: Option<u32>,
    /// Capture the real GL framebuffer as a PNG (diagnostic readback).
    #[arg(long)]
    pub(crate) capture: Option<PathBuf>,
}
fn parse_player(value: &str) -> std::result::Result<String, String> {
    talis::preferences::normalize_player(value).map_err(|error| error.to_string())
}
fn parse_progress(value: &str) -> std::result::Result<f32, String> {
    let progress = value
        .parse::<f32>()
        .map_err(|_| "Use a number from 0 to 1")?;
    if !progress.is_finite() || !(0.0..=1.0).contains(&progress) {
        return Err("Arm progress must be a finite number from 0 to 1".into());
    }
    Ok(progress)
}

fn parse_size(value: &str) -> std::result::Result<(i32, i32), String> {
    let (width, height) = value
        .split_once('x')
        .ok_or("Use WIDTHxHEIGHT, e.g. 2560x1440")?;
    let width = width.parse::<i32>().map_err(|_| "Invalid width")?;
    let height = height.parse::<i32>().map_err(|_| "Invalid height")?;
    if !(1..=16384).contains(&width) || !(1..=16384).contains(&height) {
        return Err("Each dimension must be between 1 and 16384 logical pixels".into());
    }
    Ok((width, height))
}

#[cfg(test)]
mod tests {
    #[test]
    fn preference_isolation_and_owned_startup_flags_are_explicit() {
        use clap::Parser;
        assert!(
            super::Args::try_parse_from(["talis", "--no-user-settings"])
                .unwrap()
                .no_user_settings
        );
        assert!(
            super::Args::try_parse_from(["talis", "--startup-config", "/tmp/settings.toml"])
                .is_err()
        );
        assert!(
            super::Args::try_parse_from([
                "talis",
                "--desktop",
                "--startup-config",
                "/tmp/settings.toml",
                "--config",
                "/tmp/other.toml"
            ])
            .is_err()
        );
        let args = super::Args::try_parse_from([
            "talis",
            "--desktop",
            "--arm",
            "--now-playing",
            "--startup-config",
            "/tmp/settings.toml",
            "--startup-player",
            "spotify",
        ])
        .unwrap();
        assert_eq!(
            args.startup_player.as_deref(),
            Some("org.mpris.MediaPlayer2.spotify")
        );
    }
    #[test]
    fn wayland_requires_explicit_desktop_and_baseline_presenter() {
        use clap::Parser;
        assert!(super::Args::try_parse_from(["talis", "--experimental-wayland"]).is_err());
        assert!(
            super::Args::try_parse_from(["talis", "--desktop", "--experimental-wayland"]).is_ok()
        );
        for presenter in ["--native-patch", "--regional-presenter"] {
            assert!(
                super::Args::try_parse_from([
                    "talis",
                    "--desktop",
                    "--experimental-wayland",
                    "--arm",
                    presenter
                ])
                .is_err()
            );
        }
    }
    use super::*;
    #[test]
    fn native_patch_is_explicit_desktop_arm_only() {
        assert!(!Args::try_parse_from(["talis"]).unwrap().native_patch);
        assert!(Args::try_parse_from(["talis", "--native-patch", "--arm"]).is_err());
        assert!(Args::try_parse_from(["talis", "--native-patch", "--desktop"]).is_err());
        assert!(
            Args::try_parse_from(["talis", "--native-patch", "--desktop", "--arm"])
                .unwrap()
                .native_patch
        );
        for flag in ["--regional-presenter", "--capture"] {
            let mut args = vec!["talis", "--native-patch", "--desktop", "--arm", flag];
            if flag == "--capture" {
                args.push("/tmp/capture.png");
            }
            assert!(Args::try_parse_from(args).is_err());
        }
    }
    #[test]
    fn regional_presentation_is_opt_in_arm_only_and_capture_uses_full_target() {
        assert!(!Args::try_parse_from(["talis"]).unwrap().regional_presenter);
        assert!(Args::try_parse_from(["talis", "--regional-presenter"]).is_err());
        assert!(
            Args::try_parse_from(["talis", "--arm", "--regional-presenter"])
                .unwrap()
                .regional_presenter
        );
        assert!(
            Args::try_parse_from([
                "talis",
                "--arm",
                "--regional-presenter",
                "--capture",
                "/tmp/capture.png"
            ])
            .is_err()
        );
    }
    #[test]
    fn gpu_profile_does_not_enable_benchmark_scheduling() {
        let defaults = Args::try_parse_from(["talis"]).unwrap();
        assert!(!defaults.gpu_profile);
        let args = Args::try_parse_from(["talis", "--gpu-profile"]).unwrap();
        assert!(args.gpu_profile);
        assert!(!args.benchmark);
    }
    #[test]
    fn lighting_variant_is_explicit_and_independent_of_motion_mode() {
        for variant in ["day", "night"] {
            assert_eq!(
                Args::try_parse_from(["talis", "--scene", variant])
                    .unwrap()
                    .scene
                    .as_deref(),
                Some(variant)
            );
            assert!(
                Args::try_parse_from(["talis", "--arm", "--arm-flow", "--scene", variant]).is_ok()
            );
        }
        assert!(Args::try_parse_from(["talis", "--scene", "unknown"]).is_err());
    }
    #[test]
    fn music_mode_and_player_names_are_explicit() {
        let args = Args::try_parse_from(["talis", "--arm", "--now-playing", "--player", "spotify"])
            .unwrap();
        assert_eq!(
            args.player.as_deref(),
            Some("org.mpris.MediaPlayer2.spotify")
        );
        assert!(Args::try_parse_from(["talis", "--now-playing"]).is_err());
        assert!(Args::try_parse_from(["talis", "--player", "spotify"]).is_err());
        assert!(Args::try_parse_from(["talis", "--arm", "--now-playing", "--arm-demo"]).is_err());
        assert!(
            Args::try_parse_from(["talis", "--arm", "--now-playing", "--arm-progress", "0.5"])
                .is_err()
        );
        for name in ["", "bad/name", "bad..name", "123", "invalid name"] {
            assert!(parse_player(name).is_err());
        }
        assert!(parse_player("firefox.instance_123").is_ok());
    }
    #[test]
    fn preview_size_is_bounded_and_unambiguous() {
        assert_eq!(parse_size("2560x1440").unwrap(), (2560, 1440));
        for value in ["0x720", "1920", "-1x1080", "99999x99999", "1920x1080x2"] {
            assert!(parse_size(value).is_err());
        }
    }
    #[test]
    fn incompatible_preview_selections_are_rejected() {
        for removed in ["--raster", "--placeholder", "--model"] {
            assert!(Args::try_parse_from(["talis", removed]).is_err());
        }
        assert!(Args::try_parse_from(["talis", "--arm", "--background", "x.png"]).is_err());
        assert!(Args::try_parse_from(["talis", "--arm-demo"]).is_err());
        assert!(Args::try_parse_from(["talis", "--arm-flow"]).is_err());
        assert!(
            Args::try_parse_from(["talis", "--arm", "--arm-flow"])
                .unwrap()
                .arm_flow
        );
        assert!(parse_progress("NaN").is_err());
        assert!(parse_progress("1.1").is_err());
        assert_eq!(parse_progress("0.5").unwrap(), 0.5);
    }
    #[test]
    fn desktop_arguments_are_explicit_and_bounded() {
        assert!(
            Args::try_parse_from(["talis", "--desktop", "--start-minimized"])
                .unwrap()
                .start_minimized
        );
        assert!(Args::try_parse_from(["talis", "--start-minimized", "--scene-only"]).is_err());
        assert!(Args::try_parse_from(["talis", "--monitor", "1"]).is_err());
        assert!(Args::try_parse_from(["talis", "--desktop", "--fullscreen"]).is_err());
        assert!(Args::try_parse_from(["talis", "--desktop", "--size", "1280x720"]).is_err());
        assert!(Args::try_parse_from(["talis", "--desktop", "--monitor", "-1"]).is_err());
        assert_eq!(
            Args::try_parse_from(["talis", "--desktop", "--monitor", "1"])
                .unwrap()
                .monitor,
            Some(1)
        );
    }
}
