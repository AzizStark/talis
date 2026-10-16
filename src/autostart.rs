//! Per-user XDG desktop autostart, enabled only by an explicit settings action.
use crate::config::{Config, PreviewMode};
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const ENTRY_NAME: &str = "io.github.azizstark.Talis.desktop";
const OWNER: &str = "X-Talis-Managed=true";
const CONFIG_HEADER: &str = "# Managed by Talis for desktop startup\n";
#[cfg(any(feature = "desktop", test))]
const PROMPT_SEEN: &str = "Talis startup prompt answered v1\n";
static SERIAL: AtomicU64 = AtomicU64::new(0);

pub struct Launch {
    pub executable: PathBuf,
    pub directory: PathBuf,
    pub config: Config,
    pub monitor: u32,
    pub following: bool,
    pub player: Option<String>,
    pub graphics_environment: Vec<(String, String)>,
    /// Explicit native Wayland preload; never inherit arbitrary preload entries.
    pub wayland_library: Option<PathBuf>,
}
impl Launch {
    fn configuration(&self) -> Result<String> {
        ensure!(
            self.executable.is_absolute() && self.executable.is_file(),
            "Talis executable is unavailable"
        );
        ensure!(
            self.directory.is_absolute() && self.directory.is_dir(),
            "Talis working directory is unavailable"
        );
        let mut config = self.config.clone();
        if let Some(library) = &self.wayland_library {
            ensure!(
                library.is_absolute() && library.is_file(),
                "Wayland layer-shell library is unavailable"
            );
            let text = path_text(library)?;
            let name = library
                .file_name()
                .context("Missing layer-shell library name")?;
            ensure!(
                !text.contains(':') && !name.as_encoded_bytes().iter().any(u8::is_ascii_whitespace),
                "Wayland library directory cannot contain colons, or its filename whitespace"
            );
            ensure!(
                self.graphics_environment.is_empty(),
                "Wayland startup cannot capture X11 graphics overrides"
            );
        }
        for path in [
            &mut config.scene.background,
            &mut config.scene.arm_poses,
            &mut config.scene.arm_flow,
        ] {
            if path.is_relative() {
                *path = self.directory.join(&*path);
            }
        }
        if let Some(path) = &mut config.scene.sleeve {
            if path.is_relative() {
                *path = self.directory.join(&*path);
            }
        }
        if let Some(path) = &mut config.scene.record_lighting {
            if path.is_relative() {
                *path = self.directory.join(&*path);
            }
        }
        if let Some(night) = &mut config.scene.night {
            night.make_absolute(&self.directory);
        }
        if let Some(path) = &mut config.scene.vinyl_detail {
            if path.is_relative() {
                *path = self.directory.join(&*path);
            }
        }
        if let Some(path) = &mut config.scene.buttons {
            if path.is_relative() {
                *path = self.directory.join(&*path);
            }
        }
        config.validate()?;
        Ok(format!("{CONFIG_HEADER}{}", toml::to_string(&config)?))
    }
    pub fn arguments(&self, snapshot: &Path) -> Result<Vec<String>> {
        let mut args = vec![
            "--startup-config".into(),
            path_text(snapshot)?,
            "--desktop".into(),
            "--monitor".into(),
            self.monitor.to_string(),
            "--start-minimized".into(),
        ];
        if self.wayland_library.is_some() {
            args.push("--experimental-wayland".into());
        }
        if self.config.scene.mode == PreviewMode::Arm {
            args.push("--arm".into());
            if self.following {
                args.push("--now-playing".into());
                if let Some(player) = &self.player {
                    args.extend(["--startup-player".into(), player.clone()]);
                }
            }
        }
        Ok(args)
    }
    fn entry(&self, snapshot: &Path) -> Result<String> {
        let mut command = Vec::new();
        if let Some(library) = &self.wayland_library {
            command.extend([
                "/usr/bin/env".into(),
                "-u".into(),
                "GDK_DEBUG".into(),
                "-u".into(),
                "GDK_DISABLE".into(),
            ]);
            command.push("GDK_BACKEND=wayland".into());
            command.push(format!(
                "LD_PRELOAD={}",
                path_text(Path::new(
                    library
                        .file_name()
                        .context("Missing layer-shell library name")?
                ))?
            ));
            command.push(format!(
                "LD_LIBRARY_PATH={}",
                path_text(
                    library
                        .parent()
                        .context("Missing layer-shell library directory")?
                )?
            ));
        } else if !self.graphics_environment.is_empty() {
            command.push("/usr/bin/env".into());
            for (name, value) in &self.graphics_environment {
                ensure!(
                    matches!(name.as_str(), "GDK_DEBUG" | "GDK_DISABLE"),
                    "Unsupported startup environment variable"
                );
                command.push(format!("{name}={value}"));
            }
        }
        command.push(path_text(&self.executable)?);
        command.extend(self.arguments(snapshot)?);
        let exec = command
            .iter()
            .map(|arg| quote_exec(arg))
            .collect::<Result<Vec<_>>>()?
            .join(" ");
        let icon = self
            .directory
            .join("assets/ui/branding/io.github.azizstark.Talis.svg");
        let icon = if icon.is_file() {
            path_text(&icon)?
        } else {
            "io.github.azizstark.Talis".into()
        };
        let session = if self.wayland_library.is_some() {
            "OnlyShowIn=KDE;\nX-Talis-Session=Wayland\n"
        } else {
            ""
        };
        Ok(format!(
            "[Desktop Entry]\nType=Application\nName=Talis\nComment=An interactive music-aware live turntable on desktop\nIcon={}\nStartupWMClass=io.github.azizstark.Talis\nExec={exec}\nTryExec={}\nPath={}\nTerminal=false\nStartupNotify=false\n{OWNER}\n",
            escape_value(&icon)?,
            escape_value(&path_text(&self.executable)?)?,
            escape_value(&path_text(&self.directory)?)?
        ) + session)
    }
}

