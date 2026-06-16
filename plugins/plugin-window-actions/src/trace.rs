pub struct ActionTimer {
    #[cfg(debug_assertions)]
    action: String,
    #[cfg(debug_assertions)]
    start: std::time::Instant,
}

impl ActionTimer {
    pub fn start(action: &str) -> Self {
        #[cfg(not(debug_assertions))]
        let _ = action;
        Self {
            #[cfg(debug_assertions)]
            action: action.to_string(),
            #[cfg(debug_assertions)]
            start: std::time::Instant::now(),
        }
    }

    pub fn finish(self, outcome: &Result<(), String>) {
        #[cfg(debug_assertions)]
        {
            let (status, detail) = match outcome {
                Ok(()) => ("ok", String::new()),
                Err(error) => ("err", format!(" err={error:?}")),
            };
            qol_runtime::probe!(
                "WINACT_DONE",
                "action={} total_ms={} outcome={status}{detail}",
                self.action,
                self.start.elapsed().as_millis()
            );
        }
        #[cfg(not(debug_assertions))]
        let _ = outcome;
    }
}
