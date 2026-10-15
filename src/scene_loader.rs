//! Coalesced scene decoding off the UI thread; stale requests never replace a newer choice.
use crate::{
    config::{Config, SceneSettings},
    preview::PreviewAsset,
};
use anyhow::{Context, Result};
use std::{
    cell::Cell,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
};

type LoadResult = (u64, SceneSettings, Result<PreviewAsset>);
struct WorkerDone(Arc<AtomicBool>);
impl Drop for WorkerDone {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
pub struct SceneLoader {
    requests: mpsc::Sender<(u64, Config)>,
    // One decoded candidate at most; an unbounded channel could retain several
    // large stale packages while the UI is busy or scene choices change.
    results: Arc<Mutex<Option<LoadResult>>>,
    worker_done: Arc<AtomicBool>,
    desired: Arc<AtomicU64>,
    alive: Arc<AtomicBool>,
    pending: Cell<bool>,
}
impl SceneLoader {
    pub fn new() -> Result<Self> {
        Self::with_loader(PreviewAsset::load)
    }

    fn with_loader(
        load: impl Fn(&Config) -> Result<PreviewAsset> + Send + 'static,
    ) -> Result<Self> {
        let (requests, input) = mpsc::channel::<(u64, Config)>();
        let results = Arc::new(Mutex::new(None));
        let output = results.clone();
        let worker_done = Arc::new(AtomicBool::new(false));
        let finished = worker_done.clone();
        let desired = Arc::new(AtomicU64::new(0));
        let alive = Arc::new(AtomicBool::new(true));
        let current = desired.clone();
        let active = alive.clone();
        thread::Builder::new()
            .name("talis-scene-loader".into())
            .spawn(move || {
                let _finished = WorkerDone(finished);
                while let Ok(mut request) = input.recv() {
                    while let Ok(newer) = input.try_recv() {
                        request = newer;
                    }
                    let (epoch, config) = request;
                    if !active.load(Ordering::Acquire) {
                        break;
                    }
                    if epoch != current.load(Ordering::Acquire) {
                        continue;
                    }
                    let result =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| load(&config)))
                            .unwrap_or_else(|_| {
                                Err(anyhow::anyhow!("Scene decoder failed unexpectedly"))
                            });
                    let mut slot = output.lock().unwrap_or_else(|error| error.into_inner());
                    if active.load(Ordering::Acquire) && epoch == current.load(Ordering::Acquire) {
                        *slot = Some((epoch, config.scene, result));
                    }
                }
            })
            .context("Cannot start scene loader")?;
        Ok(Self {
            requests,
            results,
            worker_done,
            desired,
            alive,
            pending: Cell::new(false),
        })
    }
    pub fn request(&self, config: Config) -> Result<()> {
        let epoch = self.desired.fetch_add(1, Ordering::AcqRel) + 1;
        self.results
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        self.requests
            .send((epoch, config))
            .context("Scene loader is unavailable")?;
        self.pending.set(true);
        Ok(())
    }
    pub fn cancel(&self) {
        self.desired.fetch_add(1, Ordering::AcqRel);
        self.results
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        self.pending.set(false);
    }
    pub fn pending(&self) -> bool {
        self.pending.get()
    }
    pub fn poll(&self) -> Option<Result<(SceneSettings, PreviewAsset)>> {
        let result = self
            .results
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some((epoch, variant, result)) = result {
            if epoch == self.desired.load(Ordering::Acquire) {
                self.pending.set(false);
                return Some(result.map(|asset| (variant, asset)));
            }
        }
        if self.worker_done.load(Ordering::Acquire) && self.pending.replace(false) {
            return Some(Err(anyhow::anyhow!("Scene loader stopped unexpectedly")));
        }
        None
    }
}
impl Drop for SceneLoader {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests;
