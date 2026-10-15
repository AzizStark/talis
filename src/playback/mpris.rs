//! Asynchronous MPRIS monitoring; writes require explicit transport or volume input.
mod control;
mod identity;
mod volume;
use super::{MAX_SNAPSHOT_AGE, PlaybackFeed, PlaybackSnapshot, RequestGate, RequestToken, Status};
use gtk::{gio, glib, prelude::*};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    rc::Rc,
    time::{Duration, Instant},
};

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const BUS: &str = "org.freedesktop.DBus";
const BUS_PATH: &str = "/org/freedesktop/DBus";
const TIMEOUT_MS: i32 = 1200;
const POLL: Duration = Duration::from_secs(2);
const MAX_PLAYERS: usize = 16;
type Properties = HashMap<String, glib::Variant>;
fn artwork_playback(properties: &Properties) -> Option<(Status, Instant)> {
    let status = match properties.get("PlaybackStatus")?.str()? {
        "Playing" => Status::Playing,
        "Paused" => Status::Paused,
        "Stopped" => Status::Stopped,
        _ => return None,
    };
    Some((status, Instant::now()))
}

fn artwork(properties: &Properties, track: Option<String>) -> Option<super::ArtworkSource> {
    let track = track?;
    let metadata = properties.get("Metadata")?.get::<Properties>()?;
    let url = metadata.get("mpris:artUrl")?.str()?;
    if !crate::artwork::source_length_allowed(url) {
        return None;
    }
    Some(super::ArtworkSource {
        track,
        url: url.into(),
    })
}

#[derive(Clone)]
struct Sample {
    status: Status,
    clock: PlaybackSnapshot,
    properties: Properties,
}

fn integer(value: &glib::Variant) -> Option<u64> {
    value
        .get::<i64>()
        .and_then(|n| u64::try_from(n).ok())
        .or_else(|| value.get::<u64>())
}
fn decode(properties: Properties, now: Instant) -> Result<Sample, &'static str> {
    let status = match properties.get("PlaybackStatus").and_then(|v| v.str()) {
        Some("Playing") => Status::Playing,
        Some("Paused") => Status::Paused,
        Some("Stopped") => Status::Stopped,
        _ => return Err("Invalid playback state"),
    };
    let position = properties
        .get("Position")
        .and_then(integer)
        .ok_or("Position unavailable")?;
    // Some players omit Rate from GetAll. Only absence gets the 1x fallback;
    // a supplied, invalid value must never silently become a valid clock.
    let rate = match properties.get("Rate") {
        None => 1.0,
        Some(value) => value.get::<f64>().ok_or("Invalid playback rate")?,
    };
    if !rate.is_finite() || rate == 0.0 {
        return Err("Invalid playback rate");
    }
    let metadata = properties
        .get("Metadata")
        .and_then(|v| v.get::<Properties>())
        .ok_or("Metadata unavailable")?;
    // Timing does not require an artwork/history identity. Totem can supply
    // a valid clock alongside NoTrack; keep identity validation independent.
    let duration = metadata.get("mpris:length").and_then(integer).unwrap_or(0);
    Ok(Sample {
        status,
        clock: PlaybackSnapshot {
            playing: status == Status::Playing,
            position: Duration::from_micros(position),
            duration: Duration::from_micros(duration),
            observed_at: now,
            rate,
        },
        properties,
    })
}