#[derive(Clone)]
pub struct Manager {
    entry: PathBuf,
    snapshot: PathBuf,
    #[cfg(any(feature = "desktop", test))]
    prompt_seen: PathBuf,
}
impl Manager {
    pub fn for_user() -> Result<Self> {
        Self::new(crate::preferences::config_directory()?)
    }
    pub fn new(root: PathBuf) -> Result<Self> {
        ensure!(
            root.is_absolute(),
            "Autostart config directory must be absolute"
        );
        Ok(Self {
            entry: root.join("autostart").join(ENTRY_NAME),
            snapshot: root.join("talis/startup.toml"),
            #[cfg(any(feature = "desktop", test))]
            prompt_seen: root.join("talis/startup-prompt-seen"),
        })
    }
    pub fn enabled(&self) -> Result<bool> {
        let Some(entry) = read_regular(&self.entry)? else {
            return Ok(false);
        };
        ensure!(
            entry.lines().any(|line| line.trim() == OWNER),
            "An unmanaged Talis startup entry already exists; it will not be changed"
        );
        Ok(!entry.lines().any(|line| {
            matches!(
                line.trim(),
                "Hidden=true" | "X-GNOME-Autostart-enabled=false"
            )
        }))
    }
    pub fn owns_snapshot(&self, path: &Path) -> Result<bool> {
        if path != self.snapshot {
            return Ok(false);
        }
        self.enabled()?; // Includes entry ownership and regular-file checks.
        Ok(read_regular(path)?.is_some_and(|text| text.starts_with(CONFIG_HEADER)))
    }
    /// Only update the entry if it targets this exact, now-updated executable.
    /// Preserve loader/session/path/monitor arguments and an owned legacy backup.
    pub fn migrate_preferences(&self, executable: &Path) -> Result<bool> {
        self.enabled()?;
        let Some(entry) = read_regular(&self.entry)? else {
            return Ok(false);
        };
        let old = format!(
            "{} {}",
            quote_exec("--config")?,
            quote_exec(&path_text(&self.snapshot)?)?
        );
        let expected_executable = format!("TryExec={}", escape_value(&path_text(executable)?)?);
        if !entry.lines().any(|line| line == expected_executable)
            || !entry
                .lines()
                .any(|line| line.starts_with("Exec=") && line.contains(&old))
        {
            return Ok(false);
        }
        ensure!(
            self.owns_snapshot(&self.snapshot)?,
            "Startup snapshot is not managed by Talis"
        );
        let legacy_config = Config::load(&self.snapshot)?;
        let player_marker = format!("{} \"", quote_exec("--player")?);
        let player = entry
            .lines()
            .find(|line| line.starts_with("Exec="))
            .and_then(|line| line.split_once(&player_marker))
            .and_then(|(_, value)| value.split('"').next())
            .map(crate::preferences::normalize_player)
            .transpose()?;
        let new = format!(
            "{} {}",
            quote_exec("--startup-config")?,
            quote_exec(&path_text(&self.snapshot)?)?
        );
        let mut lines = Vec::new();
        for line in entry.lines() {
            if line.starts_with("Exec=") {
                lines.push(
                    line.replacen(&old, &new, 1)
                        .replace(&quote_exec("--player")?, &quote_exec("--startup-player")?),
                );
            } else {
                lines.push(line.to_owned());
            }
        }
        let backup = self.entry.with_extension("desktop.pre-preferences");
        if let Some(previous) = read_regular(&backup)? {
            ensure!(
                previous.lines().any(|line| line.trim() == OWNER),
                "Unmanaged startup migration backup; preserved"
            );
        } else {
            atomic_write(&backup, entry.as_bytes())?;
        }
        let store = crate::preferences::Store::new(self.snapshot.with_file_name("settings.toml"))?;
        store.import_if_missing(&crate::preferences::Document::from_config(
            &legacy_config,
            player.as_deref(),
        ))?;
        atomic_write(&self.entry, format!("{}\n", lines.join("\n")).as_bytes())?;
        Ok(true)
    }
    /// Reading first-run state never enables startup or creates directories.
    #[cfg(any(feature = "desktop", test))]
    pub(crate) fn prompt_pending(&self) -> Result<bool> {
        Ok(!self.enabled()? && !self.prompt_answered()?)
    }
    #[cfg(any(feature = "desktop", test))]
    fn prompt_answered(&self) -> Result<bool> {
        let Some(contents) = read_regular(&self.prompt_seen)? else {
            return Ok(false);
        };
        ensure!(
            contents == PROMPT_SEEN,
            "Unrecognized startup prompt marker"
        );
        Ok(true)
    }
    #[cfg(any(feature = "desktop", test))]
    pub(crate) fn dismiss_prompt(&self) -> Result<()> {
        if !self.prompt_answered()? {
            atomic_write(&self.prompt_seen, PROMPT_SEEN.as_bytes())?;
        }
        Ok(())
    }
    pub fn enable(&self, launch: &Launch) -> Result<()> {
        self.enabled()?; // Check ownership before changing any files.
        if let Some(config) = read_regular(&self.snapshot)? {
            ensure!(
                config.starts_with(CONFIG_HEADER),
                "An unmanaged startup config already exists; it will not be changed"
            );
        }
        let config = launch.configuration()?;
        let entry = launch.entry(&self.snapshot)?;
        atomic_write(&self.snapshot, config.as_bytes())?;
        atomic_write(&self.entry, entry.as_bytes())
    }
    pub fn disable(&self) -> Result<()> {
        self.enabled()?; // Includes ownership checks, even for a Hidden entry.
        match fs::remove_file(&self.entry) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).context("Cannot remove Talis startup entry"),
        }
    }
}
fn read_regular(path: &Path) -> Result<Option<String>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("Cannot inspect {}", path.display()));
        }
    };
    ensure!(
        metadata.is_file() && metadata.len() <= 256 * 1024,
        "Startup path is not a regular, bounded file: {}",
        path.display()
    );
    Ok(Some(fs::read_to_string(path).with_context(|| {
        format!("Cannot read {}", path.display())
    })?))
}
fn path_text(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_owned)
        .context("Startup paths must be valid UTF-8")
}
fn escape_value(value: &str) -> Result<String> {
    ensure!(
        !value.contains(['\n', '\r', '\0']),
        "Startup values must not contain line breaks or NUL"
    );
    Ok(value.replace('\\', "\\\\"))
}
fn quote_exec(value: &str) -> Result<String> {
    let mut quoted = String::from("\"");
    for ch in value.chars() {
        if matches!(ch, '\\' | '"' | '`' | '$') {
            quoted.push('\\');
        }
        if ch == '%' {
            quoted.push('%');
        }
        quoted.push(ch);
    }
    quoted.push('"');
    escape_value(&quoted)
}
fn atomic_write(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path.parent().context("Missing startup parent directory")?;
    fs::create_dir_all(parent).with_context(|| format!("Cannot create {}", parent.display()))?;
    let temp = parent.join(format!(
        ".talis-startup-{}-{}.tmp",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temp)
        .with_context(|| format!("Cannot create {}", temp.display()))?;
    let result: Result<()> = (|| {
        file.write_all(contents)?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.with_context(|| format!("Cannot save {}", path.display()))
}

#[cfg(test)]
pub(crate) use tests::fixtures::Fixture;

#[cfg(test)]
mod tests;
