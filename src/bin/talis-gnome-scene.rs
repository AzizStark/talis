//! Opt-in owned GNOME scene source. Reuse the actual app/render/MPRIS pipeline.
use anyhow::{Context, Result, ensure};
use clap::{Parser, ValueEnum};
use std::path::PathBuf;
use talis::config::{Config, PreviewMode, ScenePackage, SceneVariant};

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq)]
enum Mode {
    Music,
    Demo,
    Static,
}

#[derive(Parser)]
struct Args {
    /// Asset root containing the selected scenes/1440p/{day,night} package.
    #[arg(long)]
    asset_root: PathBuf,
    #[arg(long, default_value = "day", value_parser = ["day", "night"])]
    scene: String,
    /// Music observes MPRIS; demo animates the arm without querying players.
    #[arg(long, value_enum, default_value = "music")]
    mode: Mode,
    #[arg(long, default_value = "1280x720", value_parser = size)]
    size: (i32, i32),
    /// Host diagnostic only; creates an ordinary preview, not a Shell surface.
    #[arg(long)]
    standalone: bool,
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..=300))]
    seconds: Option<u32>,
    /// One-time image readback for standalone quality checks, never the bridge.
    #[arg(long, requires = "standalone")]
    capture: Option<PathBuf>,
    /// Validate the selected package without starting GTK or accessing players.
    #[arg(long)]
    check: bool,
    /// Ignore saved preferences and disable autosave for isolated diagnostics.
    #[arg(long)]
    no_user_settings: bool,
}

fn size(value: &str) -> std::result::Result<(i32, i32), String> {
    let (w, h) = value.split_once('x').ok_or("Use WIDTHxHEIGHT")?;
    let w = w.parse::<i32>().map_err(|_| "Invalid width")?;
    let h = h.parse::<i32>().map_err(|_| "Invalid height")?;
    if !(1..=8192).contains(&w) || !(1..=8192).contains(&h) {
        return Err("Each dimension must be between 1 and 8192".into());
    }
    Ok((w, h))
}

fn config(args: &Args) -> Result<Config> {
    let root = args
        .asset_root
        .canonicalize()
        .context("Resolve packaged asset root")?;
    let mut config = Config::default();
    let package = ScenePackage::at(&root.join("scenes/1440p").join(&args.scene));
    ensure!(
        package.background.is_file(),
        "Selected scene package is missing"
    );
    config.scene.mode = PreviewMode::Arm;
    config.scene.variant = if args.scene == "night" {
        SceneVariant::Night
    } else {
        SceneVariant::Day
    };
    if config.scene.variant == SceneVariant::Day {
        config.scene.background = package.background;
        config.scene.arm_poses = package.arm_poses;
        config.scene.arm_flow = package.arm_flow;
        config.scene.sleeve = package.sleeve;
        config.scene.record_lighting = package.record_lighting;
        config.scene.vinyl_detail = package.vinyl_detail;
        config.scene.buttons = package.buttons;
        config.scene.night = None;
    } else {
        config.scene.night = Some(package);
    }
    config.render.arm_interpolation = true;
    // Commands still require explicit input and the existing MPRIS capability gates.
    config.transport.enabled = args.mode == Mode::Music;
    config.validate()?;
    Ok(config)
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "talis=info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let args = Args::parse();
    let packaged = config(&args)?;
    let enabled = !args.no_user_settings
        && !args.standalone
        && args.seconds.is_none()
        && args.capture.is_none()
        && !args.check
        && args.mode == Mode::Music;
    let loaded = talis::preferences::load_user(Config::default(), None, enabled)?;
    let player = loaded.document.player()?;
    let mut config = loaded.config;
    // The Shell preset supplies an explicit package/scene, not user asset paths.
    config.scene = packaged.scene;
    config.render.arm_interpolation = packaged.render.arm_interpolation;
    config.transport = packaged.transport;
    config.validate()?;
    let session = talis::preferences::Session {
        store: if enabled { loaded.store } else { None },
        issue: loaded.issue,
    };
    if args.check {
        talis::preview::PreviewAsset::load(&config)?;
        println!(
            "Selected {} scene package validated; no player access",
            args.scene
        );
        return Ok(());
    }
    talis::app::run_with_preferences(
        config,
        talis::app::Options {
            independent: true,
            fullscreen: false,
            desktop: false,
            experimental_wayland: false,
            monitor: 0,
            smoke_seconds: args.seconds,
            benchmark: false,
            gpu_profile: false,
            scene_only: args.standalone,
            shell_surface: !args.standalone,
            regional_presenter: args.capture.is_none(),
            native_patch: false,
            start_minimized: false,
            size: Some(args.size),
            capture: args.capture,
            arm_progress: 0.0,
            arm_demo: args.mode == Mode::Demo,
            now_playing: args.mode == Mode::Music,
            player,
        },
        Some(session),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_cli_and_readback_is_explicit() {
        assert_eq!(size("1920x1080").unwrap(), (1920, 1080));
        for invalid in ["0x720", "8193x720", "1x-2", "NaNx720", "720"] {
            assert!(size(invalid).is_err());
        }
        assert!(
            Args::try_parse_from([
                "talis-gnome-scene",
                "--asset-root",
                ".",
                "--capture",
                "test.png"
            ])
            .is_err()
        );
        let args = Args::try_parse_from(["talis-gnome-scene", "--asset-root", "."]).unwrap();
        assert!(!args.standalone);
        assert_eq!(args.mode, Mode::Music);
    }
}
