use super::events::CoreEvent;
use super::state::{CoreBuildProgress, CoreState};
use super::types::{BuildStatus, CoreInput};

fn normalized_percent(status: BuildStatus, percent: u8) -> u8 {
    match status {
        BuildStatus::Queued => 0,
        BuildStatus::Skipped | BuildStatus::Success | BuildStatus::Failed => 100,
        BuildStatus::Building => percent.min(100),
    }
}

pub fn reduce(mut state: CoreState, input: CoreInput) -> (CoreState, Vec<CoreEvent>) {
    match input {
        CoreInput::RunStarted => {
            state.building = true;
            state.progress.clear();
            (state, vec![CoreEvent::BuildStarted])
        }
        CoreInput::PluginProgress {
            plugin_id,
            status,
            percent,
            phase,
        } => {
            if !state.building {
                state.building = true;
            }
            let normalized = normalized_percent(status, percent);
            state.progress.insert(
                plugin_id.clone(),
                CoreBuildProgress {
                    status,
                    percent: normalized,
                    phase: phase.clone(),
                },
            );
            (
                state,
                vec![CoreEvent::BuildPluginProgress {
                    plugin_id,
                    status,
                    percent: normalized,
                    phase,
                }],
            )
        }
        CoreInput::RunFinished { results } => {
            state.building = false;
            state.progress.clear();
            (state, vec![CoreEvent::BuildComplete { results }])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dev::core::types::{BuildStatus, CoreInput};
    use proptest::prelude::*;

    fn status_from_code(code: u8) -> BuildStatus {
        match code % 5 {
            0 => BuildStatus::Queued,
            1 => BuildStatus::Building,
            2 => BuildStatus::Skipped,
            3 => BuildStatus::Success,
            _ => BuildStatus::Failed,
        }
    }

    fn run_sequence(inputs: &[CoreInput]) -> (CoreState, Vec<CoreEvent>) {
        let mut state = CoreState::default();
        let mut events = Vec::new();
        for input in inputs {
            let (next, emitted) = reduce(state, input.clone());
            state = next;
            events.extend(emitted);
        }
        (state, events)
    }

    proptest! {
        #[test]
        fn reducer_is_deterministic(
            plugin_ids in prop::collection::vec("[a-z0-9_-]{1,8}", 1..8),
            steps in prop::collection::vec((0usize..32, 0u8..10u8, 0u8..=100u8, "[A-Za-z0-9 _/\\-]{0,24}"), 1..256)
        ) {
            let mut inputs = Vec::with_capacity(steps.len() + 2);
            inputs.push(CoreInput::RunStarted);
            for (idx, status_code, percent, phase) in &steps {
                let plugin_id = plugin_ids[idx % plugin_ids.len()].clone();
                inputs.push(CoreInput::PluginProgress {
                    plugin_id,
                    status: status_from_code(*status_code),
                    percent: *percent,
                    phase: phase.clone(),
                });
            }
            inputs.push(CoreInput::RunFinished { results: Vec::new() });

            let left = run_sequence(&inputs);
            let right = run_sequence(&inputs);
            prop_assert_eq!(left, right);
        }

        #[test]
        fn reducer_emits_bounded_percent_for_progress(
            plugin_id in "[a-z0-9_-]{1,12}",
            status_code in 0u8..10u8,
            percent in 0u8..=255u8,
            phase in "[A-Za-z0-9 _/\\-]{0,24}"
        ) {
            let mut state = CoreState::default();
            let (next, _) = reduce(state.clone(), CoreInput::RunStarted);
            state = next;

            let status = status_from_code(status_code);
            let (next, emitted) = reduce(
                state,
                CoreInput::PluginProgress {
                    plugin_id,
                    status,
                    percent,
                    phase,
                },
            );
            let event = emitted.into_iter().next().expect("event");
            if let CoreEvent::BuildPluginProgress { percent, .. } = event {
                prop_assert!(percent <= 100);
            }
            if let Some(entry) = next.progress.values().next() {
                prop_assert!(entry.percent <= 100);
            }
        }
    }
}
