mod platform;

use gpui::{point, px, size, Bounds, Pixels};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeadZone {
    pub edge: Edge,
    pub bounds: Bounds<Pixels>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DeadZoneReport {
    Reported(Vec<DeadZone>),
    Unknown(&'static str),
}

impl DeadZoneReport {
    pub fn zones(&self) -> &[DeadZone] {
        match self {
            Self::Reported(zones) => zones,
            Self::Unknown(_) => &[],
        }
    }
}

pub trait DeadZoneProvider {
    fn dead_zones(&self, monitor: Bounds<Pixels>) -> DeadZoneReport;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NativeDeadZones;

impl DeadZoneProvider for NativeDeadZones {
    fn dead_zones(&self, monitor: Bounds<Pixels>) -> DeadZoneReport {
        platform::dead_zones(monitor)
    }
}

pub fn native_dead_zones(monitor: Bounds<Pixels>) -> DeadZoneReport {
    let report = NativeDeadZones.dead_zones(monitor);
    if let DeadZoneReport::Unknown(reason) = &report {
        qol_runtime::probe!(
            "DEAD_ZONE",
            "monitor={:?} result=unknown reason={reason}",
            monitor
        );
    }
    report
}

pub fn clear_area(
    monitor: Bounds<Pixels>,
    placed: Bounds<Pixels>,
    zones: &[DeadZone],
) -> Bounds<Pixels> {
    let mut left = monitor.origin.x;
    let mut top = monitor.origin.y;
    let mut right = monitor.origin.x + monitor.size.width;
    let mut bottom = monitor.origin.y + monitor.size.height;
    for zone in zones.iter().filter(|zone| overlaps(zone, placed)) {
        let bounds = zone.bounds;
        match zone.edge {
            Edge::Top => top = top.max(bounds.origin.y + bounds.size.height),
            Edge::Bottom => bottom = bottom.min(bounds.origin.y),
            Edge::Left => left = left.max(bounds.origin.x + bounds.size.width),
            Edge::Right => right = right.min(bounds.origin.x),
        }
    }
    Bounds::new(
        point(left, top),
        size((right - left).max(px(0.0)), (bottom - top).max(px(0.0))),
    )
}

fn overlaps(zone: &DeadZone, placed: Bounds<Pixels>) -> bool {
    let zone = zone.bounds;
    let across = spans_overlap(
        zone.origin.x,
        zone.size.width,
        placed.origin.x,
        placed.size.width,
    );
    let down = spans_overlap(
        zone.origin.y,
        zone.size.height,
        placed.origin.y,
        placed.size.height,
    );
    across && down
}

fn spans_overlap(a: Pixels, a_len: Pixels, b: Pixels, b_len: Pixels) -> bool {
    a < b + b_len && b < a + a_len
}

#[cfg(test)]
mod tests {
    use gpui::{point, px, size, Bounds};

    use super::{clear_area, DeadZone, DeadZoneReport, Edge};

    fn notch() -> DeadZone {
        DeadZone {
            edge: Edge::Top,
            bounds: Bounds::new(point(px(663.5), px(0.0)), size(px(185.0), px(32.0))),
        }
    }

    fn laptop() -> Bounds<gpui::Pixels> {
        Bounds::new(point(px(0.0), px(0.0)), size(px(1512.0), px(982.0)))
    }

    #[test]
    fn a_placement_under_the_notch_loses_the_notch_band() {
        let under = Bounds::new(point(px(646.0), px(12.0)), size(px(220.0), px(28.0)));
        assert_eq!(
            clear_area(laptop(), under, &[notch()]),
            Bounds::new(point(px(0.0), px(32.0)), size(px(1512.0), px(950.0)))
        );
    }

    #[test]
    fn a_placement_beside_or_below_the_notch_keeps_the_whole_monitor() {
        let cases = [
            Bounds::new(point(px(24.0), px(4.0)), size(px(200.0), px(20.0))),
            Bounds::new(point(px(646.0), px(48.0)), size(px(220.0), px(84.0))),
        ];
        for placed in cases {
            assert_eq!(
                clear_area(laptop(), placed, &[notch()]),
                laptop(),
                "{placed:?}"
            );
        }
    }

    #[test]
    fn every_edge_trims_its_own_side() {
        let monitor = Bounds::new(point(px(100.0), px(50.0)), size(px(1000.0), px(800.0)));
        let everywhere = monitor;
        let zones = [
            DeadZone {
                edge: Edge::Bottom,
                bounds: Bounds::new(point(px(100.0), px(830.0)), size(px(1000.0), px(20.0))),
            },
            DeadZone {
                edge: Edge::Left,
                bounds: Bounds::new(point(px(100.0), px(50.0)), size(px(10.0), px(800.0))),
            },
            DeadZone {
                edge: Edge::Right,
                bounds: Bounds::new(point(px(1090.0), px(50.0)), size(px(10.0), px(800.0))),
            },
        ];
        assert_eq!(
            clear_area(monitor, everywhere, &zones),
            Bounds::new(point(px(110.0), px(50.0)), size(px(980.0), px(780.0)))
        );
    }

    #[test]
    fn an_unknown_report_has_no_zones() {
        assert!(DeadZoneReport::Unknown("test").zones().is_empty());
        assert_eq!(DeadZoneReport::Reported(vec![notch()]).zones(), &[notch()]);
    }
}
