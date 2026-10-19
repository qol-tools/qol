use std::time::Instant;

use qol_connection_health::Outcome;

use super::Shared;

const TARGET: &str = "https://api.github.com/rate_limit";

pub(super) struct Running {
    shared: &'static Shared,
    outcome: Option<Outcome>,
}

pub(super) fn start(shared: &'static Shared, now: Instant) -> Running {
    shared.probe_started(now);
    Running {
        shared,
        outcome: None,
    }
}

pub(super) async fn run(mut running: Running) {
    let request = crate::features::plugin_store::github::build_github_request(
        crate::updates::http_client(),
        TARGET,
        None,
    );
    let result = super::observe(running.shared, request).await;
    running.outcome = super::outcome(&result);
}

impl Drop for Running {
    fn drop(&mut self) {
        self.shared.probe_finished(self.outcome);
    }
}
