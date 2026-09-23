pub mod elevation;
pub mod policy;
pub mod privilege;
pub mod residency;
pub mod takeover;
pub mod udev;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixState {
    Pending,
    LiveOnly,
    Applied,
}

impl FixState {
    pub fn name(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::LiveOnly => "live_only",
            Self::Applied => "applied",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub state: Option<FixState>,
    pub actionable: bool,
}

impl Finding {
    pub fn advice(
        id: impl Into<String>,
        title: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            detail: detail.into(),
            state: None,
            actionable: false,
        }
    }

    pub fn fixable(
        id: impl Into<String>,
        title: impl Into<String>,
        detail: impl Into<String>,
        state: FixState,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            detail: detail.into(),
            state: Some(state),
            actionable: state != FixState::Applied,
        }
    }

    pub fn unavailable(mut self, reason: impl Into<String>) -> Self {
        self.detail = format!("{} - {}", self.detail, reason.into());
        self.actionable = false;
        self
    }

    fn accent(&self) -> &'static str {
        if self.actionable {
            return "warning";
        }
        match self.state {
            Some(FixState::Applied) => "success",
            Some(FixState::Pending) | Some(FixState::LiveOnly) | None => "muted",
        }
    }

    fn badge(&self) -> &'static str {
        if self.actionable {
            return "Fix available";
        }
        match self.state {
            Some(FixState::Applied) => "Applied",
            Some(FixState::LiveOnly) => "Until reboot",
            Some(FixState::Pending) => "Unavailable",
            None => "Info",
        }
    }
}

pub trait HostFixes {
    fn detect(&self) -> Vec<Finding>;
    fn apply(&self, id: &str) -> anyhow::Result<String>;
}

pub fn rollup(findings: &[Finding]) -> &'static str {
    if findings.is_empty() {
        return "none";
    }
    if findings.iter().any(|finding| finding.actionable) {
        return "attention";
    }
    "ok"
}

pub fn findings_payload(findings: &[Finding]) -> serde_json::Value {
    let message = findings
        .iter()
        .filter(|finding| finding.actionable)
        .map(|finding| format!("{}: {}", finding.title, finding.detail))
        .collect::<Vec<_>>()
        .join("; ");
    let items = findings
        .iter()
        .map(|finding| {
            serde_json::json!({
                "id": finding.id,
                "title": finding.title,
                "detail": finding.detail,
                "fix_state": finding.state.map(FixState::name),
                "actionable": finding.actionable,
                "accent": finding.accent(),
                "badge": finding.badge(),
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "state": rollup(findings),
        "message": message,
        "items": items,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollup_reports_attention_only_for_actionable_findings() {
        let advice = Finding::advice("a", "Audio server", "PipeWire");
        let applied = Finding::fixable("b", "Service", "healthy", FixState::Applied);
        let pending = Finding::fixable("c", "Service", "wedged", FixState::Pending);
        let cases = [
            ("empty", vec![], "none"),
            ("advice only", vec![advice.clone()], "ok"),
            ("applied only", vec![applied.clone()], "ok"),
            (
                "one pending",
                vec![advice, applied, pending.clone()],
                "attention",
            ),
            (
                "blocked pending",
                vec![pending.unavailable("no pkexec")],
                "ok",
            ),
        ];
        for (label, findings, expected) in cases {
            assert_eq!(rollup(&findings), expected, "case: {label}");
        }
    }

    #[test]
    fn findings_carry_row_keys_the_settings_contract_reads() {
        let findings = vec![
            Finding::fixable(
                "wedged",
                "Bluetooth service",
                "rejecting audio",
                FixState::Pending,
            ),
            Finding::advice("audio", "Audio server", "PipeWire 1.0.5"),
        ];
        let payload = findings_payload(&findings);
        assert_eq!(payload["state"], "attention");
        assert_eq!(payload["message"], "Bluetooth service: rejecting audio");
        let first = &payload["items"][0];
        assert_eq!(first["id"], "wedged");
        assert_eq!(first["title"], "Bluetooth service");
        assert_eq!(first["detail"], "rejecting audio");
        assert_eq!(first["fix_state"], "pending");
        assert_eq!(first["actionable"], true);
        assert_eq!(first["accent"], "warning");
        assert_eq!(first["badge"], "Fix available");
        let second = &payload["items"][1];
        assert_eq!(second["fix_state"], serde_json::Value::Null);
        assert_eq!(second["actionable"], false);
        assert_eq!(second["accent"], "muted");
        assert_eq!(second["badge"], "Info");
    }

    #[test]
    fn applied_fixes_are_never_actionable() {
        let finding = Finding::fixable("x", "t", "d", FixState::Applied);
        assert!(!finding.actionable);
        assert_eq!(finding.accent(), "success");
        assert_eq!(finding.badge(), "Applied");
    }
}
