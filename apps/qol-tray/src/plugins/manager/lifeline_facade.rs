#[cfg(unix)]
const LIFELINE_AUDIT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
#[cfg(unix)]
const LIFELINE_AUDIT_ATTEMPTS: u32 = 6;

/// Polls the armed set, returning daemons still missing after the last attempt.
/// Daemons connect their lifeline shortly after spawn, and a dev recompile
/// briefly resets the host's armed set; retrying until it settles avoids
/// false alarms during those windows while still catching a real leak.
#[cfg(unix)]
pub(super) fn settle_missing_lifelines(expected: &[String]) -> Vec<String> {
    let client = qol_runtime::PlatformStateClient::new(std::path::PathBuf::from(
        crate::paths::STATE_SOCKET_PATH,
    ));
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

/// Host-death lifelines ride a unix-domain socket exposed by the platform-state
/// broker, which exists only on unix targets. Elsewhere there is nothing to
/// audit, so no daemon is ever reported as leaking.
#[cfg(not(unix))]
pub(super) fn settle_missing_lifelines(_expected: &[String]) -> Vec<String> {
    Vec::new()
}
