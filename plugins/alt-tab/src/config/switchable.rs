use serde::{Deserialize, Serialize};

pub const QOL_SWITCHABLE_PANELS: &[&str] = &["cli-sessions"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SwitchablePanelOverride {
    pub app: String,
    #[serde(default)]
    pub switchable: bool,
}

#[derive(Debug, Clone)]
pub struct SwitchablePanels(Vec<String>);

impl SwitchablePanels {
    pub fn resolve(overrides: &[SwitchablePanelOverride]) -> Self {
        let mut apps: Vec<String> = QOL_SWITCHABLE_PANELS
            .iter()
            .map(|app| (*app).to_string())
            .collect();
        for row in overrides {
            let app = row.app.trim();
            if app.is_empty() {
                continue;
            }
            apps.retain(|listed| !listed.eq_ignore_ascii_case(app));
            if row.switchable {
                apps.push(app.to_string());
            }
        }
        Self(apps)
    }

    pub fn allows(&self, app_name: &str) -> bool {
        let app_name = app_name.trim();
        self.0.iter().any(|app| app.eq_ignore_ascii_case(app_name))
    }
}

impl Default for SwitchablePanels {
    fn default() -> Self {
        Self::resolve(&[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn override_row(app: &str, switchable: bool) -> SwitchablePanelOverride {
        SwitchablePanelOverride {
            app: app.to_string(),
            switchable,
        }
    }

    #[test]
    fn qol_panels_are_switchable_without_any_user_overrides() {
        let apps = SwitchablePanels::default();
        assert!(apps.allows("cli-sessions"));
        assert!(!apps.allows("Microsoft Teams"));
    }

    #[test]
    fn user_can_remove_a_qol_panel() {
        let apps = SwitchablePanels::resolve(&[override_row("cli-sessions", false)]);
        assert!(!apps.allows("cli-sessions"));
    }

    #[test]
    fn user_can_add_an_app_qol_does_not_ship() {
        let apps = SwitchablePanels::resolve(&[override_row("Stickies", true)]);
        assert!(apps.allows("Stickies"));
        assert!(apps.allows("cli-sessions"));
    }

    #[test]
    fn matching_ignores_case_and_surrounding_space() {
        let apps = SwitchablePanels::resolve(&[override_row("  Stickies  ", true)]);
        assert!(apps.allows("STICKIES"));
        assert!(apps.allows(" stickies "));
        assert!(apps.allows("CLI-Sessions"));
    }

    #[test]
    fn later_override_row_wins_over_earlier_one() {
        let apps = SwitchablePanels::resolve(&[
            override_row("cli-sessions", false),
            override_row("CLI-SESSIONS", true),
        ]);
        assert!(apps.allows("cli-sessions"));
    }

    #[test]
    fn blank_override_rows_are_ignored() {
        let apps = SwitchablePanels::resolve(&[override_row("   ", false), override_row("", true)]);
        assert!(apps.allows("cli-sessions"));
        assert!(!apps.allows(""));
    }
}
