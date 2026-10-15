//! Source selection and feedback; only explicit scene clicks control the player.
use super::{scheduling::ensure_animation, state::AppState, style};
use crate::playback::PlaybackFeed;
use gtk::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Instant,
};

#[derive(Clone)]
pub(super) struct MediaUi {
    pub widget: gtk::Box,
    player: gtk::ComboBoxText,
    pub label: gtk::Label,
    pub thumbnail: super::thumbnail::HeaderArtwork,
    error: gtk::Label,
    syncing: Rc<Cell<bool>>,
    catalog: Rc<RefCell<Vec<String>>>,
}
impl MediaUi {
    pub(super) fn selection_reader(&self) -> impl Fn() -> Option<String> + 'static {
        let player = self.player.downgrade();
        let syncing = self.syncing.clone();
        move || {
            if syncing.get() {
                return None;
            }
            player.upgrade()?.active_id().map(|id| {
                if id == "auto" {
                    "automatic".into()
                } else {
                    id.into()
                }
            })
        }
    }
    pub fn sync(&self, feed: &PlaybackFeed) {
        self.syncing.set(true);
        self.player.set_sensitive(feed.enabled);
        let mut names = feed.players.clone();
        if let Some(preferred) = &feed.preferred {
            if !names.contains(preferred) {
                names.push(preferred.clone());
            }
        }
        if *self.catalog.borrow() != names || self.player.active_id().is_none() {
            self.player.remove_all();
            self.player.append(Some("auto"), "Automatic");
            for name in &names {
                self.player.append(
                    Some(name),
                    &capitalized(name.trim_start_matches("org.mpris.MediaPlayer2.")),
                );
            }
            *self.catalog.borrow_mut() = names;
        }
        self.player
            .set_active_id(Some(feed.preferred.as_deref().unwrap_or("auto")));
        let text = header_status(feed, Instant::now());
        if self.label.text() != text {
            self.label.set_text(&text);
        }
        let error = control_error(feed.control_notice.as_deref());
        if self.error.text() != error.unwrap_or("") {
            self.error.set_text(error.unwrap_or(""));
            self.error.set_visible(error.is_some());
        }
        self.syncing.set(false);
    }
}

// Keep diagnostics in the playback model, not in the compact settings header.
fn header_status(feed: &PlaybackFeed, now: Instant) -> String {
    if !feed.enabled {
        return "Playback off".into();
    }
    let Some(player) = feed.active.as_deref() else {
        return "No player".into();
    };
    if feed.progress_at(now).is_some() {
        return capitalized(&feed.label(now));
    }
    capitalized(player.trim_start_matches("org.mpris.MediaPlayer2."))
}

fn capitalized(text: &str) -> String {
    let mut characters = text.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => String::new(),
    }
}

// Suppress only known routine progress/acknowledgements; preserve every error.
fn control_error(notice: Option<&str>) -> Option<&str> {
    notice.filter(|notice| {
        !matches!(
            *notice,
            "Sending Play…"
                | "Sending Pause…"
                | "Sending Previous…"
                | "Sending Next…"
                | "Play requested"
                | "Pause requested"
                | "Previous requested"
                | "Next requested"
        )
    })
}

