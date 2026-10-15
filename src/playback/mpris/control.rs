//! User-triggered writes only. One bounded request, exact owner, no retries/queue.
use super::*;
use crate::playback::transport::{Capabilities, Command, Target};

pub(super) struct Pending {
    id: u64,
    target: Target,
    method: &'static str,
    revision: u64,
    pub(super) task: Option<glib::JoinHandle<()>>,
}
pub(super) fn sample(properties: &Properties, player: &str, owner: &str) -> Option<Target> {
    let (status, observed_at) = artwork_playback(properties)?;
    let flag = |key: &str| {
        properties
            .get(key)
            .and_then(|v| v.get::<bool>())
            .unwrap_or(false)
    };
    Some(Target {
        player: player.into(),
        owner: owner.into(),
        status,
        observed_at,
        capabilities: Capabilities {
            control: flag("CanControl"),
            play: flag("CanPlay"),
            pause: flag("CanPause"),
            previous: flag("CanGoPrevious"),
            next: flag("CanGoNext"),
        },
    })
}
fn current(i: &Inner, p: &Pending) -> bool {
    let feed = i.feed.borrow();
    !i.stopped
        && feed.enabled
        && feed.selection_revision == p.revision
        && feed.active.as_deref() == Some(p.target.player.as_str())
        && i.entries.get(&p.target.player).is_some_and(|e| {
            e.owner.as_deref() == Some(p.target.owner.as_str())
                && e.transport.as_ref().is_some_and(|t| {
                    t.same_player(&p.target) && t.supports_method(p.method, Instant::now())
                })
        })
}
pub(super) fn cancel(i: &mut Inner) {
    if let Some(mut pending) = i.control.take() {
        if let Some(task) = pending.task.take() {
            task.abort();
        }
    }
    let mut feed = i.feed.borrow_mut();
    feed.control_pending = false;
    feed.control_notice = None;
    feed.control_notice_target = None;
}
impl Monitor {
    /// Request from the current selection; never launches a player.
    pub fn command(&self, command: Command) -> Result<(), &'static str> {
        let (target, revision) = {
            let i = self.0.borrow();
            let feed = i.feed.borrow();
            (
                feed.command_target(command, Instant::now())
                    .cloned()
                    .ok_or("Control unavailable or busy")?,
                feed.selection_revision,
            )
        };
        self.command_for(command, &target, revision)
    }
    /// A gesture retains its selected player/owner until release. A changed
    /// selection, owner or play/pause state cancels it instead of retargeting it.
    pub fn command_for(
        &self,
        command: Command,
        target: &Target,
        revision: u64,
    ) -> Result<(), &'static str> {
        let mut i = self.0.borrow_mut();
        if i.stopped || i.control.is_some() {
            return Err("Control unavailable or busy");
        }
        {
            let feed = i.feed.borrow();
            let fresh = feed
                .command_target(command, Instant::now())
                .ok_or("Control unavailable or stale")?;
            if revision != feed.selection_revision
                || !fresh.same_player(target)
                || (command == Command::PlayPause && fresh.status != target.status)
            {
                return Err("Player changed during click");
            }
        }
        let connection = i
            .connection
            .clone()
            .filter(|c| !c.is_closed())
            .ok_or("Session bus unavailable")?;
        i.request_id = i.request_id.wrapping_add(1);
        let id = i.request_id;
        let method = command.method(target.status);
        let pending = Pending {
            id,
            target: target.clone(),
            method,
            revision,
            task: None,
        };
        if !current(&i, &pending) {
            return Err("Player unavailable or stale");
        }
        i.control = Some(pending);
        {
            let mut feed = i.feed.borrow_mut();
            feed.control_pending = true;
            feed.control_notice = Some(format!("Sending {method}…"));
            feed.control_notice_target = Some((target.player.clone(), target.owner.clone()));
        }
        let weak = Rc::downgrade(&self.0);
        let target = target.clone();
        i.control.as_mut().unwrap().task = Some(glib::spawn_future_local(async move {
            // Recheck well-known-name ownership before addressing the captured
            // unique owner. Even a last-moment replacement can never receive it.
            let owner = connection
                .call_future(
                    Some(BUS),
                    BUS_PATH,
                    BUS,
                    "GetNameOwner",
                    Some(&(target.player.as_str(),).to_variant()),
                    None,
                    gio::DBusCallFlags::NO_AUTO_START,
                    TIMEOUT_MS,
                )
                .await
                .ok()
                .and_then(|v| v.get::<(String,)>().map(|(s,)| s));
            let Some(shared) = weak.upgrade() else {
                return;
            };
            let valid = {
                let i = shared.borrow();
                i.control
                    .as_ref()
                    .is_some_and(|p| p.id == id && current(&i, p))
            };
            if owner.as_deref() != Some(target.owner.as_str()) || !valid {
                Self::finish_control(
                    &shared,
                    id,
                    Err("Player or selection changed; command cancelled".into()),
                );
                return;
            }
            let result = connection
                .call_future(
                    Some(&target.owner),
                    PATH,
                    PLAYER,
                    method,
                    None,
                    None,
                    gio::DBusCallFlags::NO_AUTO_START,
                    TIMEOUT_MS,
                )
                .await;
            let result = match result {
                Ok(v) if v.get::<()>().is_some() => Ok(()),
                Ok(_) => Err("Invalid player reply; command was not retried".into()),
                Err(e) => Err(format!("{method} failed or timed out (not retried): {e}")),
            };
            Self::finish_control(&shared, id, result);
        }));
        drop(i);
        Self::publish(&self.0);
        Ok(())
    }
    fn finish_control(shared: &Rc<RefCell<Inner>>, id: u64, result: Result<(), String>) {
        {
            let mut i = shared.borrow_mut();
            if !i.control.as_ref().is_some_and(|p| p.id == id) {
                return;
            }
            let pending = i.control.take().unwrap();
            let feed_current = {
                let feed = i.feed.borrow();
                !i.stopped
                    && feed.enabled
                    && feed.selection_revision == pending.revision
                    && feed.active.as_deref() == Some(pending.target.player.as_str())
                    && i.entries
                        .get(&pending.target.player)
                        .is_some_and(|e| e.owner.as_deref() == Some(pending.target.owner.as_str()))
            };
            i.feed.borrow_mut().control_pending = false;
            if feed_current {
                i.feed.borrow_mut().control_notice = Some(match result {
                    Ok(()) => format!("{} requested", pending.method),
                    Err(note) => note.chars().take(400).collect(),
                });
                // Never optimistically flip status/track/arm. Discard reads started
                // before the command and obtain the real player state immediately.
                if let Some(entry) = i.entries.get_mut(&pending.target.player) {
                    entry.invalidate("Control requested; synchronizing…");
                }
            } else {
                let mut feed = i.feed.borrow_mut();
                feed.control_notice = None;
                feed.control_notice_target = None;
            }
        }
        Self::tick(shared);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_capabilities_are_independent_of_timing_and_missing_flags_disable() {
        let mut p = Properties::new();
        p.insert("PlaybackStatus".into(), "Paused".to_variant());
        p.insert("CanControl".into(), true.to_variant());
        p.insert("CanPlay".into(), true.to_variant());
        p.insert("CanGoNext".into(), "true".to_variant());
        let t = sample(&p, "org.mpris.MediaPlayer2.fixture", ":1.42").unwrap();
        assert!(t.supports(Command::PlayPause, Instant::now()));
        assert!(!t.supports(Command::Next, Instant::now()));
        assert!(!t.supports(Command::Previous, Instant::now()));
        p.insert("PlaybackStatus".into(), "invalid".to_variant());
        assert!(sample(&p, "org.mpris.MediaPlayer2.fixture", ":1.42").is_none());
    }
}
