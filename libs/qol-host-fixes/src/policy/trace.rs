use super::managed;
use anyhow::Result;
use qol_runtime::probe;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CarrierObservation {
    pub token: String,
    pub reason: String,
}

pub fn raw_operation(args: &[String]) -> String {
    let Some(first) = args.first() else {
        return "status".to_string();
    };
    let operation = first
        .strip_prefix("__resident-policy-")
        .unwrap_or(first.as_str());
    probe::token(operation)
}

pub fn carrier_observation(operation: &str) -> CarrierObservation {
    if matches!(operation, "status" | "help") {
        return CarrierObservation {
            token: "not_applicable".to_string(),
            reason: String::new(),
        };
    }
    let proof = match carrier_grade_for(operation) {
        managed::CarrierGrade::Activation => managed::carrier_proof_activation(),
        managed::CarrierGrade::Release => managed::carrier_proof(),
    };
    match proof {
        Ok(_) => CarrierObservation {
            token: "managed".to_string(),
            reason: String::new(),
        },
        Err(managed::CarrierError::NotCanonicalPath { .. }) => CarrierObservation {
            token: "unmanaged".to_string(),
            reason: String::new(),
        },
        Err(error) => CarrierObservation {
            token: "unknown".to_string(),
            reason: sanitize_reason(&error.to_string()),
        },
    }
}

pub fn outcome_of(result: &Result<i32>) -> &'static str {
    match result {
        Ok(code) if *code == 0 => "ok",
        Ok(_) => "exit_nonzero",
        Err(_) => "error",
    }
}

pub fn error_reason(result: &Result<i32>) -> String {
    match result {
        Err(error) => sanitize_reason(&format!("{error:#}")),
        Ok(_) => String::new(),
    }
}

pub fn policy_state() -> String {
    super::nvidia::status(&super::ResidentPolicy::nvidia())
        .map(|view| view.state.as_str().to_string())
        .unwrap_or_else(|_| "unreadable".to_string())
}

pub fn sanitize_reason(detail: &str) -> String {
    probe::token(detail)
}

pub fn request_message(operation: &str, carrier: &CarrierObservation) -> String {
    format!(
        "operation={} carrier={} carrier_reason={}",
        sanitize_reason(operation),
        sanitize_reason(&carrier.token),
        sanitize_reason(&carrier.reason)
    )
}

pub fn result_message(
    operation: &str,
    carrier: &CarrierObservation,
    outcome: &str,
    state: &str,
    reason: &str,
) -> String {
    format!(
        "operation={} carrier={} carrier_reason={} outcome={} state={} reason={}",
        sanitize_reason(operation),
        sanitize_reason(&carrier.token),
        sanitize_reason(&carrier.reason),
        sanitize_reason(outcome),
        sanitize_reason(state),
        sanitize_reason(reason)
    )
}

pub(crate) trait EmissionRecorder {
    fn on_request(&mut self);
    fn on_result(&mut self);
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct NoopEmissionRecorder;

impl EmissionRecorder for NoopEmissionRecorder {
    fn on_request(&mut self) {}
    fn on_result(&mut self) {}
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CountedEmissionRecorder {
    pub(crate) requests: usize,
    pub(crate) results: usize,
}

#[cfg(test)]
impl EmissionRecorder for CountedEmissionRecorder {
    fn on_request(&mut self) {
        self.requests += 1;
    }

    fn on_result(&mut self) {
        self.results += 1;
    }
}

pub(crate) fn carrier_grade_for(operation: &str) -> managed::CarrierGrade {
    if operation == "disable" {
        managed::CarrierGrade::Release
    } else {
        managed::CarrierGrade::Activation
    }
}

pub fn cli_request(args: &[String]) -> CarrierObservation {
    let operation = raw_operation(args);
    let carrier = carrier_observation(&operation);
    qol_runtime::probe!(
        "GPU_DRIVER_SYNC_POLICY_REQUEST",
        "{}",
        request_message(&operation, &carrier)
    );
    carrier
}

pub fn cli_result(args: &[String], carrier: &CarrierObservation, outcome: &str, reason: &str) {
    let operation = raw_operation(args);
    let state = policy_state();
    qol_runtime::probe!(
        "GPU_DRIVER_SYNC_POLICY_RESULT",
        "{}",
        result_message(&operation, carrier, outcome, &state, reason)
    );
}

#[cfg(test)]
mod tests {
    use super::super::test_support;
    use super::*;

