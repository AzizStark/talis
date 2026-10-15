use anyhow::Result;
mod cli;
use clap::Parser;
use cli::Args;
#[cfg(feature = "desktop")]
use talis::graphics;
use talis::{
    config::{Config, PreviewMode, SceneVariant},
    desktop::Session,
};

fn main() -> Result<()> {
    let args = Args::parse();
    #[cfg(feature = "desktop")]
    // SAFETY: no GTK, logging setup or application threads have started yet.
    unsafe {
        graphics::configure(args.experimental_wayland)?;
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "talis=info".into()),
        )
        .init();
    let result = run(args);
    #[cfg(feature = "desktop")]
    return result.or_else(graphics::recover);
    #[cfg(not(feature = "desktop"))]
    result
}

fn run(args: Args) -> Result<()> {
    if let Some(fps) = args.fps {
        anyhow::ensure!(
            (1..=120).contains(&fps),
            "render.fps must be between 1 and 120"
        );
    }
    if args.desktop && !cfg!(feature = "desktop") {
        anyhow::bail!("Desktop feature disabled");
    }
    if args.experimental_wayland && !cfg!(feature = "wayland-desktop") {
        anyhow::bail!("Native Wayland desktop requires the wayland-desktop build feature");
    }
    let isolated = args.no_user_settings
        || args.benchmark
        || args.gpu_profile
        || args.capture.is_some()
        || args.smoke_seconds.is_some()
        || args.arm_demo
        || args.background.is_some();
    let manager = talis::autostart::Manager::for_user().ok();
    let legacy_startup = args.desktop
        && args.start_minimized
        && args.config.as_ref().is_some_and(|path| {
            manager
                .as_ref()
                .is_some_and(|manager| manager.owns_snapshot(path).unwrap_or(false))
        });
    let startup_path = args.startup_config.as_ref().or(if legacy_startup {
        args.config.as_ref()
    } else {
        None
    });
    let writable = !isolated && (args.config.is_none() || legacy_startup);
    #[cfg(feature = "desktop")]
    let independent =
        isolated || (args.config.is_some() && !legacy_startup) || args.arm_progress.is_some();
    let mut migration_issue = None;
    if writable && args.desktop && !args.check {
        if let Some(manager) = &manager {
            if let Err(error) = std::env::current_exe()
                .map_err(anyhow::Error::from)
                .and_then(|exe| manager.migrate_preferences(&exe))
            {
                migration_issue = Some(format!(
                    "Startup migration failed; existing entry preserved: {error:#}"
                ));
            }
        }
    }
    let base = if let Some(path) = startup_path {
        anyhow::ensure!(
            manager
                .as_ref()
                .is_some_and(|manager| manager.owns_snapshot(path).unwrap_or(false)),
            "Unmanaged startup configuration refused"
        );
        talis::preferences::startup_base(Config::load(path)?)
    } else {
        Config::default()
    };
    let mut loaded = talis::preferences::load_user(
        base,
        if startup_path.is_none() {
            args.config.as_deref()
        } else {
            None
        },
        !isolated,
    )?;
    let player = if legacy_startup {
        if loaded.document.preferred_player.is_some() {
            loaded.document.player()?
        } else {
            args.player.clone()
        }
    } else {
        args.player
            .clone()
            .or(if loaded.document.preferred_player.is_some() {
                loaded.document.player()?
            } else {
                args.startup_player.clone()
            })
    };
    if let Some(issue) = migration_issue {
        loaded.issue = Some(issue);
    }
    let session = talis::preferences::Session {
        store: if writable { loaded.store.take() } else { None },
        issue: loaded.issue.take(),
    };
    let mut config = loaded.config;
    config.scene.resolve_resources()?;
    if let Some(issue) = &session.issue {
        tracing::warn!("{issue}");
    }
    if let Some(scene) = args.scene {
        config.scene.variant = if scene == "night" {
            SceneVariant::Night
        } else {
            SceneVariant::Day
        };
    }
    if let Some(path) = args.background {
        match config.scene.variant {
            SceneVariant::Day => config.scene.background = path,
            SceneVariant::Night => {
                config
                    .scene
                    .night
                    .as_mut()
                    .ok_or_else(|| anyhow::anyhow!("No night scene package configured"))?
                    .background = path;
            }
        }
        config.scene.mode = PreviewMode::Beauty;
    }
    if args.arm {
        config.scene.mode = PreviewMode::Arm;
    }
    if args.arm_flow {
        config.render.arm_interpolation = true;
    }
    if let Some(fps) = args.fps {
        config.render.fps = fps;
    }
    config.validate()?;
    if args.check {
        if args.desktop {
            #[cfg(feature = "desktop")]
            {
                gtk::init()?;
                let placement = talis::desktop::DesktopHost::new_for_session(
                    args.monitor.unwrap_or(0),
                    args.experimental_wayland,
                )?;
                println!("Placement: {}", placement.description());
                println!(
                    "Backend: {:?}; capabilities: {:?}",
                    placement.kind(),
                    placement.capabilities()
                );
            }
            #[cfg(not(feature = "desktop"))]
            anyhow::bail!("Desktop placement checks require the desktop feature");
        }
        println!("Talis {}", env!("CARGO_PKG_VERSION"));
        println!(
            "Settings: valid ({} FPS, {:?} finish)",
            config.render.fps, config.render.finish
        );
        println!("Session: {:?}", Session::detect());
        println!("Scene: {}", config.scene.variant.label());
        #[cfg(feature = "assets")]
        match talis::preview::PreviewAsset::load(&config)?.scene {
            talis::preview::PreviewScene::Beauty(image) => println!(
                "Mode: Blender beauty {}×{}; complete static render, not live 3D.",
                image.width(),
                image.height()
            ),
            talis::preview::PreviewScene::Arm(assets) => println!(
                "Mode: fixed-camera arm animation, {} Blender poses, {:?} crop; {}; manual/demo or read-only MPRIS timing.",
                assets.poses.len(),
                assets.manifest.crop,
                if assets.flow.is_some() {
                    "live motion interpolation"
                } else {
                    "sharp stepped playback"
                }
            ),
        }
        #[cfg(not(feature = "assets"))]
        println!(
            "Selected mode: {:?}; asset loading not verified (assets feature disabled).",
            config.scene.mode
        );
        if args.now_playing {
            println!(
                "MPRIS: player {}; scene transport buttons {} (bus not queried by --check).",
                player.as_deref().unwrap_or("automatic"),
                if config.transport.enabled {
                    "enabled"
                } else {
                    "disabled"
                }
            );
        }
        return Ok(());
    }
    #[cfg(feature = "desktop")]
    return talis::app::run_with_preferences(
        config,
        talis::app::Options {
            independent,
            fullscreen: args.fullscreen,
            desktop: args.desktop,
            experimental_wayland: args.experimental_wayland,
            monitor: args.monitor.unwrap_or(0),
            smoke_seconds: args.smoke_seconds,
            benchmark: args.benchmark,
            gpu_profile: args.gpu_profile,
            scene_only: args.scene_only,
            shell_surface: false,
            regional_presenter: args.regional_presenter,
            native_patch: args.native_patch,
            start_minimized: args.start_minimized,
            size: args.size,
            arm_progress: args.arm_progress.unwrap_or(0.0),
            arm_demo: args.arm_demo,
            now_playing: args.now_playing,
            player,
            capture: args.capture,
        },
        Some(session),
    );
    #[cfg(not(feature = "desktop"))]
    anyhow::bail!("Desktop feature disabled. Build with --features desktop, or use --check.");
}
