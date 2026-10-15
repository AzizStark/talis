//! Artwork-only identity fallback. Never substitute it for a transport track ID.
use super::Properties;
use sha2::{Digest, Sha256};

const NO_TRACK: &str = "/org/mpris/MediaPlayer2/TrackList/NoTrack";
const FIREFOX_TRACK: &str = "/org/mpris/MediaPlayer2/firefox";
const MAX_TEXT: usize = 8192;
const MAX_METADATA: usize = 16 * 1024;

#[derive(Default)]
pub(super) struct Tracker {
    fallback: Option<Fallback>,
}
struct Fallback {
    details: Details,
    key: String,
}
#[derive(Default, serde::Serialize)]
struct Details {
    title: Option<String>,
    artist: Option<Vec<String>>,
    album: Option<String>,
    url: Option<String>,
}
fn text(value: Option<&gtk::glib::Variant>, budget: &mut usize) -> Option<Option<String>> {
    let Some(value) = value
        .and_then(|v| v.str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return Some(None);
    };
    if value.len() > MAX_TEXT || value.len() > *budget {
        return None;
    }
    *budget -= value.len();
    Some(Some(value.to_owned()))
}
impl Details {
    fn read(m: &Properties) -> Option<Self> {
        let mut budget = MAX_METADATA;
        let title = text(m.get("xesam:title"), &mut budget)?;
        let album = text(m.get("xesam:album"), &mut budget)?;
        let url = text(m.get("xesam:url"), &mut budget)?;
        let mut artist = Vec::new();
        if let Some(v) = m.get("xesam:artist").filter(|v| v.type_().as_str() == "as") {
            if v.n_children() > 32 {
                return None;
            }
            for n in 0..v.n_children() {
                if let Some(value) = text(Some(&v.child_value(n)), &mut budget)? {
                    artist.push(value);
                }
            }
        }
        // Album/artist alone cannot distinguish songs on the same album.
        if title.is_none() && url.is_none() {
            return None;
        }
        Some(Self {
            title,
            artist: (!artist.is_empty()).then_some(artist),
            album,
            url,
        })
    }
    fn conflicts(&self, other: &Self) -> bool {
        fn changed<T: PartialEq>(a: &Option<T>, b: &Option<T>) -> bool {
            a.as_ref().zip(b.as_ref()).is_some_and(|(a, b)| a != b)
        }
        changed(&self.title, &other.title)
            || changed(&self.artist, &other.artist)
            || changed(&self.album, &other.album)
            || changed(&self.url, &other.url)
    }
    fn enrich(&mut self, other: Self) {
        if other.title.is_some() {
            self.title = other.title;
        }
        if other.artist.is_some() {
            self.artist = other.artist;
        }
        if other.album.is_some() {
            self.album = other.album;
        }
        if other.url.is_some() {
            self.url = other.url;
        }
    }
}
impl Tracker {
    pub(super) fn observe(&mut self, properties: &Properties) -> Option<String> {
        let m = properties.get("Metadata")?.get::<Properties>()?;
        let raw = m
            .get("mpris:trackid")
            .and_then(|v| v.str())
            .unwrap_or("")
            .trim();
        if raw == NO_TRACK || raw.len() > MAX_TEXT {
            return None;
        }
        if !raw.is_empty() && raw != FIREFOX_TRACK {
            // A proper per-song ID is authoritative; tag edits are not skips.
            self.fallback = None;
            return Some(raw.to_owned());
        }
        let details = Details::read(&m)?;
        if let Some(old) = &mut self.fallback {
            if !old.details.conflicts(&details) {
                old.details.enrich(details);
                return Some(old.key.clone());
            }
        }
        // Structured serialization avoids delimiter collisions. No art URL,
        // duration, position, playback status or private metadata enters logs.
        let encoded = serde_json::to_vec(&details).ok()?;
        let key = format!("metadata:{:x}", Sha256::digest(encoded));
        self.fallback = Some(Fallback {
            details,
            key: key.clone(),
        });
        Some(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk::glib::variant::ToVariant;
    fn properties(id: Option<&str>, title: Option<&str>) -> Properties {
        let mut m = Properties::new();
        if let Some(id) = id {
            m.insert("mpris:trackid".into(), id.to_variant());
        }
        if let Some(title) = title {
            m.insert("xesam:title".into(), title.to_variant());
        }
        Properties::from([("Metadata".into(), m.to_variant())])
    }
    fn set(p: &mut Properties, name: &str, value: gtk::glib::Variant) {
        let mut m = p["Metadata"].get::<Properties>().unwrap();
        m.insert(name.into(), value);
        p.insert("Metadata".into(), m.to_variant());
    }
    #[test]
    fn unique_ids_are_authoritative_even_when_tags_change() {
        let mut t = Tracker::default();
        assert_eq!(
            t.observe(&properties(Some("/track/a"), Some("original")))
                .as_deref(),
            Some("/track/a")
        );
        assert_eq!(
            t.observe(&properties(Some("/track/a"), Some("edited")))
                .as_deref(),
            Some("/track/a")
        );
        assert_eq!(
            t.observe(&properties(Some("/track/b"), Some("edited")))
                .as_deref(),
            Some("/track/b")
        );
    }
    #[test]
    fn firefox_and_missing_ids_distinguish_songs_on_the_same_album() {
        for raw in [Some(FIREFOX_TRACK), None, Some("")] {
            let mut t = Tracker::default();
            let mut p = properties(raw, Some("First"));
            set(&mut p, "xesam:album", "Album".to_variant());
            let a = t.observe(&p).unwrap();
            set(&mut p, "xesam:title", "Second".to_variant());
            let b = t.observe(&p).unwrap();
            assert_ne!(a, b);
            set(&mut p, "xesam:title", "First".to_variant());
            assert_eq!(t.observe(&p).unwrap(), a);
        }
    }
    #[test]
    fn artwork_refresh_seek_status_and_duration_do_not_change_identity() {
        let mut t = Tracker::default();
        let mut p = properties(Some(FIREFOX_TRACK), Some("Song"));
        let a = t.observe(&p).unwrap();
        for url in ["file:///first.png", "file:///refreshed.png"] {
            set(&mut p, "mpris:artUrl", url.to_variant());
            set(&mut p, "mpris:length", 200_000_000_i64.to_variant());
            p.insert("Position".into(), 90_000_000_i64.to_variant());
            p.insert("PlaybackStatus".into(), "Paused".to_variant());
            assert_eq!(t.observe(&p).unwrap(), a);
        }
    }
    #[test]
    fn incomplete_and_enriched_metadata_preserves_the_existing_identity() {
        let mut t = Tracker::default();
        let mut p = properties(Some(FIREFOX_TRACK), Some("Song"));
        let a = t.observe(&p).unwrap();
        assert!(t.observe(&Properties::new()).is_none());
        assert!(t.observe(&properties(Some(FIREFOX_TRACK), None)).is_none());
        set(&mut p, "xesam:artist", vec!["Artist"].to_variant());
        set(&mut p, "xesam:album", "Album".to_variant());
        assert_eq!(t.observe(&p).unwrap(), a);
        assert_eq!(
            t.observe(&properties(Some(FIREFOX_TRACK), Some(" Song ")))
                .unwrap(),
            a
        );
        assert_eq!(t.observe(&p).unwrap(), a);
    }
    #[test]
    fn generic_id_disappearing_and_returning_is_not_a_track_switch() {
        let mut t = Tracker::default();
        let a = t
            .observe(&properties(Some(FIREFOX_TRACK), Some("Song")))
            .unwrap();
        for raw in [None, Some(""), Some(FIREFOX_TRACK)] {
            assert_eq!(t.observe(&properties(raw, Some("Song"))).unwrap(), a);
        }
        assert_ne!(t.observe(&properties(None, Some("Next song"))).unwrap(), a);
    }
    #[test]
    fn artist_album_and_track_url_can_distinguish_same_named_songs() {
        for field in ["xesam:artist", "xesam:album", "xesam:url"] {
            let mut t = Tracker::default();
            let mut p = properties(Some(FIREFOX_TRACK), Some("Song"));
            let value = |s: &str| {
                if field == "xesam:artist" {
                    vec![s].to_variant()
                } else {
                    s.to_variant()
                }
            };
            set(&mut p, field, value("first"));
            let a = t.observe(&p).unwrap();
            set(&mut p, field, value("second"));
            assert_ne!(t.observe(&p).unwrap(), a);
        }
    }
    #[test]
    fn unusable_oversized_and_no_track_metadata_is_not_an_identity() {
        let mut t = Tracker::default();
        assert!(
            t.observe(&properties(Some(NO_TRACK), Some("old stale title")))
                .is_none()
        );
        assert!(
            t.observe(&properties(Some(&"x".repeat(MAX_TEXT + 1)), Some("title")))
                .is_none()
        );
        assert!(
            t.observe(&properties(
                Some(FIREFOX_TRACK),
                Some(&"x".repeat(MAX_TEXT + 1))
            ))
            .is_none()
        );
        let mut p = properties(None, Some("title"));
        set(&mut p, "xesam:artist", vec!["artist"; 33].to_variant());
        assert!(t.observe(&p).is_none());
        set(
            &mut p,
            "xesam:artist",
            vec!["x".repeat(MAX_TEXT); 3].to_variant(),
        );
        assert!(t.observe(&p).is_none());
        let mut p = properties(None, None);
        set(&mut p, "xesam:title", 10_i64.to_variant());
        assert!(t.observe(&p).is_none());
        set(
            &mut p,
            "xesam:url",
            "https://example.org/track/one".to_variant(),
        );
        assert!(t.observe(&p).is_some());
    }
}
