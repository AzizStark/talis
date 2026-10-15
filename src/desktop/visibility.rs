//! Read-only visibility evidence. Foreign-window queries never run on GTK's thread.
mod xcb;
pub(crate) use super::geometry::Rect;

use std::{
    sync::mpsc::{self, Receiver, SyncSender, TrySendError},
    time::Instant,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Targets {
    native: NativeTargets,
    /// Expected native animation bounds, physical top-left scene coordinates.
    /// None retains conservative whole-scene coverage (including fallback).
    pub animation: Option<Rect>,
    pub scene_size: [i32; 2],
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NativeTargets {
    X11 { scene: u32, patch: Option<u32> },
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Layout {
    pub animation: Option<Rect>,
    pub scene_size: [i32; 2],
    pub generation: u64,
}
impl Targets {
    pub(super) fn x11(scene: u32, patch: Option<u32>, layout: Layout) -> Self {
        Self {
            native: NativeTargets::X11 { scene, patch },
            animation: patch.and(layout.animation),
            scene_size: layout.scene_size,
            generation: layout.generation,
        }
    }
    #[cfg(test)]
    pub(crate) fn fixture(layout: Layout) -> Self {
        Self::x11(1, Some(2), layout)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hidden {
    ScreenOff,
    Covered,
    AnimationCovered,
}
impl Hidden {
    pub(crate) fn coverage(self) -> bool {
        matches!(self, Self::Covered | Self::AnimationCovered)
    }
}

pub(crate) struct Sample {
    pub targets: Targets,
    pub at: Instant,
    pub hidden: Option<Hidden>,
    /// Reuse the scene translation already read for coverage, without a second
    /// polling source or per-animation-frame native query.
    pub scene_origin: Option<[i32; 2]>,
}

pub(crate) struct Probe {
    requests: SyncSender<Targets>,
    replies: Receiver<Sample>,
}

impl Probe {
    pub(super) fn start_x11() -> std::io::Result<Self> {
        let (requests, jobs) = mpsc::sync_channel::<Targets>(1);
        let (results, replies) = mpsc::channel();
        std::thread::Builder::new()
            .name("talis-visibility".into())
            .spawn(move || {
                let mut native = xcb::Observer::new()
                    .map_err(|error| {
                        tracing::warn!(%error, "Native visibility unavailable; keep rendering");
                    })
                    .ok();
                while let Ok(targets) = jobs.recv() {
                    let at = Instant::now();
                    let hidden = native.as_mut().and_then(|observer| {
                        observer.sample(&targets).unwrap_or_else(|error| {
                            // BadWindow is ordinary: clients close during queries.
                            tracing::debug!(%error, "Visibility uncertain; keep rendering");
                            None
                        })
                    });
                    let scene_origin = native.as_ref().and_then(|o| o.scene_origin);
                    if results
                        .send(Sample {
                            targets,
                            at,
                            hidden,
                            scene_origin,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                // Owned connections and libraries drop on their worker thread.
            })?;
        Ok(Self { requests, replies })
    }
    pub(crate) fn request(&self, targets: Targets) -> bool {
        match self.requests.try_send(targets) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => false,
        }
    }
    pub(crate) fn latest(&self) -> Option<Sample> {
        self.replies.try_iter().last()
    }
}

/// Exact union coverage, not bounding-box coverage (which hides holes).
fn covered(target: Rect, rects: &[Rect]) -> bool {
    if target.w <= 0 || target.h <= 0 || rects.len() > 256 {
        return false;
    }
    let clips: Vec<_> = rects
        .iter()
        .filter_map(|r| r.intersection(target))
        .collect();
    let mut ys = vec![target.y, target.y + target.h];
    for r in &clips {
        ys.extend([r.y, r.y + r.h]);
    }
    ys.sort_unstable();
    ys.dedup();
    ys.windows(2).all(|strip| {
        let mut spans: Vec<_> = clips
            .iter()
            .filter(|r| r.y <= strip[0] && r.y + r.h >= strip[1])
            .map(|r| (r.x, r.x + r.w))
            .collect();
        spans.sort_unstable();
        let mut end = target.x;
        for (start, right) in spans {
            if start > end {
                return false;
            }
            end = end.max(right);
        }
        end >= target.x + target.w
    })
}

fn coverage_state(scene: Rect, animation: Option<Rect>, rects: &[Rect]) -> Option<Hidden> {
    if covered(scene, rects) {
        Some(Hidden::Covered)
    } else if animation.is_some_and(|patch| covered(patch, rects)) {
        Some(Hidden::AnimationCovered)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn target_identity_and_patch_presence_invalidate_coverage() {
        let layout = Layout {
            animation: Some(Rect {
                x: 10,
                y: 20,
                w: 30,
                h: 40,
            }),
            scene_size: [100, 100],
            generation: 7,
        };
        let current = Targets::x11(1, Some(2), layout);
        assert_ne!(current, Targets::x11(3, Some(2), layout));
        assert_ne!(current, Targets::x11(1, Some(4), layout));
        let fallback = Targets::x11(1, None, layout);
        assert_ne!(current, fallback);
        assert_eq!(fallback.animation, None);
    }
    #[test]
    fn maximized_coverage_can_hide_animation_without_hiding_static_borders() {
        let scene = Rect {
            x: 0,
            y: 0,
            w: 2560,
            h: 1440,
        };
        let animation = Rect {
            x: 894,
            y: 582,
            w: 868,
            h: 636,
        };
        let maximized = Rect { h: 1402, ..scene };
        assert_eq!(
            coverage_state(scene, Some(animation), &[maximized]),
            Some(Hidden::AnimationCovered)
        );
        assert_eq!(coverage_state(scene, None, &[maximized]), None);
        assert_eq!(
            coverage_state(scene, Some(animation), &[scene]),
            Some(Hidden::Covered)
        );
        let left = Rect {
            w: 434,
            ..animation
        };
        let right = Rect {
            x: 1328,
            w: 434,
            ..animation
        };
        assert_eq!(coverage_state(scene, Some(animation), &[left]), None);
        assert_eq!(
            coverage_state(scene, Some(animation), &[left, right]),
            Some(Hidden::AnimationCovered)
        );
        assert_eq!(
            coverage_state(
                scene,
                Some(animation),
                &[
                    left,
                    Rect {
                        x: 1329,
                        w: 433,
                        ..right
                    }
                ]
            ),
            None
        );
    }
    #[test]
    fn coverage_requires_every_pixel_not_union_bounds() {
        let screen = Rect {
            x: -100,
            y: 40,
            w: 200,
            h: 100,
        };
        let left = Rect { w: 100, ..screen };
        let right = Rect {
            x: 0,
            w: 100,
            ..screen
        };
        assert!(covered(screen, &[left, right]));
        assert!(!covered(
            screen,
            &[
                left,
                Rect {
                    x: 1,
                    w: 99,
                    ..right
                }
            ]
        ));
        assert!(!covered(screen, &[Rect { h: 99, ..screen }]));
        assert!(covered(
            screen,
            &[Rect {
                x: -200,
                y: 0,
                w: 500,
                h: 300
            }]
        ));
        assert!(!covered(screen, &[]));
    }
    #[test]
    fn stacked_strips_and_overlaps_cover_without_overcounting() {
        let target = Rect {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
        };
        assert!(covered(
            target,
            &[
                Rect { h: 5, ..target },
                Rect {
                    y: 5,
                    h: 5,
                    ..target
                }
            ]
        ));
        assert!(!covered(target, &[Rect { w: 6, ..target }; 10]));
        assert!(!covered(Rect { w: 0, ..target }, &[target]));
    }
}
