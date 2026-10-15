//! Scene-normalized transport hotspots, independent of the finished button artwork.
use crate::playback::transport::Command;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TransportSettings {
    pub enabled: bool,
    /// Top-left x/y, width/height of projected elliptical button faces.
    pub play_pause: [f64; 4],
    pub previous: [f64; 4],
    pub next: [f64; 4],
    /// Current fixed-camera knob silhouette; wheel-only, no visual treatment.
    pub volume: [f64; 4],
}
impl Default for TransportSettings {
    fn default() -> Self {
        // Projected from the ORIGINAL scene already rendered in the app, not
        // the user's in-progress replacement scene. Update with new beauty later.
        Self {
            enabled: true,
            play_pause: [0.3576794863, 0.6391354501, 0.0197497606, 0.0134078562],
            previous: [0.3829452097, 0.6451604068, 0.0101774633, 0.0073829293],
            next: [0.3994382620, 0.6451604068, 0.0100741386, 0.0073829293],
            volume: [0.6222092509, 0.6178158224, 0.0193424225, 0.0323154032],
        }
    }
}
impl TransportSettings {
    /// The fixed-camera wordmark on the front-left wooden turntable face.
    /// Independent of player capabilities and the transport-enabled setting.
    pub fn project_logo(width: i32, height: i32, scale: i32, aspect: f32) -> Option<Ellipse> {
        Self::default().project_bounds([0.348, 0.674, 0.031, 0.035], width, height, scale, aspect)
    }
    pub fn bounds(&self, command: Command) -> [f64; 4] {
        match command {
            Command::PlayPause => self.play_pause,
            Command::Previous => self.previous,
            Command::Next => self.next,
        }
    }
    pub fn validate(&self) -> Result<()> {
        let rectangles = [self.previous, self.play_pause, self.next, self.volume];
        for [x, y, w, h] in rectangles {
            ensure!(
                [x, y, w, h].iter().all(|v| v.is_finite())
                    && x >= 0.0
                    && y >= 0.0
                    && w > 0.0
                    && h > 0.0
                    && w <= 0.15
                    && h <= 0.15
                    && x + w <= 1.0
                    && y + h <= 1.0,
                "Transport hotspots must be finite scene rectangles, at most 15% per axis"
            );
        }
        for button in [self.previous, self.play_pause, self.next] {
            let [x, y, w, h] = self.volume;
            let [bx, by, bw, bh] = button;
            ensure!(
                x + w <= bx || bx + bw <= x || y + h <= by || by + bh <= y,
                "Volume knob must not overlap playback buttons"
            );
        }
        for (a, b) in [
            (Command::Previous, Command::PlayPause),
            (Command::Previous, Command::Next),
            (Command::PlayPause, Command::Next),
        ] {
            let [x, y, w, h] = self.bounds(a);
            let [bx, by, bw, bh] = self.bounds(b);
            ensure!(
                x + w <= bx || bx + bw <= x || y + h <= by || by + bh <= y,
                "Transport hotspots must not overlap"
            );
        }
        Ok(())
    }
    pub fn project(
        &self,
        command: Command,
        width: i32,
        height: i32,
        scale: i32,
        aspect: f32,
    ) -> Option<Ellipse> {
        self.project_bounds(self.bounds(command), width, height, scale, aspect)
    }
    pub fn project_volume(
        &self,
        width: i32,
        height: i32,
        scale: i32,
        aspect: f32,
    ) -> Option<Ellipse> {
        self.project_bounds(self.volume, width, height, scale, aspect)
    }
    fn project_bounds(
        &self,
        bounds: [f64; 4],
        width: i32,
        height: i32,
        scale: i32,
        aspect: f32,
    ) -> Option<Ellipse> {
        if !self.enabled
            || width <= 0
            || height <= 0
            || scale <= 0
            || !aspect.is_finite()
            || aspect <= 0.0
        {
            return None;
        }
        let pw = width.checked_mul(scale)?;
        let ph = height.checked_mul(scale)?;
        let (vx, vy, vw, vh) = fitted_viewport(pw, ph, aspect);
        let viewport = crate::desktop::geometry::Rect::from_gl([vx, vy, vw, vh], ph)?;
        let [x, y, w, h] = bounds;
        let s = scale as f64;
        // GL viewport is bottom-up; mouse/surface coordinates are top-down.
        Some(Ellipse {
            x: (viewport.x as f64 + x * vw as f64) / s,
            y: (viewport.y as f64 + y * vh as f64) / s,
            w: w * vw as f64 / s,
            h: h * vh as f64 / s,
        })
    }
}

