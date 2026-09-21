use std::time::Duration;

use super::super::Channel;
use crate::desktop_state::SharedPlatform;

const MIN_INTERVAL: Duration = Duration::from_millis(100);

pub(crate) struct WindowListChannel {
    platform: SharedPlatform,
    fingerprint: Option<u64>,
}

impl WindowListChannel {
    pub(crate) fn new(platform: SharedPlatform) -> Self {
        Self {
            platform,
            fingerprint: None,
        }
    }
}

impl Channel for WindowListChannel {
    fn poll(&mut self) -> bool {
        let fresh = self.platform.window_list_fingerprint();
        if fresh.is_none() {
            return false;
        }
        if fresh == self.fingerprint {
            return false;
        }
        self.fingerprint = fresh;
        true
    }

    fn min_interval(&self) -> Duration {
        MIN_INTERVAL
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop_state::Platform;
    use qol_runtime::MonitorBounds;
    use std::sync::{Arc, Mutex};

    struct ScriptedPlatform {
        fingerprints: Mutex<Vec<Option<u64>>>,
    }

    impl ScriptedPlatform {
        fn new(fingerprints: Vec<Option<u64>>) -> Arc<Self> {
            Arc::new(Self {
                fingerprints: Mutex::new(fingerprints),
            })
        }
    }

    impl Platform for ScriptedPlatform {
        fn cursor_position(&self) -> Option<(f32, f32)> {
            None
        }
        fn focused_window_bounds(&self) -> Option<MonitorBounds> {
            None
        }
        fn physical_monitors(&self) -> Vec<MonitorBounds> {
            Vec::new()
        }
        fn window_list_fingerprint(&self) -> Option<u64> {
            let mut q = self.fingerprints.lock().unwrap();
            if q.is_empty() {
                return None;
            }
            q.remove(0)
        }
    }

    fn run(seq: Vec<Option<u64>>) -> Vec<bool> {
        let n = seq.len();
        let mut ch = WindowListChannel::new(ScriptedPlatform::new(seq));
        (0..n).map(|_| ch.poll()).collect()
    }

    #[test]
    fn first_change_reported_then_stable_is_noop() {
        let changed = run(vec![Some(1), Some(1), Some(1)]);
        assert_eq!(changed, vec![true, false, false]);
    }

    #[test]
    fn distinct_fingerprints_each_change() {
        let changed = run(vec![Some(1), Some(2), Some(3)]);
        assert_eq!(changed, vec![true, true, true]);
    }

    #[test]
    fn fingerprint_unavailable_never_signals_change() {
        let changed = run(vec![None, None, None]);
        assert_eq!(changed, vec![false, false, false]);
    }

    #[test]
    fn unavailable_then_available_still_signals_on_first_value() {
        let changed = run(vec![None, Some(7), Some(7)]);
        assert_eq!(changed, vec![false, true, false]);
    }
}
