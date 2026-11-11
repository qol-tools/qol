use super::NATIVE_NOTIFICATIONS_ID;
use crate::doctor::framework::CheckReport;
use crate::features::notifications::takeover::{self, Health, Outcome, TakeoverState};

const TAKEOVER_ROUTE: &str = "/api/core/notifications/queries/takeover";

pub(super) fn diagnosis() -> CheckReport {
    match current_state() {
        Ok(state) => report(&state),
        Err(reason) => CheckReport::ok(format!(
            "what happens to other apps' notifications is unknown: qol-tray did not answer ({reason})"
        )),
    }
}

fn current_state() -> Result<TakeoverState, String> {
    if takeover::started() {
        return Ok(takeover::state());
    }
    let (status, body) = qol_plugin_api::host_exec::get_from_daemon(TAKEOVER_ROUTE)
        .map_err(|error| error.to_string())?;
    if status != 200 {
        return Err(format!("HTTP {status}"));
    }
    serde_json::from_str(&body).map_err(|error| error.to_string())
}

fn report(state: &TakeoverState) -> CheckReport {
    let healthy = |summary: &str| CheckReport::ok(summary);
    let broken = |summary: &str| {
        CheckReport::warn(
            format!("{summary}{}", remedy(state)),
            NATIVE_NOTIFICATIONS_ID,
            Vec::new(),
        )
    };
    match state.value {
        Outcome::Off => healthy("other apps' notifications are left to the system"),
        Outcome::Starting => healthy("qol is starting to take over other apps' notifications"),
        Outcome::Toasts => healthy("other apps' notifications are shown as qol toasts"),
        Outcome::Hidden => healthy(
            "other apps' notifications are hidden; qol cannot show them as toasts on this system",
        ),
        Outcome::Muted => {
            healthy("notifications are off, so other apps' notifications are hidden too")
        }
        Outcome::Unsupported => healthy(
            "qol cannot take over other apps' notifications on this system; the system shows them",
        ),
        Outcome::ToastsPartly => {
            broken("other apps' notifications are shown as qol toasts, but some are missed")
        }
        Outcome::ToastsAndBanners => {
            broken("other apps' notifications show twice, as qol toasts and as system banners")
        }
        Outcome::Blocked => broken("the system still shows other apps' notifications"),
        Outcome::Failed => broken("qol could not take over other apps' notifications"),
    }
}

fn remedy(state: &TakeoverState) -> String {
    let permissions: Vec<&str> = state
        .permissions()
        .into_iter()
        .map(|permission| permission.label())
        .collect();
    if !permissions.is_empty() {
        return format!(
            "; allow qol-tray {} in System Settings > Privacy & Security",
            permissions.join(" and ")
        );
    }
    let failing: Vec<String> = state
        .mechanisms
        .iter()
        .filter(|mechanism| mechanism.health != Health::Active)
        .map(|mechanism| mechanism.describe())
        .collect();
    if failing.is_empty() {
        return String::new();
    }
    format!(" ({})", failing.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::notifications::takeover::{Mechanism, Permission};

    fn history(health: Health) -> Mechanism {
        Mechanism::showing("notification history", health)
    }

    fn watch(health: Health) -> Mechanism {
        Mechanism::showing_and_hiding("banner watch", health)
    }

    #[test]
    fn the_report_names_what_happens_and_what_to_allow() {
        let disk = Health::NeedsPermission(Permission::FullDiskAccess);
        let access = Health::NeedsPermission(Permission::Accessibility);
        let gone = Health::Failed("Notification Center is not running".to_string());
        let cases = [
            (false, vec![], 0, "left to the system"),
            (true, vec![], 0, "cannot take over"),
            (
                true,
                vec![history(Health::Active), watch(Health::Active)],
                0,
                "shown as qol toasts",
            ),
            (
                true,
                vec![Mechanism::hiding("desktop banner switch", Health::Active)],
                0,
                "hidden",
            ),
            (
                true,
                vec![history(disk.clone()), watch(Health::Active)],
                1,
                "allow qol-tray Full Disk Access",
            ),
            (
                true,
                vec![history(Health::Active), watch(access.clone())],
                1,
                "allow qol-tray Accessibility",
            ),
            (
                true,
                vec![history(disk), watch(access)],
                1,
                "Full Disk Access and Accessibility",
            ),
            (
                true,
                vec![history(Health::Active), watch(gone)],
                1,
                "banner watch: failed: Notification Center is not running",
            ),
        ];
        for (wanted, mechanisms, issues, needle) in cases {
            let report = report(&TakeoverState::new(wanted, true, mechanisms));
            assert_eq!(report.issues.len(), issues, "{}", report.summary);
            assert!(report.summary.contains(needle), "{}", report.summary);
        }
    }
}
