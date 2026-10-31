use gpui::{Bounds, Pixels};

use crate::dead_zone::DeadZoneReport;

pub(crate) fn dead_zones(_monitor: Bounds<Pixels>) -> DeadZoneReport {
    DeadZoneReport::Reported(Vec::new())
}
