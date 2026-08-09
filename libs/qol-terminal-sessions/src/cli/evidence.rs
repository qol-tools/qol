use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CliRuntimeState {
    Working,
    Ready,
    NeedsInput,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CliViewportState {
    Live,
    Historical,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CliActivityEvidence {
    pub file_fresh: Option<bool>,
    pub file_has_work: Option<bool>,
}

impl CliActivityEvidence {
    pub fn combined(self) -> Option<bool> {
        self.file_fresh
            .zip(self.file_has_work)
            .map(|(fresh, work)| fresh && work)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CliSessionEvidence {
    pub runtime: CliRuntimeState,
    pub activity: CliActivityEvidence,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CliScreenEvidence {
    pub runtime: CliRuntimeState,
    pub viewport: CliViewportState,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CliLaunchProgram {
    pub program: String,
    pub args: Vec<String>,
}

impl CliLaunchProgram {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CliActivityEvidence, CliLaunchProgram, CliRuntimeState, CliScreenEvidence,
        CliSessionEvidence, CliViewportState,
    };

    #[test]
    fn runtime_and_viewport_default_to_unknown() {
        assert_eq!(CliRuntimeState::default(), CliRuntimeState::Unknown);
        assert_eq!(CliViewportState::default(), CliViewportState::Unknown);
        assert_eq!(
            CliSessionEvidence::default().runtime,
            CliRuntimeState::Unknown
        );
        assert_eq!(
            CliSessionEvidence::default().activity,
            CliActivityEvidence::default()
        );
        assert_eq!(
            CliScreenEvidence::default().viewport,
            CliViewportState::Unknown
        );
        assert_eq!(
            CliScreenEvidence::default().runtime,
            CliRuntimeState::Unknown
        );
    }

    #[test]
    fn runtime_state_serde_round_trips_every_variant() {
        let cases = [
            CliRuntimeState::Working,
            CliRuntimeState::Ready,
            CliRuntimeState::NeedsInput,
            CliRuntimeState::Unknown,
        ];
        for state in cases {
            let encoded = serde_json::to_string(&state).unwrap();
            let decoded = serde_json::from_str::<CliRuntimeState>(&encoded).unwrap();
            assert_eq!(decoded, state, "encoded: {encoded}");
        }
        assert_eq!(
            serde_json::to_string(&CliRuntimeState::NeedsInput).unwrap(),
            "\"needs_input\""
        );
    }

    #[test]
    fn viewport_state_serde_round_trips_every_variant() {
        let cases = [
            CliViewportState::Live,
            CliViewportState::Historical,
            CliViewportState::Unknown,
        ];
        for viewport in cases {
            let encoded = serde_json::to_string(&viewport).unwrap();
            assert_eq!(
                serde_json::from_str::<CliViewportState>(&encoded).unwrap(),
                viewport
            );
        }
    }

    #[test]
    fn weak_activity_never_implies_a_runtime_state() {
        let combinations = [
            CliActivityEvidence {
                file_fresh: None,
                file_has_work: None,
            },
            CliActivityEvidence {
                file_fresh: Some(true),
                file_has_work: None,
            },
            CliActivityEvidence {
                file_fresh: None,
                file_has_work: Some(true),
            },
            CliActivityEvidence {
                file_fresh: Some(true),
                file_has_work: Some(true),
            },
            CliActivityEvidence {
                file_fresh: Some(false),
                file_has_work: Some(true),
            },
            CliActivityEvidence {
                file_fresh: Some(true),
                file_has_work: Some(false),
            },
        ];
        for activity in combinations {
            let evidence = CliSessionEvidence {
                runtime: CliRuntimeState::Unknown,
                activity,
            };
            assert_eq!(evidence.runtime, CliRuntimeState::Unknown);
        }
    }

    #[test]
    fn combined_activity_requires_freshness_and_work() {
        let cases = [
            (
                CliActivityEvidence {
                    file_fresh: Some(true),
                    file_has_work: Some(true),
                },
                Some(true),
            ),
            (
                CliActivityEvidence {
                    file_fresh: Some(false),
                    file_has_work: Some(true),
                },
                Some(false),
            ),
            (
                CliActivityEvidence {
                    file_fresh: Some(true),
                    file_has_work: Some(false),
                },
                Some(false),
            ),
            (
                CliActivityEvidence {
                    file_fresh: None,
                    file_has_work: Some(true),
                },
                None,
            ),
            (
                CliActivityEvidence {
                    file_fresh: Some(true),
                    file_has_work: None,
                },
                None,
            ),
            (
                CliActivityEvidence {
                    file_fresh: None,
                    file_has_work: None,
                },
                None,
            ),
        ];
        for (activity, expected) in cases {
            assert_eq!(activity.combined(), expected);
        }
    }

    #[test]
    fn launch_program_carries_the_program_and_empty_default_args() {
        let launch = CliLaunchProgram::new("codex");
        assert_eq!(launch.program, "codex");
        assert!(launch.args.is_empty());
        assert_eq!(
            serde_json::from_str::<CliLaunchProgram>(&serde_json::to_string(&launch).unwrap())
                .unwrap(),
            launch
        );
    }
}
