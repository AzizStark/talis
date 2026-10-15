//! Pixel-aligned presentation bounds; independent of cache/motion cadence.
pub(super) fn region(
    crops: &[[f32; 4]],
    viewport: (i32, i32, i32, i32),
    dimensions: (i32, i32),
    scale: i32,
) -> Option<[i32; 4]> {
    let (width, height) = dimensions;
    if crops.is_empty()
        || width <= 0
        || height <= 0
        || scale <= 0
        || width % scale != 0
        || height % scale != 0
        || scale > i32::MAX / 2
    {
        return None;
    }
    let mut left = width;
    let mut bottom = height;
    let mut right = 0;
    let mut top = 0;
    for &crop in crops {
        if crop.iter().any(|v| !v.is_finite()) || crop[2] <= 0.0 || crop[3] <= 0.0 {
            return None;
        }
        let [x, y, w, h] = super::cache::region(crop, viewport);
        left = left.min(x.clamp(0, width));
        bottom = bottom.min(y.clamp(0, height));
        right = right.max((x + w).clamp(0, width));
        top = top.max((y + h).clamp(0, height));
    }
    // Implicit texture derivatives use 2×2 physical-pixel quads. Preserve
    // their global phase (not just logical-pixel alignment) or perspective
    // mip/anisotropic sampling can change when the output origin is cropped.
    let alignment = if scale % 2 == 0 { scale } else { scale * 2 };
    left = left / alignment * alignment;
    bottom = bottom / alignment * alignment;
    right = ((right + alignment - 1) / alignment * alignment).min(width);
    top = ((top + alignment - 1) / alignment * alignment).min(height);
    (right > left && top > bottom).then_some([left, bottom, right - left, top - bottom])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn union_alignment_and_overscan_are_conservative() {
        let crops = [[
            900.0 / 2560.0,
            585.0 / 1440.0,
            860.0 / 2560.0,
            630.0 / 1440.0,
        ]];
        for (w, h, s) in [
            (2560, 1440, 1),
            (3840, 2160, 2),
            (2560, 1080, 1),
            (2880, 1620, 3),
        ] {
            let r = region(&crops, (0, -180, w, 1440), (w, h), s).unwrap();
            assert!(r[0] >= 0 && r[1] >= 0 && r[0] + r[2] <= w && r[1] + r[3] <= h);
            assert!(r.iter().all(|x| x % s == 0));
            assert_eq!(r[0] % 2, 0);
            assert_eq!(r[1] % 2, 0);
        }
        assert!(region(&[], (0, 0, 1, 1), (1, 1), 1).is_none());
        assert!(region(&[[f32::NAN, 0.0, 1.0, 1.0]], (0, 0, 1, 1), (1, 1), 1).is_none());
        assert!(region(&crops, (0, 0, 2560, 1440), (2560, 1440), 0).is_none());
        assert!(region(&crops, (0, 0, 2560, 1440), (2560, 1440), 3).is_none());
        let r = region(&[[0.0, 0.0, 1.0, 1.0]], (0, 0, 2559, 1439), (2559, 1439), 1).unwrap();
        assert_eq!(r, [0, 0, 2559, 1439]);
    }
}
