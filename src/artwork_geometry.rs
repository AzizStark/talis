//! Scene-normalized projective placement; independent of GTK and image fetching.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct ArtworkSettings {
    pub enabled: bool,
    /// Display the same artwork as a stationary, lit album sleeve.
    pub sleeve_enabled: bool,
    /// Artistic lift of low-illumination sleeve regions, 0..=1; not image exposure.
    pub sleeve_shadow_lift: f64,
    /// Apply the selected scene's baked lighting to the rotating label.
    pub record_lighting: bool,
    /// Internal diagnostic override; legacy user values now use automatic rotation.
    #[serde(skip_serializing, deserialize_with = "automatic_vinyl_rotation")]
    pub vinyl_rotation: bool,
    /// Scale authored dust/scratch alpha, 0..=1; 1 preserves Blender opacity.
    pub vinyl_opacity: f64,
    /// Scale dust/scratch linear colour only, 0..=2; not record exposure.
    pub vinyl_brightness: f64,
    /// Circle diameter as a percentage of the perspective-fitted vinyl.
    #[serde(alias = "diameter_percent")]
    pub size_percent: f64,
    /// Clockwise turns per minute; zero freezes the current angle.
    pub rotation_rpm: f64,
    /// Top-left, top-right, bottom-right, bottom-left of the projected square.
    /// Coordinates are fractions of the authored scene, not the window.
    pub corners: [[f64; 2]; 4],
}
fn automatic_vinyl_rotation<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> Result<bool, D::Error> {
    let _legacy_preference = bool::deserialize(decoder)?;
    Ok(true)
}
impl Default for ArtworkSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            sleeve_enabled: true,
            sleeve_shadow_lift: 0.25,
            record_lighting: true,
            vinyl_rotation: true,
            vinyl_opacity: 1.0,
            vinyl_brightness: 1.0,
            size_percent: 36.0,
            rotation_rpm: crate::artwork_rotation::DEFAULT_RPM,
            corners: [
                [0.38203880, 0.49312621],
                [0.57033771, 0.49312621],
                [0.58017546, 0.62102577],
                [0.36554030, 0.62102577],
            ],
        }
    }
}
impl ArtworkSettings {
    /// Scene UV -> source UV, column-major for OpenGL. Reject folded/degenerate quads.
    pub fn transform(&self) -> Result<[f32; 9]> {
        ensure!(
            self.vinyl_opacity.is_finite() && (0.0..=1.0).contains(&self.vinyl_opacity),
            "Vinyl surface opacity must be a finite value between 0 and 1"
        );
        ensure!(
            self.vinyl_brightness.is_finite() && (0.0..=2.0).contains(&self.vinyl_brightness),
            "Vinyl surface brightness must be a finite value between 0 and 2"
        );
        ensure!(
            self.sleeve_shadow_lift.is_finite() && (0.0..=1.0).contains(&self.sleeve_shadow_lift),
            "Sleeve shadow lift must be a finite value between 0 and 1"
        );
        ensure!(
            self.rotation_rpm.is_finite()
                && (0.0..=crate::artwork_rotation::MAX_RPM).contains(&self.rotation_rpm),
            "Artwork rotation speed must be a finite RPM between 0 and 120"
        );
        ensure!(
            self.size_percent.is_finite() && (5.0..=100.0).contains(&self.size_percent),
            "Artwork size must be a finite percentage between 5 and 100"
        );
        let p = self.corners;
        ensure!(
            p.iter()
                .flatten()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
            "Artwork corners must be finite scene coordinates between 0 and 1"
        );
        for i in 0..4 {
            let a = p[i];
            let b = p[(i + 1) % 4];
            let c = p[(i + 2) % 4];
            ensure!(
                (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]) > 1e-7,
                "Artwork corners must form a clockwise, non-degenerate convex square"
            );
        }
        let uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let mut rows = [[0.0_f64; 9]; 8];
        for i in 0..4 {
            let [x, y] = p[i];
            let [u, v] = uv[i];
            rows[2 * i] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y, u];
            rows[2 * i + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y, v];
        }
        for column in 0..8 {
            let pivot = (column..8)
                .max_by(|&a, &b| rows[a][column].abs().total_cmp(&rows[b][column].abs()))
                .unwrap();
            ensure!(
                rows[pivot][column].abs() > 1e-10,
                "Artwork perspective is singular"
            );
            rows.swap(column, pivot);
            let divisor = rows[column][column];
            for value in &mut rows[column][column..] {
                *value /= divisor;
            }
            for row in 0..8 {
                if row == column {
                    continue;
                }
                let factor = rows[row][column];
                let pivot_row = rows[column];
                for (value, pivot_value) in rows[row][column..].iter_mut().zip(&pivot_row[column..])
                {
                    *value -= factor * pivot_value;
                }
            }
        }
        let h: Vec<_> = rows.iter().map(|row| row[8]).collect();
        let mut transform = [
            h[0] as f32,
            h[3] as f32,
            h[6] as f32,
            h[1] as f32,
            h[4] as f32,
            h[7] as f32,
            h[2] as f32,
            h[5] as f32,
            1.0,
        ];
        // Scale in the record's plane, not screen space, to retain perspective
        // and the projected center. The entire image shrinks with the circle.
        let scale = (100.0 / self.size_percent) as f32;
        let offset = 0.5 * (1.0 - scale);
        for column in 0..3 {
            let base = column * 3;
            transform[base] = transform[base] * scale + offset * transform[base + 2];
            transform[base + 1] = transform[base + 1] * scale + offset * transform[base + 2];
        }
        Ok(transform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_surface_rotation_switch_uses_automatic_rotation() {
        for legacy in ["true", "false"] {
            let settings: ArtworkSettings =
                toml::from_str(&format!("vinyl_rotation = {legacy}")).unwrap();
            assert!(settings.vinyl_rotation);
            assert!(
                !toml::to_string(&settings)
                    .unwrap()
                    .contains("vinyl_rotation")
            );
        }
    }
    #[test]
    fn vinyl_adjustments_default_validate_and_roundtrip_without_changing_fit() {
        let mut settings: ArtworkSettings = toml::from_str("vinyl_rotation = true").unwrap();
        assert_eq!(
            (settings.vinyl_opacity, settings.vinyl_brightness),
            (1.0, 1.0)
        );
        let fit = settings.transform().unwrap();
        for opacity in [0.0, 0.25, 0.5, 1.0] {
            for brightness in [0.0, 0.5, 1.0, 1.5, 2.0] {
                settings.vinyl_opacity = opacity;
                settings.vinyl_brightness = brightness;
                assert_eq!(settings.transform().unwrap(), fit);
                assert_eq!(
                    toml::from_str::<ArtworkSettings>(&toml::to_string(&settings).unwrap())
                        .unwrap(),
                    settings
                );
            }
        }
        for invalid in [-0.1, 1.1, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            settings.vinyl_opacity = invalid;
            assert!(settings.transform().is_err());
        }
        settings.vinyl_opacity = 1.0;
        for invalid in [-0.1, 2.1, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            settings.vinyl_brightness = invalid;
            assert!(settings.transform().is_err());
        }
    }
    #[test]
    fn sleeve_shadow_lift_defaults_validates_and_leaves_record_fit_unchanged() {
        let mut settings: ArtworkSettings = toml::from_str("size_percent=35.0").unwrap();
        assert_eq!(settings.sleeve_shadow_lift, 0.25);
        let fit = settings.transform().unwrap();
        for lift in [0.0, 0.25, 0.5, 1.0] {
            settings.sleeve_shadow_lift = lift;
            assert_eq!(settings.transform().unwrap(), fit);
            assert_eq!(
                toml::from_str::<ArtworkSettings>(&toml::to_string(&settings).unwrap()).unwrap(),
                settings
            );
        }
        for lift in [-0.1, 1.1, f64::NAN, f64::INFINITY] {
            settings.sleeve_shadow_lift = lift;
            assert!(settings.transform().is_err());
        }
    }
    #[test]
    fn sleeve_visibility_defaults_on_and_does_not_change_record_fit() {
        let mut settings = ArtworkSettings::default();
        assert!(settings.sleeve_enabled);
        let fit = settings.transform().unwrap();
        settings.sleeve_enabled = false;
        assert_eq!(settings.transform().unwrap(), fit);
        let saved: ArtworkSettings = toml::from_str(&toml::to_string(&settings).unwrap()).unwrap();
        assert!(!saved.sleeve_enabled);
        assert!(
            toml::from_str::<ArtworkSettings>("size_percent=35.0")
                .unwrap()
                .sleeve_enabled
        );
    }
    #[test]
    fn speed_defaults_validates_and_roundtrips_without_affecting_fit() {
        let mut settings: ArtworkSettings = toml::from_str("size_percent = 30.0").unwrap();
        assert_eq!(settings.rotation_rpm, crate::artwork_rotation::DEFAULT_RPM);
        let fit = settings.transform().unwrap();
        for rpm in [0.0, 12.5, 45.0, 78.0, 120.0] {
            settings.rotation_rpm = rpm;
            assert_eq!(settings.transform().unwrap(), fit);
            let saved = toml::to_string(&settings).unwrap();
            assert_eq!(toml::from_str::<ArtworkSettings>(&saved).unwrap(), settings);
        }
        for rpm in [-0.1, 120.1, f64::NAN, f64::INFINITY] {
            settings.rotation_rpm = rpm;
            assert!(settings.transform().is_err());
        }
    }
    #[test]
    fn perspective_maps_all_four_corners_exactly() {
        for settings in [
            ArtworkSettings {
                size_percent: 100.0,
                ..ArtworkSettings::default()
            },
            ArtworkSettings {
                enabled: true,
                size_percent: 100.0,
                corners: [[0.1, 0.1], [0.8, 0.2], [0.9, 0.9], [0.2, 0.7]],
                ..ArtworkSettings::default()
            },
        ] {
            let h = settings.transform().unwrap();
            for (p, expected) in
                settings
                    .corners
                    .iter()
                    .zip([[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]])
            {
                let x = p[0] as f32;
                let y = p[1] as f32;
                let w = h[2] * x + h[5] * y + h[8];
                assert!(((h[0] * x + h[3] * y + h[6]) / w - expected[0]).abs() < 0.0001);
                assert!(((h[1] * x + h[4] * y + h[7]) / w - expected[1]).abs() < 0.0001);
            }
        }
    }
    #[test]
    fn folded_tiny_and_nonfinite_placements_are_rejected() {
        let mut s = ArtworkSettings::default();
        s.corners.swap(1, 2);
        assert!(s.transform().is_err());
        s.corners = [[0.5, 0.5]; 4];
        assert!(s.transform().is_err());
        s = ArtworkSettings::default();
        s.corners[0][0] = f64::NAN;
        assert!(s.transform().is_err());
    }
    #[test]
    fn diameter_scales_the_whole_cover_without_changing_center_or_fit() {
        fn map(h: [f32; 9], p: [f32; 2]) -> [f32; 2] {
            let w = h[2] * p[0] + h[5] * p[1] + h[8];
            [
                (h[0] * p[0] + h[3] * p[1] + h[6]) / w,
                (h[1] * p[0] + h[4] * p[1] + h[7]) / w,
            ]
        }
        let mut settings = ArtworkSettings {
            corners: [[0.1, 0.2], [0.9, 0.2], [0.9, 0.8], [0.1, 0.8]],
            ..ArtworkSettings::default()
        };
        let original = settings.corners;
        for percent in [5.0, 30.0, 50.0, 100.0] {
            settings.size_percent = percent;
            let h = settings.transform().unwrap();
            let center = map(h, [0.5, 0.5]);
            assert!((center[0] - 0.5).abs() < 0.00001 && (center[1] - 0.5).abs() < 0.00001);
            let right = map(h, [0.5 + 0.4 * (percent as f32 / 100.0), 0.5]);
            assert!((right[0] - 1.0).abs() < 0.00001 && (right[1] - 0.5).abs() < 0.00001);
            assert_eq!(settings.corners, original);
        }
        for percent in [0.0, 4.9, 100.1, f64::NAN, f64::INFINITY] {
            settings.size_percent = percent;
            assert!(settings.transform().is_err());
        }
        let old: ArtworkSettings = toml::from_str("enabled = true").unwrap();
        assert_eq!(
            old.size_percent, 36.0,
            "configs without a cover size use the new center-label default"
        );
        settings = ArtworkSettings::default();
        settings.size_percent = 30.0;
        let saved = toml::to_string(&settings).unwrap();
        assert_eq!(toml::from_str::<ArtworkSettings>(&saved).unwrap(), settings);
        let legacy: ArtworkSettings = toml::from_str("diameter_percent = 40.0").unwrap();
        assert_eq!(legacy.size_percent, 40.0);
    }
}
