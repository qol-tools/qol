use super::Observation;

pub(crate) fn observe(observation: &Observation) {
    let (outcome, detail) = match observation {
        Observation::Unsupported => ("unsupported", String::new()),
        Observation::NotLoaded => ("not_loaded", String::new()),
        Observation::LoadedUnavailable => ("unavailable", "probe=loaded".to_string()),
        Observation::OnDiskUnavailable { loaded } => {
            ("unavailable", format!("probe=on_disk loaded={loaded}"))
        }
        Observation::Matched { loaded } => ("matched", format!("loaded={loaded}")),
        Observation::Mismatch { loaded, on_disk } => {
            ("mismatch", format!("loaded={loaded} on_disk={on_disk}"))
        }
    };
    #[cfg(debug_assertions)]
    {
        qol_runtime::probe!("GPU_DRIVER_SYNC_OBSERVE", "outcome={outcome} {detail}");
    }
    #[cfg(not(debug_assertions))]
    let _ = (outcome, detail);
}

pub(crate) fn notify(
    outcome: &str,
    loaded: Option<&str>,
    on_disk: Option<&str>,
    policy: Option<&str>,
) {
    let loaded = loaded.unwrap_or("?");
    let on_disk = on_disk.unwrap_or("?");
    let policy = policy.unwrap_or("?");
    #[cfg(debug_assertions)]
    {
        qol_runtime::probe!(
            "GPU_DRIVER_SYNC_NOTIFY",
            "outcome={outcome} loaded={loaded} on_disk={on_disk} policy={policy}"
        );
    }
    #[cfg(not(debug_assertions))]
    let _ = (outcome, loaded, on_disk, policy);
}
