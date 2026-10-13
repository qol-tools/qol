use std::sync::Arc;

use qol_runtime::MonitorBounds;

mod ignore_pids;
pub(crate) mod platform;

pub(crate) type SharedPlatform = Arc<dyn Platform>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FocusedWindow {
    pub id: Option<u32>,
    pub monitor: MonitorBounds,
}

pub(crate) trait Platform: Send + Sync {
    fn cursor_position(&self) -> Option<(f32, f32)>;
    fn focused_window_bounds(&self) -> Option<MonitorBounds>;
    fn physical_monitors(&self) -> Vec<MonitorBounds>;

    fn focused_window(&self) -> Option<FocusedWindow> {
        Some(FocusedWindow {
            id: None,
            monitor: self.focused_window_bounds()?,
        })
    }

    fn poll_focused_window(&self) -> bool {
        true
    }

    fn window_list_fingerprint(&self) -> Option<u64> {
        None
    }

    fn window_open(&self, _id: u32) -> Option<bool> {
        None
    }
}

pub(crate) fn create_shared() -> SharedPlatform {
    Arc::new(platform::create())
}

pub(crate) fn fingerprint_windows(
    windows: impl IntoIterator<Item = u64>,
    focused: Option<u64>,
) -> u64 {
    let seed = focused.map_or(0, |id| spread(id).rotate_left(32));
    windows
        .into_iter()
        .fold(seed, |fingerprint, id| fingerprint ^ spread(id))
}

fn spread(id: u64) -> u64 {
    let mixed = id.wrapping_mul(0x9E3779B97F4A7C15);
    mixed ^ (mixed >> 32)
}

pub(crate) use ignore_pids::is_ignored_pid;
pub(crate) use ignore_pids::{add_ignore_pid, remove_ignore_pid};

#[cfg(test)]
mod tests {
    use super::fingerprint_windows;

    #[test]
    fn a_focus_change_alone_changes_the_fingerprint() {
        let windows = [11, 22, 33];
        assert_ne!(
            fingerprint_windows(windows, Some(11)),
            fingerprint_windows(windows, Some(22))
        );
    }

    #[test]
    fn focusing_a_listed_window_does_not_cancel_it_out() {
        assert_ne!(
            fingerprint_windows([11, 22], Some(11)),
            fingerprint_windows([22], None)
        );
    }

    #[test]
    fn stacking_order_does_not_change_the_fingerprint() {
        assert_eq!(
            fingerprint_windows([11, 22, 33], Some(22)),
            fingerprint_windows([33, 11, 22], Some(22))
        );
    }
}
