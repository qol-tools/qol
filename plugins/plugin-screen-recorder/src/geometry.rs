#[derive(Debug, Clone, Copy)]
pub struct Monitor {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

const SNAP_MARGIN_PX: i32 = 50;

pub(crate) fn prepare_recording_rect(
    mut rect: Rect,
    monitors: &[Monitor],
    fallback_bounds: Monitor,
) -> Rect {
    let bounds = monitor_for_selection(rect, monitors).unwrap_or(fallback_bounds);
    rect = clamp_to_bounds(rect, bounds);
    snap_to_bottom(rect, bounds.y + bounds.h)
}

pub(crate) fn even_dimensions(mut rect: Rect) -> Rect {
    if rect.w % 2 != 0 {
        rect.w -= 1;
    }
    if rect.h % 2 != 0 {
        rect.h -= 1;
    }
    rect
}

pub(crate) fn monitor_for_selection(rect: Rect, monitors: &[Monitor]) -> Option<Monitor> {
    let center_x = rect.x + rect.w / 2;
    let center_y = rect.y + rect.h / 2;
    monitors.iter().copied().find(|monitor| {
        center_x >= monitor.x
            && center_x < monitor.x + monitor.w
            && center_y >= monitor.y
            && center_y < monitor.y + monitor.h
    })
}

pub(crate) fn clamp_to_bounds(mut rect: Rect, bounds: Monitor) -> Rect {
    if rect.x < bounds.x {
        rect.w -= bounds.x - rect.x;
        rect.x = bounds.x;
    }
    if rect.y < bounds.y {
        rect.h -= bounds.y - rect.y;
        rect.y = bounds.y;
    }
    if rect.x + rect.w > bounds.x + bounds.w {
        rect.w = bounds.x + bounds.w - rect.x;
    }
    if rect.y + rect.h > bounds.y + bounds.h {
        rect.h = bounds.y + bounds.h - rect.y;
    }
    rect
}

fn snap_to_bottom(mut rect: Rect, bottom: i32) -> Rect {
    let gap = bottom - (rect.y + rect.h);
    if gap > 0 && gap <= SNAP_MARGIN_PX {
        rect.h = bottom - rect.y;
    }
    rect
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepare_clamps_to_selected_monitor() {
        let rect = Rect {
            x: 10,
            y: 10,
            w: 400,
            h: 300,
        };
        let monitors = [Monitor {
            x: 0,
            y: 0,
            w: 320,
            h: 240,
        }];
        let prepared = prepare_recording_rect(rect, &monitors, monitors[0]);
        assert_eq!(prepared.x, 10);
        assert_eq!(prepared.y, 10);
        assert_eq!(prepared.w, 310);
        assert_eq!(prepared.h, 230);
    }

    #[test]
    fn even_dimensions_drop_odd_pixel() {
        let rect = even_dimensions(Rect {
            x: 0,
            y: 0,
            w: 101,
            h: 99,
        });
        assert_eq!(rect.w, 100);
        assert_eq!(rect.h, 98);
    }
}