pub(super) fn controls(
    area: &gtk::GLArea,
    state: Rc<RefCell<AppState>>,
    running: Rc<Cell<bool>>,
) -> MediaUi {
    let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let player = gtk::ComboBoxText::new();
    player.set_widget_name("playback-source");
    player.set_tooltip_text(Some(
        "Choose a player, or use Automatic to follow one that's playing.",
    ));
    player.set_size_request(185, -1);
    for cell in player.cells() {
        if let Ok(text) = cell.downcast::<gtk::CellRendererText>() {
            text.set_ellipsize(gtk::pango::EllipsizeMode::End);
            text.set_max_width_chars(20);
        }
    }
    widget.append(&style::row(
        "Music source",
        "Select a target player.",
        &player,
    ));
    let label = gtk::Label::new(None);
    label.set_widget_name("now-playing-info");
    label.set_xalign(1.0);
    label.set_single_line_mode(true);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_max_width_chars(40);
    label.add_css_class("quiet-status");
    let error = style::label("", "quiet-status");
    error.set_widget_name("playback-control-error");
    error.add_css_class("settings-feedback-source");
    error.add_css_class("card-footer");
    error.set_visible(false);
    widget.append(&error);
    let ui = MediaUi {
        widget,
        player,
        label,
        thumbnail: super::thumbnail::HeaderArtwork::new(),
        error,
        syncing: Rc::new(Cell::new(false)),
        catalog: Rc::new(RefCell::new(Vec::new())),
    };
    ui.sync(&state.borrow().playback.borrow());
    {
        let ui = ui.clone();
        let area = area.clone();
        ui.player.clone().connect_changed(move |combo| {
            if ui.syncing.get() {
                return;
            }
            let name = combo
                .active_id()
                .filter(|id| id != "auto")
                .map(|id| id.to_string());
            let feed = state.borrow().playback.clone();
            feed.borrow_mut().select(name);
            state.borrow_mut().sync_playback(Instant::now());
            ui.sync(&feed.borrow());
            super::presenter::queue_content_render(&area);
            ensure_animation(&area, state.clone(), running.clone());
        });
    }
    ui
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        arm::ArmState,
        config::{Config, PreviewMode},
    };
    #[test]
    fn header_status_keeps_missing_or_stale_timing_concise() {
        use crate::playback::{MAX_SNAPSHOT_AGE, PlaybackSnapshot, Status};
        use std::time::Duration;

        let now = Instant::now();
        let mut feed = PlaybackFeed::default();
        assert_eq!(header_status(&feed, now), "Playback off");
        feed.set_enabled(true);
        feed.notice = "Waiting for a compatible media player…".into();
        assert_eq!(header_status(&feed, now), "No player");
        feed.active = Some("org.mpris.MediaPlayer2.spotify".into());
        assert_eq!(header_status(&feed, now), "Spotify");
        feed.status = Status::Playing;
        feed.snapshot = Some(PlaybackSnapshot::demo(now));
        assert_eq!(header_status(&feed, now), "Spotify · Playing · 0:00 / 3:00");
        feed.status = Status::Paused;
        assert_eq!(header_status(&feed, now), "Spotify · Paused · 0:00 / 3:00");
        assert_eq!(
            header_status(&feed, now + MAX_SNAPSHOT_AGE + Duration::from_secs(1)),
            "Spotify"
        );
        feed.snapshot.as_mut().unwrap().duration = Duration::ZERO;
        assert_eq!(header_status(&feed, now), "Spotify");
        feed.status = Status::Stopped;
        assert_eq!(header_status(&feed, now), "Spotify");
    }
    #[test]
    fn player_display_names_capitalize_only_the_first_character() {
        for (input, output) in [
            ("spotify", "Spotify"),
            ("tauon", "Tauon"),
            ("VLC", "VLC"),
            ("élan", "Élan"),
            ("", ""),
        ] {
            assert_eq!(capitalized(input), output);
        }
    }
    #[test]
    fn routine_control_notices_are_hidden_but_errors_are_retained() {
        for method in ["Play", "Pause", "Previous", "Next"] {
            assert!(control_error(Some(&format!("Sending {method}…"))).is_none());
            assert!(control_error(Some(&format!("{method} requested"))).is_none());
        }
        assert!(control_error(None).is_none());
        for error in [
            "Play failed or timed out (not retried)",
            "Player unavailable",
            "Requested operation failed",
        ] {
            assert_eq!(control_error(Some(error)), Some(error));
        }
    }
    #[test]
    #[ignore = "Requires an isolated GTK display; run explicitly under Xvfb"]
    fn player_controls_round_trip_without_feedback() {
        gtk::init().unwrap();
        let feed = Rc::new(RefCell::new(PlaybackFeed::default()));
        let state = Rc::new(RefCell::new(AppState {
            config: Config::default(),
            previous_frame: Instant::now(),
            mode: PreviewMode::Arm,
            benchmark: false,
            gpu_profile: false,
            shell_surface: false,
            arm: ArmState::default(),
            arm_motion: crate::arm::motion::ArmMotion::new(0.0, 0.6),
            playback: feed.clone(),
            artwork_rotation: Default::default(),
            artwork_ready: false,
            vinyl_ready: false,
        }));
        let area = gtk::GLArea::new();
        let running = Rc::new(Cell::new(false));
        let ui = controls(&area, state.clone(), running);
        assert!(style::find_widget(&ui.widget.clone().upcast(), "follow-playback").is_none());
        assert!(!ui.player.is_sensitive());
        feed.borrow_mut().set_enabled(true);
        ui.sync(&feed.borrow());
        assert!(feed.borrow().enabled);
        assert!(ui.player.is_sensitive());
        feed.borrow_mut().control_notice = Some("Play requested".into());
        ui.sync(&feed.borrow());
        assert!(!ui.error.is_visible());
        assert!(!ui.label.text().contains("requested"));
        feed.borrow_mut().control_notice = Some("Play failed or timed out (not retried)".into());
        ui.sync(&feed.borrow());
        assert!(ui.error.is_visible());
        assert!(ui.error.text().contains("failed"));
        ui.error.set_visible(false);
        ui.sync(&feed.borrow());
        assert!(
            !ui.error.is_visible(),
            "dismissed errors must not reappear at every poll"
        );
        feed.borrow_mut().control_notice = Some("Player unavailable".into());
        ui.sync(&feed.borrow());
        assert!(ui.error.is_visible(), "new errors still need feedback");
        feed.borrow_mut().control_notice = None;
        ui.sync(&feed.borrow());
        assert!(!ui.error.is_visible());
        let name = "org.mpris.MediaPlayer2.test";
        feed.borrow_mut().players = vec![name.into()];
        ui.sync(&feed.borrow());
        ui.player.set_active_id(Some(name));
        assert_eq!(ui.player.active_text().as_deref(), Some("Test"));
        assert_eq!(feed.borrow().preferred.as_deref(), Some(name));
        feed.borrow_mut().players.clear();
        feed.borrow_mut().notice = "Disconnected".into();
        ui.sync(&feed.borrow());
        assert_eq!(ui.player.active_id().as_deref(), Some(name));
        assert_eq!(ui.label.text(), "No player");
        ui.player.set_active_id(Some("auto"));
        assert!(feed.borrow().preferred.is_none());
        feed.borrow_mut().set_enabled(false);
        ui.sync(&feed.borrow());
        assert!(!feed.borrow().enabled);
        assert!(!ui.player.is_sensitive());
        assert_eq!(ui.label.text(), "Playback off");
        let pose = state.borrow().arm.position;
        ui.sync(&feed.borrow());
        assert_eq!(
            state.borrow().arm.position,
            pose,
            "UI sync must not move the arm"
        );
    }
}
