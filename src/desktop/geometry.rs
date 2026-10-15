//! Checked coordinate conversions shared by placement, visibility and input.
#[cfg(feature = "desktop")]
use anyhow::{Context, Result, ensure};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}
impl Rect {
    /// Convert bottom-up GL bounds to physical top-left scene coordinates.
    /// Signed origins are retained for viewports cropped by the scene fit.
    pub(crate) fn from_gl([x, y, w, h]: [i32; 4], height: i32) -> Option<Self> {
        if height <= 0 || w <= 0 || h <= 0 {
            return None;
        }
        Some(Self {
            x,
            y: height.checked_sub(y)?.checked_sub(h)?,
            w,
            h,
        })
    }
    #[cfg(feature = "desktop")]
    pub(crate) fn scaled(self, scale: i32) -> Result<Self> {
        ensure!(
            scale > 0 && self.w > 0 && self.h > 0,
            "Invalid desktop monitor geometry"
        );
        let multiply = |n: i32| {
            n.checked_mul(scale)
                .context("Desktop monitor geometry overflow")
        };
        Ok(Self {
            x: multiply(self.x)?,
            y: multiply(self.y)?,
            w: multiply(self.w)?,
            h: multiply(self.h)?,
        })
    }
    #[cfg(feature = "desktop")]
    pub(crate) fn intersection(self, other: Self) -> Option<Self> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self
            .x
            .checked_add(self.w)?
            .min(other.x.checked_add(other.w)?);
        let bottom = self
            .y
            .checked_add(self.h)?
            .min(other.y.checked_add(other.h)?);
        (right > x && bottom > y).then_some(Self {
            x,
            y,
            w: right.checked_sub(x)?,
            h: bottom.checked_sub(y)?,
        })
    }
}

/// Native patches need exact integer-scale alignment; rounding would create seams.
#[cfg(feature = "desktop")]
pub(crate) fn logical_patch(region: [i32; 4], height: i32, scale: i32) -> Option<[i32; 4]> {
    // Unlike a cropped viewport, an independently placed patch must stay
    // inside the scene vertically. Preserve the original native-patch guard.
    if region[1] < 0 {
        return None;
    }
    let Rect { x, y, w, h } = Rect::from_gl(region, height)?;
    if scale <= 0 || x < 0 || y < 0 || [x, y, w, h].iter().any(|n| n % scale != 0) {
        return None;
    }
    Some([x / scale, y / scale, w / scale, h / scale])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gl_coordinates_preserve_crop_and_reject_overflow() {
        assert_eq!(
            Rect::from_gl([0, 222, 868, 636], 1440),
            Some(Rect {
                x: 0,
                y: 582,
                w: 868,
                h: 636
            })
        );
        assert_eq!(
            Rect::from_gl([-20, -10, 100, 90], 80),
            Some(Rect {
                x: -20,
                y: 0,
                w: 100,
                h: 90
            })
        );
        assert_eq!(Rect::from_gl([0, i32::MIN, 1, 1], 1), None);
        assert_eq!(Rect::from_gl([0, 0, 0, 1], 1), None);
    }
    #[test]
    #[cfg(feature = "desktop")]
    fn scaling_preserves_negative_outputs_and_checks_overflow() {
        let rect = Rect {
            x: -1920,
            y: -100,
            w: 1920,
            h: 1080,
        };
        assert_eq!(
            rect.scaled(2).unwrap(),
            Rect {
                x: -3840,
                y: -200,
                w: 3840,
                h: 2160
            }
        );
        assert!(rect.scaled(0).is_err());
        assert!(
            Rect {
                x: i32::MAX,
                ..rect
            }
            .scaled(2)
            .is_err()
        );
    }
    #[test]
    #[cfg(feature = "desktop")]
    fn patch_coordinates_require_exact_alignment() {
        assert_eq!(
            logical_patch([894, 222, 868, 636], 1440, 1),
            Some([894, 582, 868, 636])
        );
        assert_eq!(
            logical_patch([894, 222, 868, 636], 1440, 2),
            Some([447, 291, 434, 318])
        );
        assert_eq!(logical_patch([893, 222, 868, 636], 1440, 2), None);
        assert_eq!(logical_patch([0, 900, 10, 600], 1440, 1), None);
        assert_eq!(logical_patch([0, 0, 10, 10], 1440, 0), None);
        assert_eq!(logical_patch([0, -2, 10, 10], 20, 1), None);
    }
}
