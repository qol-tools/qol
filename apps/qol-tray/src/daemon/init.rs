use super::EventBus;

#[derive(Clone)]
pub struct Daemon {
    pub events: std::sync::Arc<EventBus>,
}

impl Daemon {
    pub fn new() -> Self {
        Self {
            events: std::sync::Arc::new(EventBus::new()),
        }
    }
}

impl Default for Daemon {
    fn default() -> Self {
        Self::new()
    }
}