/// The renderer and hit testing use the exact same fixed-camera fitting.
pub fn fitted_viewport(width: i32, height: i32, aspect: f32) -> (i32, i32, i32, i32) {
    if width as f32 / height as f32 > aspect {
        let w = (height as f32 * aspect).round() as i32;
        ((width - w) / 2, 0, w, height)
    } else {
        let h = (width as f32 / aspect).round() as i32;
        (0, (height - h) / 2, width, h)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ellipse {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}
impl Ellipse {
    pub fn contains(self, x: f64, y: f64) -> bool {
        x.is_finite()
            && y.is_finite()
            && self.w > 0.0
            && self.h > 0.0
            && ((x - self.x - self.w / 2.0) / (self.w / 2.0)).powi(2)
                + ((y - self.y - self.h / 2.0) / (self.h / 2.0)).powi(2)
                <= 1.0
    }
    /// Cairo/GDK input regions use logical integer coordinates. Only pixels
    /// whose centres are in the ellipse are accepted, not its bounding box.
    pub fn spans(self, offset: [f64; 2]) -> Vec<[i32; 4]> {
        let mut result = Vec::new();
        let shifted = Self {
            x: self.x + offset[0],
            y: self.y + offset[1],
            ..self
        };
        for y in shifted.y.floor() as i32..(shifted.y + shifted.h).ceil() as i32 {
            let dy = (y as f64 + 0.5 - shifted.y - shifted.h / 2.0) / (shifted.h / 2.0);
            if dy.abs() > 1.0 {
                continue;
            }
            let dx = shifted.w / 2.0 * (1.0 - dy * dy).sqrt();
            let centre = shifted.x + shifted.w / 2.0;
            let left = (centre - dx - 0.5).ceil() as i32;
            let right = (centre + dx - 0.5).floor() as i32;
            if right >= left {
                result.push([left, y, right - left + 1, 1]);
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn projection_matches_rendered_viewport_at_all_sizes_and_scales() {
        let s = TransportSettings::default();
        s.validate().unwrap();
        for (w, h) in [
            (1280, 720),
            (1920, 1080),
            (2560, 1440),
            (3840, 2160),
            (3440, 1440),
            (1600, 1000),
        ] {
            for scale in [1, 2, 3] {
                for c in Command::ALL {
                    let e = s.project(c, w, h, scale, 16.0 / 9.0).unwrap();
                    assert!(e.contains(e.x + e.w / 2.0, e.y + e.h / 2.0));
                    assert!(!e.contains(e.x, e.y));
                    assert!(!e.contains(f64::NAN, e.y));
                    let (vx, vy, vw, vh) = fitted_viewport(w * scale, h * scale, 16.0 / 9.0);
                    let [x, y, _, _] = s.bounds(c);
                    assert!((e.x * scale as f64 - vx as f64 - x * vw as f64).abs() < 1e-6);
                    assert!(
                        (e.y * scale as f64 - (h * scale - vy - vh) as f64 - y * vh as f64).abs()
                            < 1e-6
                    );
                    for [x, y, w, _] in e.spans([0.0, 0.0]) {
                        for x in x..x + w {
                            assert!(e.contains(x as f64 + 0.5, y as f64 + 0.5));
                        }
                    }
                }
            }
        }
        assert!(s.project(Command::Next, 0, 720, 1, 16.0 / 9.0).is_none());
        assert!(s.project(Command::Next, 1280, 720, 0, 16.0 / 9.0).is_none());
        assert!(
            s.project(Command::Next, i32::MAX, 720, 2, 16.0 / 9.0)
                .is_none()
        );
    }
    #[test]
    fn old_configs_keep_defaults_and_unsafe_hotspots_are_rejected() {
        let s: TransportSettings = toml::from_str("").unwrap();
        assert_eq!(s, TransportSettings::default());
        assert_eq!(
            toml::from_str::<TransportSettings>(&toml::to_string(&s).unwrap()).unwrap(),
            s
        );
        for bounds in [
            [f64::NAN, 0.0, 0.01, 0.01],
            [0.9, 0.9, 0.2, 0.2],
            [0.0, 0.0, 0.0, 0.01],
        ] {
            let mut s = s.clone();
            s.next = bounds;
            assert!(s.validate().is_err());
        }
        let mut overlapping = s.clone();
        overlapping.next = overlapping.previous;
        assert!(overlapping.validate().is_err());
        overlapping = s.clone();
        overlapping.volume = overlapping.previous;
        assert!(overlapping.validate().is_err());
        overlapping = s.clone();
        overlapping.volume[0] = f64::INFINITY;
        assert!(overlapping.validate().is_err());
    }
    #[test]
    fn knob_projection_matches_buttons_and_only_contains_its_ellipse() {
        let s = TransportSettings::default();
        for (w, h) in [(1920, 1080), (2560, 1440), (3440, 1440), (3840, 2160)] {
            for scale in [1, 2] {
                let e = s.project_volume(w, h, scale, 16.0 / 9.0).unwrap();
                assert!(e.contains(e.x + e.w / 2.0, e.y + e.h / 2.0));
                assert!(!e.contains(e.x, e.y));
                for c in Command::ALL {
                    let b = s.project(c, w, h, scale, 16.0 / 9.0).unwrap();
                    assert!(!e.contains(b.x + b.w / 2.0, b.y + b.h / 2.0));
                }
            }
        }
    }
    #[test]
    fn logo_projection_scales_without_overlapping_transport() {
        for (w, h) in [(1920, 1080), (2560, 1440), (3840, 2160), (2048, 1120)] {
            for scale in [1, 2] {
                let e = TransportSettings::project_logo(w, h, scale, 16.0 / 9.0).unwrap();
                assert!(e.contains(e.x + e.w / 2.0, e.y + e.h / 2.0));
                assert!(!e.contains(e.x, e.y));
                for c in Command::ALL {
                    let b = TransportSettings::default()
                        .project(c, w, h, scale, 16.0 / 9.0)
                        .unwrap();
                    assert!(b.y + b.h < e.y);
                }
            }
        }
        assert!(TransportSettings::project_logo(0, 720, 1, 16.0 / 9.0).is_none());
    }
}
