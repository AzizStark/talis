//! Bounded artwork loading off the GTK/render thread. No persistent cache or effects.
mod history;
mod source;
use crate::playback::PlaybackFeed;
use anyhow::{Context, Result, bail, ensure};
use gtk::glib;
use std::{
    cell::RefCell,
    fs::File,
    io::{Cursor, Read},
    path::PathBuf,
    process::{Command, Stdio},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
    time::Duration,
};

const MAX_BYTES: u64 = source::MAX_BYTES as u64;
const MAX_DIMENSION: u32 = 4096;
const TEXTURE_SIZE: u32 = 1024;
type Loaded = Result<Arc<image::RgbaImage>>;

#[derive(Clone, Debug)]
struct Key {
    player: Option<String>,
    owner: Option<String>,
    track: String,
    url: Arc<str>,
}
impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        self.player == other.player
            && self.owner == other.owner
            && self.track == other.track
            && (Arc::ptr_eq(&self.url, &other.url) || self.url == other.url)
    }
}
impl Eq for Key {}

/// Cheap, shared bound used before copying an MPRIS artwork value.
pub(crate) fn source_length_allowed(url: &str) -> bool {
    source::length_allowed(url)
}
struct Task {
    key: Key,
    rx: Receiver<Loaded>,
    cancel: Arc<AtomicBool>,
}
struct Inner {
    desired: Option<Key>,
    task: Option<Task>,
    source: Option<glib::SourceId>,
    image: Option<Arc<image::RgbaImage>>,
    revision: u64,
    cache: Option<(Key, Arc<image::RgbaImage>)>,
    notice: String,
    notify: Rc<dyn Fn()>,
    stopped: bool,
}
impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(source) = self.source.take() {
            source.remove();
        }
    }
}
#[derive(Clone)]
struct ImageLoader(Rc<RefCell<Inner>>);
impl ImageLoader {
    pub fn new(notify: impl Fn() + 'static) -> Self {
        Self(Rc::new(RefCell::new(Inner {
            desired: None,
            task: None,
            source: None,
            image: None,
            revision: 0,
            cache: None,
            notice: "No cover art".into(),
            notify: Rc::new(notify),
            stopped: false,
        })))
    }
    /// A player/track change clears the old cover immediately. Late results never win.
    fn sync_key(&self, desired: Option<Key>, seed: Option<Arc<image::RgbaImage>>) {
        let notify = {
            let mut i = self.0.borrow_mut();
            if i.stopped || i.desired == desired {
                return;
            }
            if let Some(task) = &i.task {
                task.cancel.store(true, Ordering::Relaxed);
            }
            i.desired = desired;
            i.image = None;
            i.revision = i.revision.wrapping_add(1);
            i.notice = "No cover art · original vinyl visible".into();
            if let Some(key) = &i.desired {
                if let Some((cached_key, image)) = &i.cache {
                    if cached_key == key {
                        i.image = Some(image.clone());
                    }
                }
                if i.image.is_none() {
                    i.image = seed;
                }
                i.notice = if i.image.is_some() {
                    "Cover art ready"
                } else {
                    "Loading cover art…"
                }
                .into();
            }
            i.notify.clone()
        };
        notify();
        self.start();
    }
    fn start(&self) {
        let mut i = self.0.borrow_mut();
        if i.stopped || i.task.is_some() || i.image.is_some() {
            return;
        }
        let Some(key) = i.desired.clone() else {
            return;
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let url = key.url.clone();
        let (tx, rx) = mpsc::channel();
        match thread::Builder::new()
            .name("talis-artwork".into())
            .spawn(move || {
                let _ = tx.send(load(&url, &flag));
            }) {
            Ok(_) => i.task = Some(Task { key, rx, cancel }),
            Err(_) => {
                i.notice = "Artwork worker unavailable".into();
                return;
            }
        }
        if i.source.is_none() {
            let weak = Rc::downgrade(&self.0);
            i.source = Some(glib::timeout_add_local(
                Duration::from_millis(50),
                move || {
                    let Some(inner) = weak.upgrade() else {
                        return glib::ControlFlow::Break;
                    };
                    let loader = Self(inner);
                    loader.poll();
                    let mut i = loader.0.borrow_mut();
                    if i.task.is_none() {
                        i.source = None;
                        glib::ControlFlow::Break
                    } else {
                        glib::ControlFlow::Continue
                    }
                },
            ));
        }
    }
    fn poll(&self) {
        let notify = {
            let mut i = self.0.borrow_mut();
            let Some(task) = &i.task else {
                return;
            };
            let result = match task.rx.try_recv() {
                Ok(result) => result,
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => {
                    Err(anyhow::anyhow!("Artwork worker stopped"))
                }
            };
            let task = i.task.take().unwrap();
            if i.desired.as_ref() == Some(&task.key) && !task.cancel.load(Ordering::Relaxed) {
                match result {
                    Ok(image) => {
                        i.cache = Some((task.key, image.clone()));
                        i.image = Some(image);
                        i.revision = i.revision.wrapping_add(1);
                        i.notice = "Cover art ready".into();
                    }
                    Err(error) => {
                        // Do not log the URL: it can contain signed/authenticated query parameters.
                        tracing::warn!("Cover art unavailable: {error:#}");
                        i.notice = "Cover art unavailable · original vinyl visible".into();
                    }
                }
                Some(i.notify.clone())
            } else {
                None
            }
        };
        if let Some(notify) = notify {
            notify();
        }
        // Only superseded work is retried automatically, never a failed current URL.
        let superseded = {
            let i = self.0.borrow();
            i.desired.is_some() && i.image.is_none() && i.notice == "Loading cover art…"
        };
        if superseded {
            self.start();
        }
    }
    pub fn snapshot(&self) -> (u64, Option<Arc<image::RgbaImage>>, String) {
        let i = self.0.borrow();
        (i.revision, i.image.clone(), i.notice.clone())
    }
    pub fn stop(&self) {
        let mut i = self.0.borrow_mut();
        i.stopped = true;
        if let Some(task) = &i.task {
            task.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(source) = i.source.take() {
            source.remove();
        }
    }
}

/// Two bounded image channels: current art and the immediately preceding track.
/// Each channel allows at most one worker; cancelled completions never publish.
#[derive(Clone)]
pub struct Loader {
    current: ImageLoader,
    previous: ImageLoader,
    history: Rc<RefCell<history::History>>,
}
impl Loader {
    pub fn new(notify: impl Fn() + 'static) -> Self {
        let notify: Rc<dyn Fn()> = Rc::new(notify);
        let current_notify = notify.clone();
        Self {
            current: ImageLoader::new(move || current_notify()),
            previous: ImageLoader::new(move || notify()),
            history: Rc::new(RefCell::new(history::History::default())),
        }
    }
    pub fn sync(&self, feed: &PlaybackFeed) {
        let current = feed.artwork.as_ref().filter(|_| feed.enabled).map(|a| Key {
            player: feed.active.clone(),
            owner: feed.artwork_owner.clone(),
            track: a.track.clone(),
            url: a.url.clone(),
        });
        let previous = self.history.borrow_mut().observe(feed, current.as_ref());
        let seed = previous.as_ref().and_then(|desired| {
            // A same-track URL refresh is not a new song. The last successfully
            // decoded image is still preferable to refetching or showing another track.
            self.current
                .0
                .borrow()
                .cache
                .as_ref()
                .filter(|(key, _)| {
                    key.player == desired.player
                        && key.owner == desired.owner
                        && key.track == desired.track
                })
                .map(|(_, image)| image.clone())
        });
        self.previous.sync_key(previous, seed);
        self.current.sync_key(current, None);
    }
    pub fn snapshot(&self) -> (u64, Option<Arc<image::RgbaImage>>, String) {
        self.current.snapshot()
    }
    pub fn previous_snapshot(&self) -> (u64, Option<Arc<image::RgbaImage>>) {
        let (revision, image, _) = self.previous.snapshot();
        (revision, image)
    }
    pub fn stop(&self) {
        self.current.stop();
        self.previous.stop();
    }
}

enum Location<'a> {
    Local(PathBuf),
    Remote(&'a str),
    Embedded(source::Embedded<'a>),
}
fn location(url: &str) -> Result<Location<'_>> {
    ensure!(source_length_allowed(url), "Invalid artwork URL");
    // Parse inline images before GLib's general URI parser: this is binary
    // content, not a network address, and must never reach curl or the filesystem.
    if source::is_data(url) {
        return Ok(Location::Embedded(source::Embedded::parse(url)?));
    }
    ensure!(!url.chars().any(char::is_control), "Invalid artwork URL");
    let uri = glib::Uri::parse(url, glib::UriFlags::NONE).context("Invalid artwork URI")?;
    match uri.scheme().as_str() {
        "file" => {
            let (path, host) = glib::filename_from_uri(url).context("Invalid local artwork URI")?;
            ensure!(
                host.as_ref()
                    .is_none_or(|h| h.is_empty() || h == "localhost"),
                "Remote file hosts are not allowed"
            );
            Ok(Location::Local(path))
        }
        "http" | "https" => {
            ensure!(
                uri.host().is_some_and(|h| !h.is_empty()) && uri.userinfo().is_none(),
                "Artwork HTTP URI requires a host and no credentials"
            );
            Ok(Location::Remote(url))
        }
        _ => bail!("Only file, HTTP, HTTPS and embedded PNG/JPEG artwork are supported"),
    }
}
fn bounded_read(reader: impl Read) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .context("Cannot read artwork")?;
    ensure!(
        !bytes.is_empty() && bytes.len() as u64 <= MAX_BYTES,
        "Artwork is empty or exceeds 16 MiB"
    );
    Ok(bytes)
}
fn load(url: &str, cancel: &AtomicBool) -> Loaded {
    ensure!(
        !cancel.load(Ordering::Relaxed),
        "Artwork request superseded"
    );
    let bytes = match location(url)? {
        Location::Local(path) => {
            let metadata = std::fs::metadata(&path).context("Cannot inspect local artwork")?;
            ensure!(
                metadata.is_file() && metadata.len() <= MAX_BYTES,
                "Artwork must be a regular file of at most 16 MiB"
            );
            let file = File::open(path).context("Cannot open local artwork")?;
            let metadata = file.metadata()?;
            ensure!(
                metadata.is_file() && metadata.len() <= MAX_BYTES,
                "Artwork must be a regular file of at most 16 MiB"
            );
            bounded_read(file)?
        }
        Location::Remote(url) => fetch(url, cancel)?,
        Location::Embedded(embedded) => embedded.decode()?,
    };
    ensure!(
        !cancel.load(Ordering::Relaxed),
        "Artwork request superseded"
    );
    Ok(Arc::new(decode(&bytes)?))
}
fn fetch(url: &str, cancel: &AtomicBool) -> Result<Vec<u8>> {
    // No shell expansion; ignore user curlrc, verify TLS, restrict all redirect protocols.
    let mut child = Command::new("curl")
        .args([
            "--disable",
            "--silent",
            "--fail",
            "--location",
            "--proto",
            "=http,https",
            "--proto-redir",
            "=http,https",
            "--max-redirs",
            "5",
            "--connect-timeout",
            "4",
            "--max-time",
            "10",
            "--max-filesize",
            "16777216",
            "--url",
            url,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("HTTP artwork requires curl on PATH")?;
    let stdout = child.stdout.take().unwrap();
    let failed = Arc::new(AtomicBool::new(false));
    let read_failed = failed.clone();
    let reader = match thread::Builder::new()
        .name("talis-artwork-read".into())
        .spawn(move || {
            let result = bounded_read(stdout);
            if result.is_err() {
                read_failed.store(true, Ordering::Relaxed);
            }
            result
        }) {
        Ok(reader) => reader,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.into());
        }
    };
    let status = loop {
        if cancel.load(Ordering::Relaxed) || failed.load(Ordering::Relaxed) {
            // A finished reader may mean the size limit was hit; never leave curl blocked.
            if let Some(status) = child.try_wait()? {
                break status;
            }
            let _ = child.kill();
            break child.wait()?;
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        thread::sleep(Duration::from_millis(20));
    };
    let bytes = reader
        .join()
        .map_err(|_| anyhow::anyhow!("Artwork reader stopped"))??;
    ensure!(
        !cancel.load(Ordering::Relaxed),
        "Artwork request superseded"
    );
    ensure!(
        status.success(),
        "Artwork HTTP request failed (timeout, size limit or server error)"
    );
    Ok(bytes)
}
fn decode(bytes: &[u8]) -> Result<image::RgbaImage> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let image = reader
        .decode()
        .context("Unsupported or damaged artwork (PNG/JPEG supported)")?;
    let side = image.width().min(image.height());
    ensure!(side > 0, "Empty artwork dimensions");
    let square = image.crop_imm(
        (image.width() - side) / 2,
        (image.height() - side) / 2,
        side,
        side,
    );
    Ok(if side > TEXTURE_SIZE {
        square.resize_exact(
            TEXTURE_SIZE,
            TEXTURE_SIZE,
            image::imageops::FilterType::Triangle,
        )
    } else {
        square
    }
    .into_rgba8())
}

#[cfg(test)]
mod tests;