    #[test]
    fn raw_operation_is_bounded_and_hidden_prefixed() {
        assert_eq!(raw_operation(&[]), "status");
        assert_eq!(raw_operation(&["status".to_string()]), "status");
        assert_eq!(
            raw_operation(&[
                "__resident-policy-disable".to_string(),
                "--policy".to_string()
            ]),
            "disable"
        );
        assert_eq!(
            raw_operation(&["__resident-policy-help".to_string()]),
            "help"
        );
        let mut noisy = "x".repeat(200);
        noisy.push_str(" with spaces!");
        assert_eq!(raw_operation(&[noisy.clone()]), probe::token(&noisy));
        assert!(raw_operation(&[noisy]).len() <= 96);
    }

    #[test]
    fn carrier_observation_never_probes_for_help_or_status() {
        assert_eq!(
            carrier_observation("status"),
            CarrierObservation {
                token: "not_applicable".to_string(),
                reason: String::new(),
            }
        );
        assert_eq!(
            carrier_observation("help"),
            CarrierObservation {
                token: "not_applicable".to_string(),
                reason: String::new(),
            }
        );
        let observation = carrier_observation("enable");
        assert!(
            matches!(
                observation.token.as_str(),
                "unmanaged" | "managed" | "unknown"
            ),
            "{}",
            observation.token
        );
    }

    #[test]
    fn carrier_grade_for_uses_activation_unless_the_operation_is_disable() {
        for operation in ["enable", "join", "transfer", "status", "help", "bogus"] {
            assert_eq!(
                carrier_grade_for(operation),
                super::super::managed::CarrierGrade::Activation,
                "{operation:?} must classify as activation"
            );
        }
        assert_eq!(
            carrier_grade_for("disable"),
            super::super::managed::CarrierGrade::Release,
            "disable must classify as release so prerm removal-desired state stays managed"
        );
    }

    #[test]
    fn emission_messages_sanitize_every_field_at_the_boundary() {
        let carrier = CarrierObservation {
            token: "managed".to_string(),
            reason: "ok".to_string(),
        };
        let message = result_message(
            "disable",
            &carrier,
            "refused",
            "active",
            "hidden residency route requires the __resident-policy-<op> shape\u{1b}[31m",
        );
        assert!(
            !message.contains('\u{1b}'),
            "control characters must be sanitized: {message:?}"
        );
        assert!(
            message
                .split(' ')
                .all(|token| !token.is_empty() && token.matches('=').count() == 1),
            "no caller-supplied field may corrupt key=value framing: {message:?}"
        );
        assert!(
            message.contains("outcome=refused"),
            "the fixed outcome token must survive: {message:?}"
        );
        let request = request_message("enable", &carrier);
        assert!(
            request
                .split(' ')
                .all(|token| !token.is_empty() && token.matches('=').count() == 1),
            "{request:?}"
        );
    }

    #[test]
    fn outcome_classification_is_exhaustive() {
        assert_eq!(outcome_of(&Ok(0)), "ok");
        assert_eq!(outcome_of(&Ok(2)), "exit_nonzero");
        assert_eq!(outcome_of(&Err(anyhow::anyhow!("boom"))), "error");
        assert!(error_reason(&Ok(0)).is_empty());
        assert!(error_reason(&Err(anyhow::anyhow!("boom"))).contains("boom"));
    }

    #[test]
    fn a_traced_malformed_attempt_emits_exactly_one_request_and_one_result() {
        let _guard = test_support::serialized();
        let mut recorder = CountedEmissionRecorder::default();
        let result = super::super::nvidia::run_resident_policy_cli_traced_with(
            &[
                "__resident-policy-bogus".to_string(),
                "--policy".to_string(),
            ],
            &mut recorder,
        );
        assert!(result.is_err(), "the malformed invocation must fail");
        assert_eq!(
            recorder,
            CountedEmissionRecorder {
                requests: 1,
                results: 1,
            },
            "a malformed attempt must still emit one request and one result"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_traced_status_attempt_emits_exactly_one_request_and_one_result() {
        let _guard = test_support::serialized();
        let mut recorder = CountedEmissionRecorder::default();
        let result = super::super::nvidia::run_resident_policy_cli_traced_with(
            &["status".to_string()],
            &mut recorder,
        );
        assert!(result.is_ok());
        assert_eq!(
            recorder,
            CountedEmissionRecorder {
                requests: 1,
                results: 1,
            },
            "a valid attempt must emit one request and one result"
        );
    }
}
