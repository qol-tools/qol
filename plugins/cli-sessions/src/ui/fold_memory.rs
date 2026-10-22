use gpui::{point, px, size, Bounds, Pixels};

use crate::storage::paths;
use crate::storage::persist::{self, UnfoldedPanel};

fn record_of(bounds: Bounds<Pixels>) -> UnfoldedPanel {
    UnfoldedPanel {
        x: bounds.origin.x.to_f64(),
        y: bounds.origin.y.to_f64(),
        width: bounds.size.width.to_f64(),
        height: bounds.size.height.to_f64(),
    }
}

fn bounds_of(record: UnfoldedPanel) -> Option<Bounds<Pixels>> {
    let drawable = record.width > 0.0 && record.height > 0.0;
    let finite = [record.x, record.y, record.width, record.height]
        .iter()
        .all(|value| value.is_finite());
    (drawable && finite).then(|| {
        Bounds::new(
            point(px(record.x as f32), px(record.y as f32)),
            size(px(record.width as f32), px(record.height as f32)),
        )
    })
}

pub fn save(unfolded: Bounds<Pixels>) {
    if let Some(path) = paths::unfolded_panel_path() {
        persist::save_unfolded_panel(&path, record_of(unfolded));
    }
}

pub fn load() -> Option<Bounds<Pixels>> {
    bounds_of(persist::load_unfolded_panel(&paths::unfolded_panel_path()?)?)
}

pub fn clear() {
    if let Some(path) = paths::unfolded_panel_path() {
        persist::clear_unfolded_panel(&path);
    }
}

#[cfg(test)]
mod tests {
    use super::{bounds_of, record_of};
    use crate::storage::persist::{self, UnfoldedPanel};
    use gpui::{point, px, size, Bounds};

    #[test]
    fn unfolded_bounds_round_trip_through_the_record_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("panel-unfolded.json");
        let bounds = Bounds::new(point(px(-1200.0), px(36.5)), size(px(420.0), px(712.0)));
        persist::save_unfolded_panel(&path, record_of(bounds));
        let read = persist::load_unfolded_panel(&path).unwrap();
        assert_eq!(bounds_of(read), Some(bounds));
        persist::clear_unfolded_panel(&path);
        assert_eq!(persist::load_unfolded_panel(&path), None);
    }

    fn record(x: f64, y: f64, width: f64, height: f64) -> UnfoldedPanel {
        UnfoldedPanel {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn an_undrawable_or_corrupt_record_restores_nothing() {
        for stored in [
            record(0.0, 0.0, 0.0, 400.0),
            record(0.0, 0.0, 360.0, -1.0),
            record(f64::NAN, 0.0, 360.0, 400.0),
            record(0.0, f64::INFINITY, 360.0, 400.0),
        ] {
            assert_eq!(bounds_of(stored), None, "{stored:?}");
        }
    }
}
