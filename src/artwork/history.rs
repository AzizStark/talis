//! Session-local previous-track identity. No timers, network, or renderer state.
use super::Key;
use crate::playback::PlaybackFeed;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Scope {
    player: String,
    owner: Option<String>,
}
#[derive(Default)]
pub(super) struct History {
    selection: Option<u64>,
    scope: Option<Scope>,
    current: Option<(String, Option<Key>)>,
    previous: Option<Key>,
}
impl History {
    pub(super) fn observe(&mut self, feed: &PlaybackFeed, artwork: Option<&Key>) -> Option<Key> {
        // Use the same playback-state decision as the exposed steel holder.
        // Stops/disconnection end history; pause and transient metadata gaps do not.
        let visible = feed.sleeve_visible();
        if !visible || self.selection.is_some_and(|s| s != feed.selection_revision) {
            self.scope = None;
            self.current = None;
            self.previous = None;
        }
        self.selection = Some(feed.selection_revision);
        if !visible {
            return None;
        }
        let player = feed.active.as_ref()?;
        let scope = Scope {
            player: player.clone(),
            owner: feed.artwork_owner.clone(),
        };
        if self.scope.as_ref() != Some(&scope) {
            self.current = None;
            self.previous = None;
            self.scope = Some(scope);
        }
        let track = feed
            .artwork_track
            .as_deref()
            .or_else(|| artwork.map(|a| a.track.as_str()));
        let Some(track) = track else {
            return self.previous.clone();
        };
        if self.current.as_ref().is_some_and(|(id, _)| id != track) {
            self.previous = self.current.take().and_then(|(_, source)| source);
        }
        if let Some((_, source)) = &mut self.current {
            if artwork.is_some() {
                *source = artwork.cloned();
            }
        } else {
            self.current = Some((track.to_owned(), artwork.cloned()));
        }
        self.previous.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playback::Status;
    fn feed(track: &str, art: bool) -> (PlaybackFeed, Option<Key>) {
        let f = PlaybackFeed {
            enabled: true,
            active: Some("player".into()),
            artwork_owner: Some(":1.5".into()),
            artwork_track: Some(track.into()),
            status: Status::Playing,
            ..Default::default()
        };
        let key = art.then(|| Key {
            player: f.active.clone(),
            owner: f.artwork_owner.clone(),
            track: track.into(),
            url: format!("file:///{track}.png").into(),
        });
        (f, key)
    }
    #[test]
    fn only_new_track_advances_history_not_seek_pause_url_or_metadata_gaps() {
        let mut h = History::default();
        let (mut f, a) = feed("a", true);
        assert_eq!(h.observe(&f, a.as_ref()), None);
        f.status = Status::Paused;
        assert_eq!(h.observe(&f, a.as_ref()), None);
        f.status = Status::Playing;
        let mut updated = a.clone().unwrap();
        updated.url = "file:///refresh.png".into();
        assert_eq!(h.observe(&f, Some(&updated)), None);
        f.artwork_track = None;
        assert_eq!(h.observe(&f, None), None);
        let (f, b) = feed("b", true);
        assert_eq!(h.observe(&f, b.as_ref()), Some(updated));
        assert_eq!(h.observe(&f, b.as_ref()), h.previous);
    }
    #[test]
    fn rapid_skips_missing_art_and_reverse_skip_keep_immediate_track_not_last_good_image() {
        let mut h = History::default();
        for (track, art, expected) in [
            ("a", true, None),
            ("b", true, Some("a")),
            ("c", false, Some("b")),
            ("d", true, None),
            ("b", true, Some("d")),
        ] {
            let (f, k) = feed(track, art);
            assert_eq!(
                h.observe(&f, k.as_ref()).as_ref().map(|k| k.track.as_str()),
                expected
            );
        }
    }
    #[test]
    fn stop_and_disappearance_clear_history_but_pause_keeps_it() {
        for disconnected in [false, true] {
            let mut h = History::default();
            let (a, ak) = feed("a", true);
            h.observe(&a, ak.as_ref());
            let (mut b, bk) = feed("b", true);
            assert_eq!(h.observe(&b, bk.as_ref()), ak);
            b.status = Status::Paused;
            assert_eq!(h.observe(&b, bk.as_ref()), ak);
            if disconnected {
                b.active = None;
            } else {
                b.status = Status::Stopped;
            }
            assert!(!b.sleeve_visible());
            for _ in 0..3 {
                assert_eq!(h.observe(&b, bk.as_ref()), None);
            }
            b.active = Some("player".into());
            b.status = Status::Paused;
            assert_eq!(h.observe(&b, bk.as_ref()), None);
            let (c, ck) = feed("c", true);
            assert_eq!(h.observe(&c, ck.as_ref()), bk);
        }
    }
    #[test]
    fn artwork_status_matches_stand_even_when_timing_is_unavailable() {
        let mut h = History::default();
        let (a, ak) = feed("a", true);
        h.observe(&a, ak.as_ref());
        let (mut b, bk) = feed("b", true);
        h.observe(&b, bk.as_ref());
        b.status = Status::Stopped;
        b.artwork_playback = Some((Status::Paused, std::time::Instant::now()));
        assert!(b.sleeve_visible());
        assert_eq!(h.observe(&b, bk.as_ref()), ak);
        b.status = Status::Playing;
        b.artwork_playback = Some((Status::Stopped, std::time::Instant::now()));
        assert!(!b.sleeve_visible());
        assert_eq!(h.observe(&b, bk.as_ref()), None);
    }
    #[test]
    fn selection_owner_player_and_disabled_tracking_reset_history() {
        let mut h = History::default();
        let (a, ak) = feed("a", true);
        h.observe(&a, ak.as_ref());
        let (mut b, bk) = feed("b", true);
        h.observe(&b, bk.as_ref());
        b.artwork_owner = Some(":1.6".into());
        assert_eq!(h.observe(&b, bk.as_ref()), None);
        h.observe(&a, ak.as_ref());
        h.observe(&b, bk.as_ref());
        b.selection_revision += 1;
        assert_eq!(h.observe(&b, bk.as_ref()), None);
        b.active = Some("other".into());
        assert_eq!(h.observe(&b, bk.as_ref()), None);
        b.enabled = false;
        assert_eq!(h.observe(&b, bk.as_ref()), None);
    }
}
