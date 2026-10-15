use super::{Document, Patch, config_directory};
use anyhow::{Context, Result, ensure};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::{fs::OpenOptionsExt, io::AsRawFd},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
const LIMIT: u64 = 64 * 1024;
static SERIAL: AtomicU64 = AtomicU64::new(0);

/// Only an actual contended flock is retryable, not unrelated I/O failures.
#[derive(Debug)]
pub(super) struct LockBusy;
impl std::fmt::Display for LockBusy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Preferences lock unavailable: another writer is saving settings")
    }
}
impl std::error::Error for LockBusy {}

#[derive(Clone, Debug)]
pub struct Store {
    path: PathBuf,
}
impl Store {
    pub fn for_user() -> Result<Self> {
        Self::new(config_directory()?.join("talis/settings.toml"))
    }
    pub fn new(path: PathBuf) -> Result<Self> {
        ensure!(
            path.is_absolute() && path.parent().is_some(),
            "Preferences path must be absolute"
        );
        Ok(Self { path })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn read(&self) -> Result<Document> {
        match read_regular(&self.path)? {
            None => Ok(Document::empty()),
            Some(text) => parse(&text),
        }
    }
    pub fn write(&self, patch: &Patch) -> Result<()> {
        self.write_inner(patch, false, Duration::from_millis(750))
    }
    /// The background worker owns its retry deadline; never stack lock waits.
    pub(super) fn try_write(&self, patch: &Patch) -> Result<()> {
        self.write_inner(patch, false, Duration::ZERO)
    }
    /// Check absence under the writer lock so concurrent preferences always win.
    pub fn import_if_missing(&self, document: &Document) -> Result<()> {
        document.validate()?;
        let mut patch = document.values();
        patch.remove("schema_version");
        self.write_inner(&patch, true, Duration::from_millis(750))
    }
    fn write_inner(&self, patch: &Patch, only_if_missing: bool, lock_wait: Duration) -> Result<()> {
        if patch.is_empty() {
            return Ok(());
        }
        ensure!(
            !patch.contains_key("schema_version"),
            "Schema version is not an editable preference"
        );
        let parent = self.path.parent().unwrap();
        if let Ok(metadata) = fs::symlink_metadata(parent) {
            ensure!(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "Preferences directory is unsafe"
            );
        }
        fs::create_dir_all(parent).context("Create preferences directory")?;
        let _lock = Lock::acquire(&self.path.with_extension("lock"), lock_wait)?;
        let previous = read_regular(&self.path)?;
        if only_if_missing && previous.is_some() {
            return Ok(());
        }
        let document = previous
            .as_deref()
            .map(parse)
            .transpose()?
            .unwrap_or_else(Document::empty);
        let mut values = document.values();
        values.extend(patch.clone());
        let next: Document = toml::Value::Table(values)
            .try_into()
            .context("Invalid preference fields")?;
        next.validate()?;
        if let Some(previous) = previous {
            let backup = self.path.with_extension("toml.bak");
            if let Some(existing) = read_regular(&backup)? {
                parse(&existing).context("Invalid preferences backup; preserved")?;
            }
            atomic_write(&backup, previous.as_bytes())?;
        }
        atomic_write(&self.path, toml::to_string_pretty(&next)?.as_bytes())
    }
}
fn parse(text: &str) -> Result<Document> {
    let document: Document = toml::from_str(text).context("Invalid preferences TOML")?;
    document.validate()?;
    Ok(document)
}
pub(crate) fn read_regular(path: &Path) -> Result<Option<String>> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("Cannot read {}", path.display())),
    };
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= LIMIT,
        "Preferences must be a regular file no larger than 64 KiB"
    );
    let mut text = String::new();
    file.take(LIMIT + 1).read_to_string(&mut text)?;
    ensure!(
        text.len() as u64 <= LIMIT,
        "Preferences grew beyond the size limit"
    );
    Ok(Some(text))
}
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    read_regular(path)?; // Refuse symbolic links and non-regular reserved targets.
    let parent = path.parent().unwrap();
    let temporary = parent.join(format!(
        ".talis-settings-{}-{}.tmp",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    let mut created = false;
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temporary)?;
        created = true;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() && created {
        let _ = fs::remove_file(&temporary);
    }
    result.with_context(|| format!("Cannot save {}", path.display()))
}
struct Lock(File);
impl Lock {
    fn acquire(path: &Path, wait: Duration) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)?;
        ensure!(
            file.metadata()?.is_file(),
            "Preferences lock is not a regular file"
        );
        let until = Instant::now() + wait;
        loop {
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                return Ok(Self(file));
            }
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::EWOULDBLOCK) {
                return Err(error).context("Preferences lock unavailable");
            }
            let remaining = until.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(LockBusy.into());
            }
            std::thread::sleep(Duration::from_millis(10).min(remaining));
        }
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
