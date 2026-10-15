//! Serial background writes; no GTK access or idle polling.
use super::{Patch, Store, storage::LockBusy};
use anyhow::{Result, anyhow};
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

// One contention budget, shorter than the existing two-second flush wait.
// This bounds lock waiting, not a filesystem's write/fsync completion time.
const LOCK_WAIT: Duration = Duration::from_millis(1500);

fn retry_busy(mut write: impl FnMut() -> Result<()>, budget: Duration) -> Result<()> {
    let started = Instant::now();
    let mut delay = Duration::from_millis(25);
    loop {
        let error = match write() {
            Ok(()) => return Ok(()),
            Err(error) => error,
        };
        if !error.is::<LockBusy>() {
            return Err(error);
        }
        let remaining = budget.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err(error);
        }
        tracing::debug!(
            wait_ms = started.elapsed().as_millis() as u64,
            "Settings lock busy; retrying on the preferences worker"
        );
        std::thread::sleep(delay.min(remaining));
        delay = (delay * 2).min(Duration::from_millis(200));
    }
}
enum Request {
    Save(u64, Patch),
    Flush(mpsc::SyncSender<std::result::Result<(), String>>),
    Stop,
}
pub struct Worker {
    requests: mpsc::Sender<Request>,
    results: mpsc::Receiver<(u64, std::result::Result<(), String>)>,
}
impl Worker {
    pub fn new(store: Store) -> Result<Self> {
        let (requests, receiver) = mpsc::channel();
        let (results, result_receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("talis-preferences".into())
            .spawn(move || {
                let mut latest = Ok(());
                for request in receiver {
                    match request {
                        Request::Save(id, patch) => {
                            latest = retry_busy(|| store.try_write(&patch), LOCK_WAIT)
                                .map_err(|error| format!("{error:#}"));
                            let _ = results.send((id, latest.clone()));
                        }
                        Request::Flush(reply) => {
                            let _ = reply.send(latest.clone());
                        }
                        Request::Stop => break,
                    }
                }
            })?;
        Ok(Self {
            requests,
            results: result_receiver,
        })
    }
    pub fn save(&self, id: u64, patch: Patch) -> Result<()> {
        self.requests
            .send(Request::Save(id, patch))
            .map_err(|_| anyhow!("Preferences writer stopped"))
    }
    pub fn poll(&self) -> Option<(u64, std::result::Result<(), String>)> {
        self.results.try_recv().ok()
    }
    pub fn flush(&self) -> Result<()> {
        let (reply, receiver) = mpsc::sync_channel(1);
        self.requests
            .send(Request::Flush(reply))
            .map_err(|_| anyhow!("Preferences writer stopped"))?;
        receiver
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| anyhow!("Preferences flush timed out"))?
            .map_err(anyhow::Error::msg)
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.requests.send(Request::Stop);
    }
}

#[cfg(test)]
mod tests;
