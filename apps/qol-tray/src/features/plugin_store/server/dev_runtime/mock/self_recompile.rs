use std::sync::Arc;

use crate::daemon::{DaemonEvent, EventBus};
use crate::dev::adapters::{DevMockTarget, DevRuntimeStateStore};

pub(super) async fn run(state: Arc<dyn DevRuntimeStateStore>, events: Arc<EventBus>) {
    let progress_events = Arc::clone(&events);
    super::run_percent_task(
        state,
        DevMockTarget::SelfRecompile,
        super::MOCK_RECOMPILE_DELAY,
        move |percent| {
            progress_events.send(DaemonEvent::SelfRecompileProgress {
                percent,
                phase: recompile_phase(percent).to_string(),
            })
        },
        move || events.send(DaemonEvent::SelfRecompileComplete),
    )
    .await;
}

fn recompile_phase(percent: u8) -> &'static str {
    match percent {
        0..=10 => "Preparing build",
        11..=35 => "Resolving dependencies",
        36..=95 => "Compiling crates",
        _ => "Finalizing build",
    }
}