struct Entry {
    volume: Option<super::volume::Target>,
    volume_blocked_until: Option<Instant>,
    volume_revision: u64,
    transport: Option<super::transport::Target>,
    properties: Option<Properties>,
    artwork: Option<super::ArtworkSource>,
    artwork_track: Option<String>,
    artwork_identity: identity::Tracker,
    artwork_playback: Option<(Status, Instant)>,
    owner: Option<String>,
    gate: RequestGate,
    sample: Option<Sample>,
    note: String,
    due: Instant,
    failures: u32,
    task: Option<glib::JoinHandle<()>>,
}
impl Entry {
    fn playing(&self, now: Instant) -> bool {
        self.artwork_playback.is_some_and(|(status, observed)| {
            status == Status::Playing && now.saturating_duration_since(observed) <= MAX_SNAPSHOT_AGE
        })
    }
    fn fresh(&self, now: Instant) -> Option<&Sample> {
        self.sample
            .as_ref()
            .filter(|s| now.saturating_duration_since(s.clock.observed_at) <= MAX_SNAPSHOT_AGE)
    }
    fn new(now: Instant) -> Self {
        Self {
            volume: None,
            volume_blocked_until: None,
            volume_revision: 0,
            transport: None,
            properties: None,
            artwork: None,
            artwork_track: None,
            artwork_identity: identity::Tracker::default(),
            artwork_playback: None,
            owner: None,
            gate: RequestGate::default(),
            sample: None,
            note: "Waiting for timing…".into(),
            due: now,
            failures: 0,
            task: None,
        }
    }
    fn invalidate(&mut self, note: &str) {
        self.transport = None;
        self.gate.invalidate();
        self.sample = None;
        self.note = note.into();
        self.due = Instant::now();
    }
    fn update_artwork(&mut self, properties: &Properties) {
        // Use exactly the same effective identity for history and image keys.
        // Retain identity context across metadata gaps; a new owner gets a new Entry.
        self.artwork_track = self.artwork_identity.observe(properties);
        self.artwork = artwork(properties, self.artwork_track.clone());
    }
}
impl Drop for Entry {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}
struct Inner {
    volume: Option<volume::Pending>,
    control: Option<control::Pending>,
    feed: Rc<RefCell<PlaybackFeed>>,
    notify: Rc<dyn Fn()>,
    entries: BTreeMap<String, Entry>,
    connection: Option<gio::DBusConnection>,
    subscriptions: Vec<gio::SignalSubscription>,
    connecting: Option<glib::JoinHandle<()>>,
    connect_started: Option<Instant>,
    discovering: Option<glib::JoinHandle<()>>,
    source: Option<glib::SourceId>,
    catalog_revision: u64,
    request_id: u64,
    list_due: Instant,
    connect_due: Instant,
    stopped: bool,
}
impl Drop for Inner {
    fn drop(&mut self) {
        control::cancel(self);
        volume::cancel(self);
        if let Some(source) = self.source.take() {
            source.remove();
        }
        if let Some(task) = self.connecting.take() {
            task.abort();
        }
        if let Some(task) = self.discovering.take() {
            task.abort();
        }
    }
}
#[derive(Clone)]
pub struct Monitor(Rc<RefCell<Inner>>);
impl Monitor {
    pub fn start(feed: Rc<RefCell<PlaybackFeed>>, notify: impl Fn() + 'static) -> Self {
        let now = Instant::now();
        let inner = Rc::new(RefCell::new(Inner {
            volume: None,
            control: None,
            feed,
            notify: Rc::new(notify),
            entries: BTreeMap::new(),
            connection: None,
            subscriptions: Vec::new(),
            connecting: None,
            connect_started: None,
            discovering: None,
            source: None,
            catalog_revision: 0,
            request_id: 0,
            list_due: now,
            connect_due: now,
            stopped: false,
        }));
        let weak = Rc::downgrade(&inner);
        let source = glib::timeout_add_local(Duration::from_millis(250), move || {
            let Some(inner) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            Self::tick(&inner);
            glib::ControlFlow::Continue
        });
        inner.borrow_mut().source = Some(source);
        Self::tick(&inner);
        Self(inner)
    }
    pub fn stop(&self) {
        let mut inner = self.0.borrow_mut();
        inner.stopped = true;
        if let Some(source) = inner.source.take() {
            source.remove();
        }
        Self::reset(&mut inner);
    }
    fn reset(inner: &mut Inner) {
        control::cancel(inner);
        volume::cancel(inner);
        inner.feed.borrow_mut().transport = None;
        inner.feed.borrow_mut().volume = None;
        if let Some(task) = inner.connecting.take() {
            task.abort();
        }
        inner.connect_started = None;
        if let Some(task) = inner.discovering.take() {
            task.abort();
        }
        inner.entries.clear();
        inner.subscriptions.clear();
        inner.connection = None;
        inner.catalog_revision = inner.catalog_revision.wrapping_add(1);
    }
    fn tick(shared: &Rc<RefCell<Inner>>) {
        let now = Instant::now();
        {
            let mut i = shared.borrow_mut();
            if i.stopped {
                return;
            }
            if !i.feed.borrow().enabled {
                let changed = i.connection.is_some()
                    || i.connecting.is_some()
                    || i.discovering.is_some()
                    || !i.entries.is_empty()
                    || i.control.is_some();
                let changed = changed || i.volume.is_some();
                let notify = i.notify.clone();
                Self::reset(&mut i);
                drop(i);
                // publish() intentionally skips disabled feeds. Notify once on
                // teardown so the desktop adapter also removes old hit regions.
                if changed {
                    notify();
                }
                return;
            }
            if i.connect_started
                .is_some_and(|start| now.saturating_duration_since(start) >= Duration::from_secs(3))
            {
                if let Some(task) = i.connecting.take() {
                    task.abort();
                }
                i.connect_started = None;
                i.connect_due = now + Duration::from_secs(4);
                i.feed
                    .borrow_mut()
                    .invalidate("Session bus connection timed out; retrying…");
            }
            if i.connection.as_ref().is_some_and(|c| c.is_closed()) {
                Self::reset(&mut i);
                i.connect_due = now + POLL;
                i.feed
                    .borrow_mut()
                    .invalidate("Session bus disconnected; retrying…");
            }
        }
        if shared.borrow().connection.is_none() {
            Self::connect(shared);
            Self::publish(shared);
            return;
        }
        Self::discover(shared);
        let names = shared.borrow().entries.keys().cloned().collect::<Vec<_>>();
        for name in names {
            Self::query(shared, name);
        }
        Self::publish(shared);
    }
    fn connect(shared: &Rc<RefCell<Inner>>) {
        let mut i = shared.borrow_mut();
        if i.connecting.is_some() || Instant::now() < i.connect_due {
            return;
        }
        let weak = Rc::downgrade(shared);
        i.connect_started = Some(Instant::now());
        i.connecting = Some(glib::spawn_future_local(async move {
            let result = crate::session_bus::connect().await;
            let Some(shared) = weak.upgrade() else {
                return;
            };
            {
                let mut i = shared.borrow_mut();
                i.connecting = None;
                i.connect_started = None;
                if i.stopped || !i.feed.borrow().enabled {
                    return;
                }
                match result {
                    Ok(connection) if !connection.is_closed() => {
                        i.connection = Some(connection);
                        i.list_due = Instant::now();
                    }
                    _ => {
                        i.connect_due = Instant::now() + Duration::from_secs(4);
                        i.feed
                            .borrow_mut()
                            .invalidate("Session bus unavailable; retrying…");
                    }
                }
            }
            Self::subscribe(&shared);
            Self::tick(&shared);
        }));
    }
    fn subscribe(shared: &Rc<RefCell<Inner>>) {
        let Some(connection) = shared.borrow().connection.clone() else {
            return;
        };
        let weak = Rc::downgrade(shared);
        let owners = connection.subscribe_to_signal(
            Some(BUS),
            Some(BUS),
            Some("NameOwnerChanged"),
            Some(BUS_PATH),
            Some("org.mpris.MediaPlayer2"),
            gio::DBusSignalFlags::MATCH_ARG0_NAMESPACE,
            move |signal| {
                let Some((name, _, owner)) = signal.parameters.get::<(String, String, String)>()
                else {
                    return;
                };
                let Some(shared) = weak.upgrade() else {
                    return;
                };
                {
                    let mut i = shared.borrow_mut();
                    if i.stopped
                        || signal.connection.is_closed()
                        || i.connection.as_ref() != Some(signal.connection)
                    {
                        return;
                    }
                    i.catalog_revision = i.catalog_revision.wrapping_add(1);
                    i.entries.remove(&name);
                    if !owner.is_empty()
                        && name.starts_with(PREFIX)
                        && i.entries.len() < MAX_PLAYERS
                    {
                        let mut entry = Entry::new(Instant::now());
                        entry.owner = Some(owner);
                        i.entries.insert(name, entry);
                    }
                    i.list_due = Instant::now();
                }
                Self::tick(&shared);
            },
        );
        let weak = Rc::downgrade(shared);
        let changes = connection.subscribe_to_signal(
            None,
            Some(PROPERTIES),
            Some("PropertiesChanged"),
            Some(PATH),
            Some(PLAYER),
            gio::DBusSignalFlags::NONE,
            move |signal| {
                let Some((_, values, invalidated)) =
                    signal.parameters.get::<(String, Properties, Vec<String>)>()
                else {
                    return;
                };
                let Some(shared) = weak.upgrade() else {
                    return;
                };
                {
                    let mut i = shared.borrow_mut();
                    if i.stopped
                        || signal.connection.is_closed()
                        || i.connection.as_ref() != Some(signal.connection)
                    {
                        return;
                    }
                    for (name, entry) in i
                        .entries
                        .iter_mut()
                        .filter(|(_, e)| e.owner.as_deref() == Some(signal.sender_name))
                    {
                        // Volume-only signals must not invalidate arm timing or
                        // display a playback/volume notice. A separate revision
                        // prevents old volume reads without starving timing reads.
                        let control_changed = invalidated.iter().any(|k| k == "CanControl")
                            || values.get("CanControl").is_some_and(|v| {
                                entry.properties.as_ref().and_then(|p| p.get("CanControl"))
                                    != Some(v)
                            });
                        let volume_changed = ["Volume", "CanControl"].iter().any(|key| {
                            invalidated.iter().any(|k| k == key)
                                || values.get(*key).is_some_and(|v| {
                                    entry.properties.as_ref().and_then(|p| p.get(*key)) != Some(v)
                                })
                        });
                        if volume_changed {
                            let mut properties = entry.properties.clone().unwrap_or_default();
                            for key in ["Volume", "CanControl"] {
                                if let Some(value) = values.get(key) {
                                    properties.insert(key.into(), value.clone());
                                }
                                if invalidated.iter().any(|k| k == key) {
                                    properties.remove(key);
                                }
                            }
                            entry.volume = entry
                                .owner
                                .as_deref()
                                .and_then(|owner| volume::sample(&properties, name, owner));
                            entry.properties = Some(properties);
                            entry.volume_revision = entry.volume_revision.wrapping_add(1);
                            if invalidated
                                .iter()
                                .any(|k| k == "Volume" || k == "CanControl")
                            {
                                entry.due = Instant::now();
                            }
                        }
                        let changed = control_changed
                            || [
                                "Metadata",
                                "PlaybackStatus",
                                "Rate",
                                "Position",
                                "CanControl",
                                "CanPlay",
                                "CanPause",
                                "CanGoNext",
                                "CanGoPrevious",
                            ]
                            .iter()
                            .any(|key| {
                                invalidated.iter().any(|k| k == key)
                                    || values.get(*key).is_some_and(|v| {
                                        entry.properties.as_ref().and_then(|p| p.get(*key))
                                            != Some(v)
                                    })
                            });
                        if changed {
                            if values.contains_key("PlaybackStatus") {
                                entry.artwork_playback = artwork_playback(&values);
                            }
                            if invalidated.iter().any(|key| key == "PlaybackStatus") {
                                entry.artwork_playback = None;
                            }
                            if values.contains_key("Metadata") {
                                entry.update_artwork(&values);
                            }
                            if invalidated.iter().any(|key| key == "Metadata") {
                                entry.artwork = None;
                                entry.artwork_track = None;
                            }
                            entry.invalidate("Playback changed; synchronizing…");
                        }
                    }
                }
                Self::tick(&shared);
            },
        );
        let weak = Rc::downgrade(shared);
        let seeks = connection.subscribe_to_signal(
            None,
            Some(PLAYER),
            Some("Seeked"),
            Some(PATH),
            None,
            gio::DBusSignalFlags::NONE,
            move |signal| {
                let Some(shared) = weak.upgrade() else {
                    return;
                };
                {
                    let mut i = shared.borrow_mut();
                    if i.stopped
                        || signal.connection.is_closed()
                        || i.connection.as_ref() != Some(signal.connection)
                    {
                        return;
                    }
                    for entry in i
                        .entries
                        .values_mut()
                        .filter(|e| e.owner.as_deref() == Some(signal.sender_name))
                    {
                        entry.invalidate("Seek detected; synchronizing…");
                    }
                }
                Self::tick(&shared);
            },
        );
        shared.borrow_mut().subscriptions = vec![owners, changes, seeks];
    }
    fn discover(shared: &Rc<RefCell<Inner>>) {
        let mut i = shared.borrow_mut();
        if i.discovering.is_some() || Instant::now() < i.list_due {
            return;
        }
        let Some(connection) = i.connection.clone() else {
            return;
        };
        let revision = i.catalog_revision;
        let weak = Rc::downgrade(shared);
        i.discovering = Some(glib::spawn_future_local(async move {
            let result = connection
                .call_future(
                    Some(BUS),
                    BUS_PATH,
                    BUS,
                    "ListNames",
                    None,
                    None,
                    gio::DBusCallFlags::NO_AUTO_START,
                    TIMEOUT_MS,
                )
                .await;
            let Some(shared) = weak.upgrade() else {
                return;
            };
            {
                let mut i = shared.borrow_mut();
                i.discovering = None;
                if i.stopped || revision != i.catalog_revision {
                    return;
                }
                i.list_due = Instant::now() + POLL;
                match result.ok().and_then(|v| v.get::<(Vec<String>,)>()) {
                    Some((names,)) => {
                        let mut names = names
                            .into_iter()
                            .filter(|n| n.starts_with(PREFIX))
                            .collect::<Vec<_>>();
                        names.sort();
                        let preferred = i.feed.borrow().preferred.clone();
                        if let Some(preferred) = preferred {
                            if let Some(index) = names.iter().position(|n| n == &preferred) {
                                names.swap(0, index);
                            }
                        }
                        names.truncate(MAX_PLAYERS);
                        i.entries.retain(|name, _| names.contains(name));
                        for name in names {
                            i.entries
                                .entry(name)
                                .or_insert_with(|| Entry::new(Instant::now()));
                        }
                    }
                    None => {
                        for entry in i.entries.values_mut() {
                            entry.invalidate("Discovery unavailable; retrying…");
                        }
                        i.feed
                            .borrow_mut()
                            .invalidate("Player discovery unavailable; retrying…");
                    }
                }
            }
            Self::tick(&shared);
        }));
    }
    fn query(shared: &Rc<RefCell<Inner>>, name: String) {
        let mut i = shared.borrow_mut();
        let Some(connection) = i.connection.clone() else {
            return;
        };
        i.request_id = i.request_id.wrapping_add(1);
        let id = i.request_id;
        let Some(entry) = i.entries.get_mut(&name) else {
            return;
        };
        if Instant::now() < entry.due {
            return;
        }
        let Some(token) = entry.gate.begin(id) else {
            return;
        };
        let owner = entry.owner.clone();
        let volume_revision = entry.volume_revision;
        let weak = Rc::downgrade(shared);
        entry.task = Some(glib::spawn_future_local(async move {
            let owner = match owner {
                Some(owner) => Some(owner),
                None => connection
                    .call_future(
                        Some(BUS),
                        BUS_PATH,
                        BUS,
                        "GetNameOwner",
                        Some(&(name.as_str(),).to_variant()),
                        None,
                        gio::DBusCallFlags::NO_AUTO_START,
                        TIMEOUT_MS,
                    )
                    .await
                    .ok()
                    .and_then(|v| v.get::<(String,)>().map(|(owner,)| owner)),
            };
            if let Some(shared) = weak.upgrade() {
                let mut i = shared.borrow_mut();
                if i.stopped {
                    return;
                }
                let Some(entry) = i.entries.get_mut(&name) else {
                    return;
                };
                if !entry.gate.current(token) {
                    entry.gate.finish(token);
                    return;
                }
                entry.owner = owner.clone();
            } else {
                return;
            }
            // Query the unique owner, never accidentally read a replacement process.
            let result = match owner {
                Some(owner) => connection
                    .call_future(
                        Some(&owner),
                        PATH,
                        PROPERTIES,
                        "GetAll",
                        Some(&(PLAYER,).to_variant()),
                        None,
                        gio::DBusCallFlags::NO_AUTO_START,
                        TIMEOUT_MS,
                    )
                    .await
                    .ok()
                    .and_then(|v| v.get::<(Properties,)>().map(|(p,)| p))
                    .ok_or("Player unavailable or timed out"),
                None => Err("Player disconnected"),
            };
            if let Some(shared) = weak.upgrade() {
                Self::finish(&shared, &name, token, volume_revision, result);
            }
        }));
    }
    fn finish(
        shared: &Rc<RefCell<Inner>>,
        name: &str,
        token: RequestToken,
        volume_revision: u64,
        mut result: Result<Properties, &'static str>,
    ) {
        {
            let mut i = shared.borrow_mut();
            if i.stopped {
                return;
            }
            let Some(entry) = i.entries.get_mut(name) else {
                return;
            };
            if !entry.gate.finish(token) {
                return;
            }
            // Artwork is independent of timing validity (e.g. streams without Position).
            let volume_current = entry.volume_revision == volume_revision;
            if !volume_current {
                if let Ok(properties) = result.as_mut() {
                    match entry.properties.as_ref().and_then(|p| p.get("Volume")) {
                        Some(value) => {
                            properties.insert("Volume".into(), value.clone());
                        }
                        None => {
                            properties.remove("Volume");
                        }
                    }
                }
            }
            let query_ok = result.is_ok();
            entry.transport = result.as_ref().ok().and_then(|p| {
                entry
                    .owner
                    .as_deref()
                    .and_then(|owner| control::sample(p, name, owner))
            });
            entry.properties = result.as_ref().ok().cloned();
            if volume_current {
                entry.volume = result.as_ref().ok().and_then(|p| {
                    entry
                        .owner
                        .as_deref()
                        .and_then(|owner| volume::sample(p, name, owner))
                });
            }
            if let Ok(properties) = &result {
                entry.update_artwork(properties);
            } else {
                entry.artwork = None;
                entry.artwork_track = None;
            }
            entry.artwork_playback = result.as_ref().ok().and_then(artwork_playback);
            let result = result.and_then(|p| decode(p, Instant::now()));
            match result {
                Ok(sample) => {
                    let changed = entry.sample.as_ref().is_none_or(|old| {
                        old.status != sample.status
                            || old.clock.duration != sample.clock.duration
                            || old.properties.get("Metadata") != sample.properties.get("Metadata")
                    });
                    if changed {
                        tracing::info!(player=name.trim_start_matches(PREFIX), status=?sample.status,
                        position_seconds=sample.clock.position.as_secs_f64(), duration_seconds=sample.clock.duration.as_secs_f64(),
                        "MPRIS timing synchronized (read-only)");
                    }
                    entry.note = if sample.status == Status::Stopped {
                        "Playback stopped"
                    } else if sample.clock.duration.is_zero() {
                        "Duration unavailable"
                    } else {
                        "Timing available"
                    }
                    .into();
                    entry.sample = Some(sample);
                    entry.failures = 0;
                    entry.due = Instant::now() + POLL;
                }
                Err(note) => {
                    entry.sample = None;
                    entry.note = note.into();
                    entry.failures = (entry.failures + 1).min(3);
                    entry.due = Instant::now()
                        + if query_ok {
                            POLL
                        } else {
                            Duration::from_secs(1 << entry.failures)
                        };
                }
            }
        }
        Self::publish(shared);
    }
    fn publish(shared: &Rc<RefCell<Inner>>) {
        let notify = {
            let mut i = shared.borrow_mut();
            let mut feed = i.feed.borrow_mut();
            if !feed.enabled || i.stopped {
                return;
            }
            feed.players = i.entries.keys().cloned().collect();
            let now = Instant::now();
            let active = if let Some(preferred) = &feed.preferred {
                Some(preferred.clone())
            } else {
                feed.active
                    .as_ref()
                    .filter(|name| {
                        i.entries.get(*name).is_some_and(|e| {
                            (e.sample.is_none() && e.failures == 0) || e.playing(now)
                        })
                    })
                    .cloned()
                    .or_else(|| {
                        i.entries
                            .iter()
                            .find(|(_, e)| e.playing(now))
                            .map(|(n, _)| n.clone())
                    })
                    .or_else(|| {
                        feed.active
                            .as_ref()
                            .filter(|n| i.entries.contains_key(*n))
                            .cloned()
                    })
                    .or_else(|| i.entries.keys().next().cloned())
            };
            feed.active = active;
            if feed
                .control_notice_target
                .as_ref()
                .is_some_and(|(name, owner)| {
                    feed.active.as_deref() != Some(name.as_str())
                        || i.entries
                            .get(name)
                            .is_none_or(|e| e.owner.as_deref() != Some(owner.as_str()))
                })
            {
                feed.control_notice = None;
                feed.control_notice_target = None;
            }
            if let Some(entry) = feed.active.as_ref().and_then(|n| i.entries.get(n)) {
                feed.volume = entry.volume.clone().filter(|t| {
                    entry.volume_blocked_until.is_none_or(|until| now >= until) && t.fresh(now)
                });
                feed.transport = entry
                    .transport
                    .clone()
                    .filter(|t| now.saturating_duration_since(t.observed_at) <= MAX_SNAPSHOT_AGE);
                feed.artwork = entry.artwork.clone();
                feed.artwork_track = entry.artwork_track.clone();
                feed.artwork_owner = entry.owner.clone();
                feed.artwork_playback = entry.artwork_playback;
                feed.snapshot = entry.fresh(now).map(|s| s.clock.clone());
                feed.status = entry.fresh(now).map_or(Status::Stopped, |s| s.status);
                feed.notice = if entry.sample.is_some() && feed.snapshot.is_none() {
                    "Timing stale; synchronizing…".into()
                } else {
                    entry.note.clone()
                };
            } else {
                feed.volume = None;
                feed.transport = None;
                feed.artwork = None;
                feed.artwork_track = None;
                feed.artwork_owner = None;
                feed.artwork_playback = None;
                feed.snapshot = None;
                feed.status = Status::Stopped;
                if i.connection.is_some() {
                    feed.notice = "Waiting for a compatible player…".into();
                }
            }
            drop(feed);
            if i.volume.as_ref().is_some_and(|p| !volume::current(&i, p)) {
                volume::cancel(&mut i);
            }
            i.notify.clone()
        };
        notify();
    }
}

#[cfg(test)]
pub(crate) use tests::integration::{Fixture as TransportFixture, wait_for as wait_for_transport};

#[cfg(test)]
mod tests;
