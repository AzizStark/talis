//! One throttled, coalesced volume stream; each write is explicit user intent.
use super::*;
use crate::playback::volume::{Target, adjusted};

const INTERVAL: Duration = Duration::from_millis(100);
const COOLDOWN: Duration = Duration::from_secs(3);
const CONFIRMATION: Duration = Duration::from_millis(1200);
pub(super) struct Pending {
    id: u64,
    target: Target,
    revision: u64,
    desired: f64,
    intent: u64,
    task: Option<glib::JoinHandle<()>>,
}
pub(super) fn sample(p: &Properties, player: &str, owner: &str) -> Option<Target> {
    if !p.get("CanControl")?.get::<bool>()? {
        return None;
    }
    let value = p.get("Volume")?.get::<f64>()?;
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    Some(Target {
        player: player.into(),
        owner: owner.into(),
        value,
        observed_at: Instant::now(),
    })
}
pub(super) fn current(i: &Inner, p: &Pending) -> bool {
    let f = i.feed.borrow();
    !i.stopped
        && f.enabled
        && f.selection_revision == p.revision
        && f.active.as_deref() == Some(p.target.player.as_str())
        && i.entries.get(&p.target.player).is_some_and(|e| {
            e.owner.as_deref() == Some(p.target.owner.as_str())
                && e.volume_blocked_until
                    .is_none_or(|until| Instant::now() >= until)
                && e.volume
                    .as_ref()
                    .is_some_and(|t| t.same_player(&p.target) && t.fresh(Instant::now()))
        })
}
pub(super) fn cancel(i: &mut Inner) {
    if let Some(mut p) = i.volume.take() {
        if let Some(task) = p.task.take() {
            task.abort();
        }
    }
}
impl Monitor {
    /// Relative change to the selected PLAYER volume. No popup/status changes,
    /// no audio-server fallback, and no automatic replay after failure.
    pub fn adjust_volume(&self, delta: f64) -> Result<(), &'static str> {
        let mut i = self.0.borrow_mut();
        if i.stopped {
            return Err("Volume unavailable");
        }
        let (target, revision) = {
            let f = i.feed.borrow();
            (
                f.volume_target(Instant::now())
                    .cloned()
                    .ok_or("Volume unavailable or stale")?,
                f.selection_revision,
            )
        };
        if i.volume.as_ref().is_some_and(|p| {
            !current(&i, p) || !p.target.same_player(&target) || p.revision != revision
        }) {
            cancel(&mut i);
        }
        if let Some(p) = i.volume.as_mut() {
            if let Some(value) = adjusted(p.desired, delta) {
                p.desired = value;
                p.intent = p.intent.wrapping_add(1);
            }
            return Ok(());
        }
        let Some(desired) = adjusted(target.value, delta) else {
            return Ok(());
        };
        let connection = i
            .connection
            .clone()
            .filter(|c| !c.is_closed())
            .ok_or("Session bus unavailable")?;
        i.request_id = i.request_id.wrapping_add(1);
        let id = i.request_id;
        i.volume = Some(Pending {
            id,
            target: target.clone(),
            revision,
            desired,
            intent: 0,
            task: None,
        });
        let weak = Rc::downgrade(&self.0);
        i.volume.as_mut().unwrap().task = Some(glib::spawn_future_local(async move {
            loop {
                // A fixed window bounds traffic and preserves the latest wheel
                // destination, including reversals while a write is in flight.
                glib::timeout_future(INTERVAL).await;
                let Some(shared) = weak.upgrade() else {
                    return;
                };
                if !Self::volume_current(&shared, id) {
                    Self::finish_volume(&shared, id, None);
                    return;
                }
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
                if owner.as_deref() != Some(target.owner.as_str())
                    || !Self::volume_current(&shared, id)
                {
                    Self::finish_volume(&shared, id, None);
                    return;
                }
                let (desired, intent) = {
                    let i = shared.borrow();
                    let p = i.volume.as_ref().unwrap();
                    (p.desired, p.intent)
                };
                let result = connection
                    .call_future(
                        Some(&target.owner),
                        PATH,
                        PROPERTIES,
                        "Set",
                        Some(&(PLAYER, "Volume", desired.to_variant()).to_variant()),
                        None,
                        gio::DBusCallFlags::NO_AUTO_START,
                        TIMEOUT_MS,
                    )
                    .await;
                if !Self::volume_current(&shared, id) {
                    Self::finish_volume(&shared, id, None);
                    return;
                }
                if !result.is_ok_and(|v| v.get::<()>().is_some()) {
                    Self::finish_volume(&shared, id, Some(false));
                    return;
                }
                // Some players acknowledge Set before publishing the value.
                // Confirm with bounded reads, never by resending the write or
                // predicting success. Keep hover available while it settles.
                let actual = Self::confirm_volume(&shared, &connection, id, &target, desired).await;
                if !Self::volume_current(&shared, id) {
                    Self::finish_volume(&shared, id, None);
                    return;
                }
                let Some(actual) = actual else {
                    Self::finish_volume(&shared, id, Some(false));
                    return;
                };
                let again = {
                    let mut i = shared.borrow_mut();
                    let p = i.volume.as_ref().unwrap();
                    let again = p.intent != intent && (p.desired - actual).abs() > 1e-9;
                    if let Some(e) = i.entries.get_mut(&target.player) {
                        if let Some(t) = e.volume.as_mut() {
                            t.value = actual;
                            t.observed_at = Instant::now();
                        }
                        if let Some(properties) = e.properties.as_mut() {
                            properties.insert("Volume".into(), actual.to_variant());
                        }
                        e.volume_revision = e.volume_revision.wrapping_add(1);
                    }
                    again
                };
                if !again {
                    Self::finish_volume(&shared, id, Some(true));
                    return;
                }
                Self::publish(&shared);
            }
        }));
        Ok(())
    }
    async fn confirm_volume(
        shared: &Rc<RefCell<Inner>>,
        connection: &gio::DBusConnection,
        id: u64,
        target: &Target,
        desired: f64,
    ) -> Option<f64> {
        let deadline = Instant::now() + CONFIRMATION;
        loop {
            if !Self::volume_current(shared, id) {
                return None;
            }
            let remaining = deadline.checked_duration_since(Instant::now())?;
            let actual = connection
                .call_future(
                    Some(&target.owner),
                    PATH,
                    PROPERTIES,
                    "Get",
                    Some(&(PLAYER, "Volume").to_variant()),
                    None,
                    gio::DBusCallFlags::NO_AUTO_START,
                    remaining.as_millis().clamp(1, TIMEOUT_MS as u128) as i32,
                )
                .await
                .ok()
                .and_then(|v| v.get::<(glib::Variant,)>()?.0.get::<f64>())
                .filter(|v| v.is_finite() && *v >= 0.0)?;
            if !Self::volume_current(shared, id) {
                return None;
            }
            if (actual - desired).abs() <= 0.015 {
                return Some(actual);
            }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                tracing::debug!(player=%target.player, requested=desired, actual,
                    "MPRIS volume did not converge before confirmation deadline");
                return None;
            };
            glib::timeout_future(INTERVAL.min(remaining)).await;
        }
    }
    fn volume_current(shared: &Rc<RefCell<Inner>>, id: u64) -> bool {
        let i = shared.borrow();
        i.volume
            .as_ref()
            .is_some_and(|p| p.id == id && current(&i, p))
    }
    fn finish_volume(shared: &Rc<RefCell<Inner>>, id: u64, success: Option<bool>) {
        {
            let mut i = shared.borrow_mut();
            if !i.volume.as_ref().is_some_and(|p| p.id == id) {
                return;
            }
            let p = i.volume.take().unwrap();
            if success == Some(false) && current(&i, &p) {
                if let Some(e) = i.entries.get_mut(&p.target.player) {
                    e.volume_blocked_until = Some(Instant::now() + COOLDOWN);
                    e.volume = None;
                    e.due = Instant::now() + COOLDOWN;
                }
                tracing::debug!(player=%p.target.player,"MPRIS volume rejected, ignored or timed out; wheel intent discarded without retry");
            }
        }
        Self::publish(shared);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn volume_snapshot_requires_typed_control_and_finite_nonnegative_double() {
        let mut p = Properties::from([
            ("CanControl".into(), true.to_variant()),
            ("Volume".into(), 0.5_f64.to_variant()),
        ]);
        assert!(sample(&p, "org.mpris.MediaPlayer2.fixture", ":1.9").is_some());
        for v in [f64::NAN, f64::INFINITY, -0.1] {
            p.insert("Volume".into(), v.to_variant());
            assert!(sample(&p, "p", ":1.9").is_none());
        }
        p.insert("Volume".into(), 50_i64.to_variant());
        assert!(sample(&p, "p", ":1.9").is_none());
        p.insert("Volume".into(), 0.5_f64.to_variant());
        p.insert("CanControl".into(), false.to_variant());
        assert!(sample(&p, "p", ":1.9").is_none());
    }
}
