use gpui::{point, px, size, Bounds, Pixels};
use objc2::runtime::NSObjectProtocol;
use objc2::sel;
use objc2_app_kit::NSScreen;
use objc2_foundation::{MainThreadMarker, NSRect};

use crate::dead_zone::{DeadZone, DeadZoneReport, Edge};

const MATCH_TOLERANCE: f64 = 1.0;

pub(crate) fn dead_zones(monitor: Bounds<Pixels>) -> DeadZoneReport {
    let Some(mtm) = MainThreadMarker::new() else {
        return DeadZoneReport::Unknown("NSScreen is readable only on the main thread");
    };
    let screens = NSScreen::screens(mtm);
    let Some(primary_height) = screens
        .iter()
        .next()
        .map(|screen| screen.frame().size.height)
    else {
        return DeadZoneReport::Unknown("AppKit lists no screens");
    };
    let Some(screen) = screens
        .iter()
        .find(|screen| frame_matches(screen.frame(), primary_height, monitor))
    else {
        return DeadZoneReport::Unknown("no NSScreen has this monitor's bounds");
    };
    if !screen.respondsToSelector(sel!(safeAreaInsets)) {
        return DeadZoneReport::Reported(Vec::new());
    }
    let top_inset = screen.safeAreaInsets().top;
    DeadZoneReport::Reported(top_band(
        monitor,
        top_inset,
        screen.frame(),
        screen.auxiliaryTopLeftArea(),
        screen.auxiliaryTopRightArea(),
    ))
}

fn frame_matches(frame: NSRect, primary_height: f64, monitor: Bounds<Pixels>) -> bool {
    let top = primary_height - frame.origin.y - frame.size.height;
    close(frame.origin.x, monitor.origin.x.to_f64())
        && close(top, monitor.origin.y.to_f64())
        && close(frame.size.width, monitor.size.width.to_f64())
        && close(frame.size.height, monitor.size.height.to_f64())
}

fn close(left: f64, right: f64) -> bool {
    (left - right).abs() <= MATCH_TOLERANCE
}

fn top_band(
    monitor: Bounds<Pixels>,
    top_inset: f64,
    frame: NSRect,
    top_left: NSRect,
    top_right: NSRect,
) -> Vec<DeadZone> {
    if top_inset <= 0.0 {
        return Vec::new();
    }
    let has_sides = top_left.size.width > 0.0 && top_right.size.width > 0.0;
    let (start, width) = if has_sides {
        (
            top_left.size.width,
            frame.size.width - top_left.size.width - top_right.size.width,
        )
    } else {
        (0.0, frame.size.width)
    };
    if width <= 0.0 {
        return Vec::new();
    }
    vec![DeadZone {
        edge: Edge::Top,
        bounds: Bounds::new(
            point(monitor.origin.x + px(start as f32), monitor.origin.y),
            size(px(width as f32), px(top_inset as f32)),
        ),
    }]
}

#[cfg(test)]
mod tests {
    use gpui::{point, px, size, Bounds};
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    use super::{frame_matches, top_band};
    use crate::dead_zone::{DeadZone, Edge};

    fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
        NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
    }

    #[test]
    fn a_notched_display_reports_the_camera_housing_between_its_side_areas() {
        let monitor = Bounds::new(point(px(0.0), px(0.0)), size(px(1512.0), px(982.0)));
        let zones = top_band(
            monitor,
            32.0,
            rect(0.0, 0.0, 1512.0, 982.0),
            rect(0.0, 950.0, 663.5, 32.0),
            rect(848.5, 950.0, 663.5, 32.0),
        );
        assert_eq!(
            zones,
            vec![DeadZone {
                edge: Edge::Top,
                bounds: Bounds::new(point(px(663.5), px(0.0)), size(px(185.0), px(32.0))),
            }]
        );
    }

    #[test]
    fn a_notched_secondary_display_offsets_the_zone_by_its_monitor_origin() {
        let monitor = Bounds::new(point(px(-1512.0), px(200.0)), size(px(1512.0), px(982.0)));
        let zones = top_band(
            monitor,
            32.0,
            rect(-1512.0, -200.0, 1512.0, 982.0),
            rect(-1512.0, 850.0, 663.5, 32.0),
            rect(-663.5, 850.0, 663.5, 32.0),
        );
        assert_eq!(zones[0].bounds.origin, point(px(-848.5), px(200.0)));
    }

    #[test]
    fn a_display_without_a_top_inset_reports_nothing() {
        let monitor = Bounds::new(point(px(0.0), px(0.0)), size(px(2560.0), px(1440.0)));
        let empty = rect(0.0, 0.0, 0.0, 0.0);
        assert!(top_band(monitor, 0.0, rect(0.0, 0.0, 2560.0, 1440.0), empty, empty).is_empty());
    }

    #[test]
    fn a_top_inset_without_side_areas_blocks_the_whole_top_band() {
        let monitor = Bounds::new(point(px(0.0), px(0.0)), size(px(1920.0), px(1080.0)));
        let empty = rect(0.0, 0.0, 0.0, 0.0);
        let zones = top_band(monitor, 24.0, rect(0.0, 0.0, 1920.0, 1080.0), empty, empty);
        assert_eq!(
            zones[0].bounds,
            Bounds::new(point(px(0.0), px(0.0)), size(px(1920.0), px(24.0)))
        );
    }

    #[test]
    fn screens_match_monitors_in_top_left_coordinates() {
        let secondary = Bounds::new(point(px(1512.0), px(-300.0)), size(px(2560.0), px(1440.0)));
        assert!(frame_matches(
            rect(1512.0, -158.0, 2560.0, 1440.0),
            982.0,
            secondary
        ));
        assert!(!frame_matches(
            rect(1512.0, 0.0, 2560.0, 1440.0),
            982.0,
            secondary
        ));
    }
}
