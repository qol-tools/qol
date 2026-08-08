pub(crate) fn hold(names: &[String], outcome: &str, reason: Option<&str>) {
    #[cfg(debug_assertions)]
    {
        let packages = names.join(",");
        match reason {
            Some(reason) => qol_runtime::probe!(
                "GPU_GUARD_HOLD",
                "packages={packages} outcome={outcome} reason={reason}"
            ),
            None => qol_runtime::probe!("GPU_GUARD_HOLD", "packages={packages} outcome={outcome}"),
        }
    }
    #[cfg(not(debug_assertions))]
    let _ = (names, outcome, reason);
}

pub(crate) fn unhold(names: &[String], outcome: &str, reason: Option<&str>) {
    #[cfg(debug_assertions)]
    {
        let packages = names.join(",");
        match reason {
            Some(reason) => qol_runtime::probe!(
                "GPU_GUARD_UNHOLD",
                "packages={packages} outcome={outcome} reason={reason}"
            ),
            None => {
                qol_runtime::probe!("GPU_GUARD_UNHOLD", "packages={packages} outcome={outcome}")
            }
        }
    }
    #[cfg(not(debug_assertions))]
    let _ = (names, outcome, reason);
}

pub(crate) fn update(names: &[String], outcome: &str, reason: Option<&str>) {
    #[cfg(debug_assertions)]
    {
        let packages = names.join(",");
        match reason {
            Some(reason) => qol_runtime::probe!(
                "GPU_GUARD_UPDATE",
                "packages={packages} outcome={outcome} reason={reason}"
            ),
            None => {
                qol_runtime::probe!("GPU_GUARD_UPDATE", "packages={packages} outcome={outcome}")
            }
        }
    }
    #[cfg(not(debug_assertions))]
    let _ = (names, outcome, reason);
}

pub(crate) fn notify(packages: &[String], outcome: &str, reason: Option<&str>) {
    #[cfg(debug_assertions)]
    {
        let joined = packages.join(",");
        match reason {
            Some(reason) => qol_runtime::probe!(
                "GPU_GUARD_NOTIFY",
                "packages={joined} outcome={outcome} reason={reason}"
            ),
            None => {
                qol_runtime::probe!("GPU_GUARD_NOTIFY", "packages={joined} outcome={outcome}")
            }
        }
    }
    #[cfg(not(debug_assertions))]
    let _ = (packages, outcome, reason);
}
