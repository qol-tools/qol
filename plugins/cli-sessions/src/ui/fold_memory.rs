use std::path::PathBuf;

use gpui::{point, px, size, Bounds, Pixels};
use serde::{Deserialize, Serialize};

use crate::storage::paths;

const FILE_NAME: &str = "panel-unfolded.json";

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct UnfoldedPanel {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl UnfoldedPanel {
    fn of(bounds: Bounds<Pixels>) -> Self {
        Self {
            x: bounds.origin.x.to_f64(),
            y: bounds.origin.y.to_f64(),
            width: bounds.size.width.to_f64(),
            height: bounds.size.height.to_f64(),
        }
    }

    fn bounds(self) -> Option<Bounds<Pixels>> {
        let drawable = self.width > 0.0 && self.height > 0.0;
        let finite = [self.x, self.y, self.width, self.height]
            .iter()
            .all(|value| value.is_finite());
        (drawable && finite).then(|| {
            Bounds::new(
                point(px(self.x as f32), px(self.y as f32)),
                size(px(self.width as f32), px(self.height as f32)),
            )
        })
    }
}

fn path() -> Option<PathBuf> {
    Some(paths::state_path()?.with_file_name(FILE_NAME))
}

pub fn save(unfolded: Bounds<Pixels>) {
    let Some(path) = path() else {
        return;
    };
    let json = match serde_json::to_vec(&UnfoldedPanel::of(unfolded)) {
        Ok(json) => json,
        Err(error) => {
            log::warn!("[cli-sessions] unfolded panel encode failed: {error}");
            return;
        }
    };
    if let Err(error) = qol_fs::atomic_write(&path, &json) {
        log::warn!("[cli-sessions] unfolded panel write failed: {error}");
    }
}

pub fn load() -> Option<Bounds<Pixels>> {
    let bytes = std::fs::read(path()?).ok()?;
    serde_json::from_slice::<UnfoldedPanel>(&bytes)
        .ok()?
        .bounds()
}

pub fn clear() {
    let Some(path) = path() else {
        return;
    };
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => log::warn!("[cli-sessions] unfolded panel clear failed: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::UnfoldedPanel;
    use gpui::{point, px, size, Bounds};

    #[test]
    fn unfolded_bounds_round_trip_through_the_record() {
        let bounds = Bounds::new(point(px(-1200.0), px(36.5)), size(px(420.0), px(712.0)));
        let record = UnfoldedPanel::of(bounds);
        let json = serde_json::to_string(&record).unwrap();
        let read: UnfoldedPanel = serde_json::from_str(&json).unwrap();
        assert_eq!(read.bounds(), Some(bounds));
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
            assert_eq!(stored.bounds(), None, "{stored:?}");
        }
    }
}
