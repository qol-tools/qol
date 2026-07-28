const LIFELINE_AUDIT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
const LIFELINE_AUDIT_ATTEMPTS: u32 = 6;

pub(super) fn settle_missing_lifelines(expected: &[String]) -> Vec<String> {
    let client = qol_runtime::PlatformStateClient::new(crate::dev_generation::state_socket_path());
    let mut missing = expected.to_vec();
    for _ in 0..LIFELINE_AUDIT_ATTEMPTS {
        std::thread::sleep(LIFELINE_AUDIT_INTERVAL);
        let armed = client.armed_lifelines().unwrap_or_default();
        missing.retain(|id| !armed.contains(id));
        if missing.is_empty() {
            return missing;
        }
    }
    missing
}
