use anyhow::Result;

pub struct EventRoute {
    pub pattern: EventPattern,
    pub handler: EventHandler,
}

pub enum EventPattern {
    Exact(String),
    Prefix(String),
}

impl EventPattern {
    pub fn matches(&self, event_id: &str) -> bool {
        match self {
            EventPattern::Exact(s) => s == event_id,
            EventPattern::Prefix(p) => event_id.starts_with(p),
        }
    }
}

#[allow(dead_code)]
pub enum EventHandler {
    Sync(Box<dyn Fn(&str) -> Result<HandlerResult> + Send + Sync>),
    Async(Box<dyn Fn(&str) -> Result<()> + Send + Sync>),
}

pub enum HandlerResult {
    Continue,
    Quit,
}

pub struct EventRouter {
    routes: Vec<EventRoute>,
}

impl EventRouter {
    pub fn new(routes: Vec<EventRoute>) -> Self {
        Self { routes }
    }

    pub fn route(&self, event_id: &str) -> Result<HandlerResult> {
        for route in &self.routes {
            if route.pattern.matches(event_id) {
                return match &route.handler {
                    EventHandler::Sync(f) => f(event_id),
                    EventHandler::Async(f) => {
                        f(event_id)?;
                        Ok(HandlerResult::Continue)
                    }
                };
            }
        }

        log::warn!("No route found for event: {}", event_id);
        Ok(HandlerResult::Continue)
    }
}
