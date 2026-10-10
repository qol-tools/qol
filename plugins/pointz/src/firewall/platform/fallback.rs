use qol_headless::DoctorCheckResult;

use super::super::{details, ports_label};

pub(crate) fn check(id: &str) -> DoctorCheckResult {
    DoctorCheckResult::warn(
        id,
        format!(
            "PointZ does not inspect the {} firewall; phones need {} open",
            std::env::consts::OS,
            ports_label()
        ),
    )
    .with_details(details("not_inspected"))
}

pub(crate) fn allow() -> Result<String, String> {
    Err(format!(
        "PointZ manages firewall rules only on Windows, not on {}",
        std::env::consts::OS
    ))
}

#[cfg(test)]
mod tests {
    use qol_headless::DoctorStatus;

    use super::*;

    #[test]
    fn fallback_platforms_warn_without_inspecting_or_changing_rules() {
        let result = check("firewall");
        assert_eq!(result.status, DoctorStatus::Warn);
        assert!(result.fix.is_none());
        assert_eq!(result.details.unwrap()["rule_changed"], false);
        assert!(allow().is_err());
    }
}
