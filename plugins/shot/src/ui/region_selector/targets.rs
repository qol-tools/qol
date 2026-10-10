use gpui::{Pixels, Point};

use super::{DetectedTarget, DetectedTargetRole, HoverTarget};
use crate::Rect;

impl DetectedTarget {
    pub(crate) fn window(rect: Rect) -> Self {
        Self {
            rect,
            role: DetectedTargetRole { is_window: true },
        }
    }

    pub(crate) fn monitor(rect: Rect) -> Self {
        Self {
            rect,
            role: DetectedTargetRole { is_window: false },
        }
    }
}

pub(crate) struct SnapshotTargets {
    pub(crate) windows: Vec<Rect>,
    pub(crate) monitors: Vec<Rect>,
}

impl HoverTarget for SnapshotTargets {
    fn target_at(&self, point: Point<Pixels>) -> Option<DetectedTarget> {
        let x = f32::from(point.x).round() as i32;
        let y = f32::from(point.y).round() as i32;
        let hit = |rect: &Rect| rect_contains(*rect, x, y);
        if let Some(window) = self.windows.iter().copied().find(hit) {
            return Some(DetectedTarget::window(window));
        }
        self.monitors
            .iter()
            .copied()
            .find(hit)
            .map(DetectedTarget::monitor)
    }
}

fn rect_contains(rect: Rect, x: i32, y: i32) -> bool {
    x >= rect.x && x < rect.x + rect.w && y >= rect.y && y < rect.y + rect.h
}

#[cfg(test)]
mod tests {
    use super::{DetectedTarget, HoverTarget, SnapshotTargets};
    use crate::Rect;
    use gpui::{point, px};

    #[test]
    fn hover_target_picks_topmost_window_then_the_monitor_under_the_pointer() {
        let top = Rect {
            x: 100,
            y: 100,
            w: 400,
            h: 300,
        };
        let bottom = Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        };
        let monitor = Rect {
            x: 1920,
            y: 0,
            w: 2560,
            h: 1440,
        };
        let source = SnapshotTargets {
            windows: vec![top, bottom],
            monitors: vec![monitor],
        };
        let cases = [
            (
                point(px(150.0), px(150.0)),
                Some(DetectedTarget::window(top)),
            ),
            (
                point(px(50.0), px(50.0)),
                Some(DetectedTarget::window(bottom)),
            ),
            (
                point(px(499.0), px(399.0)),
                Some(DetectedTarget::window(top)),
            ),
            (
                point(px(500.0), px(400.0)),
                Some(DetectedTarget::window(bottom)),
            ),
            (
                point(px(3000.0), px(50.0)),
                Some(DetectedTarget::monitor(monitor)),
            ),
            (point(px(5000.0), px(50.0)), None),
        ];
        for (pointer, expected) in cases {
            assert_eq!(source.target_at(pointer), expected, "pointer: {pointer:?}");
        }
    }

    #[test]
    fn detected_target_roles_follow_their_constructor() {
        let rect = Rect {
            x: 1,
            y: 2,
            w: 3,
            h: 4,
        };
        let cases = [
            (DetectedTarget::window(rect), true),
            (DetectedTarget::monitor(rect), false),
        ];
        for (target, is_window) in cases {
            assert_eq!(target.rect(), rect);
            assert_eq!(target.role.is_window, is_window);
        }
    }
}
