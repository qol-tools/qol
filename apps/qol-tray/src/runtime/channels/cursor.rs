use std::sync::Arc;
use std::time::Duration;

use super::super::channel::Channel;
use crate::desktop_state::Platform;

const MIN_INTERVAL: Duration = Duration::from_millis(16);

pub(crate) struct CursorChannel {
    platform: Arc<dyn Platform>,
    last_pos: Option<(f32, f32)>,
    current_pos: Option<(f32, f32)>,
}

impl CursorChannel {
    pub(crate) fn new(platform: Arc<dyn Platform>) -> Self {
        Self {
            platform,
            last_pos: None,
            current_pos: None,
        }
    }

    pub(crate) fn position(&self) -> Option<(f32, f32)> {
        self.current_pos
    }
}

impl Channel for CursorChannel {
    fn poll(&mut self) -> bool {
        let pos = self.platform.cursor_position();
        self.current_pos = pos;

        let changed = match (pos, self.last_pos) {
            (Some((x, y)), Some((lx, ly))) => (x - lx).abs() > 1.0 || (y - ly).abs() > 1.0,
            (Some(_), None) => true,
            _ => false,
        };
        self.last_pos = pos;
        changed
    }

    fn min_interval(&self) -> Duration {
        MIN_INTERVAL
    }
}
