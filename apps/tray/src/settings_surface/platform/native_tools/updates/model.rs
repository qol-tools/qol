use std::collections::BTreeSet;

use qol_config::text::plural;
use qol_connection_health::ConnectionHealth;
use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum TargetState {
    UpToDate,
    Unchecked,
    Available,
    Queued,
    Updating,
    Failed,
    DevBuild,
    DevLinked,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct UpdateTarget {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) current: String,
    #[serde(default)]
    pub(super) latest: Option<String>,
    pub(super) state: TargetState,
    #[serde(default)]
    pub(super) progress: Option<f64>,
    #[serde(default)]
    pub(super) error: Option<String>,
    #[serde(default)]
    pub(super) updated_from: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct UpdatesSnapshot {
    pub(super) checking: bool,
    pub(super) checks_enabled: bool,
    #[serde(default)]
    pub(super) last_checked_secs: Option<u64>,
    #[serde(default)]
    pub(super) last_success_secs: Option<u64>,
    #[serde(default)]
    pub(super) check_error: Option<String>,
    #[serde(default)]
    pub(super) connection: ConnectionHealth,
    pub(super) running: bool,
    pub(super) host: UpdateTarget,
    #[serde(default)]
    pub(super) plugins: Vec<UpdateTarget>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ValueTone {
    Normal,
    Muted,
    Attention,
    Danger,
    Success,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum RowAction {
    Check,
    Update(String),
    UpdateAll,
    StopUpdates,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SummaryDot {
    Danger,
    Warning,
    Success,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Summary {
    pub(super) dot: Option<SummaryDot>,
    pub(super) busy: bool,
    pub(super) label: String,
    pub(super) description: String,
    pub(super) action: Option<RowAction>,
    pub(super) action_label: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TargetRow {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) value: String,
    pub(super) tone: ValueTone,
    pub(super) description: Option<String>,
    pub(super) action: Option<RowAction>,
    pub(super) action_label: Option<&'static str>,
    pub(super) attention: bool,
    pub(super) spinner: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PageModel {
    pub(super) summary: Summary,
    pub(super) host: TargetRow,
    pub(super) plugins: Vec<TargetRow>,
    pub(super) fold: Option<String>,
    pub(super) plugin_total: usize,
}

pub(super) fn page(snapshot: &UpdatesSnapshot, visited: &BTreeSet<String>) -> PageModel {
    let plugins = plugin_rows(&snapshot.plugins, visited);
    let checked = !snapshot
        .plugins
        .iter()
        .any(|plugin| plugin.state == TargetState::Unchecked && !visited.contains(&plugin.id));
    PageModel {
        summary: summary(snapshot),
        host: host_row(&snapshot.host),
        fold: fold_text(snapshot.plugins.len(), plugins.len(), checked),
        plugin_total: snapshot.plugins.len(),
        plugins,
    }
}

pub(super) fn plugin_rows(plugins: &[UpdateTarget], visited: &BTreeSet<String>) -> Vec<TargetRow> {
    let mut rows = plugins
        .iter()
        .filter_map(|plugin| match plugin.state {
            TargetState::UpToDate
            | TargetState::Unchecked
            | TargetState::DevLinked
            | TargetState::DevBuild => visited
                .contains(&plugin.id)
                .then(|| finished_row(plugin, &plugin.current)),
            _ => Some(active_row(plugin)),
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| row.name.to_lowercase());
    rows
}

fn active_row(plugin: &UpdateTarget) -> TargetRow {
    let mut row = TargetRow {
        id: plugin.id.clone(),
        name: plugin.name.clone(),
        value: String::new(),
        tone: ValueTone::Normal,
        description: None,
        action: None,
        action_label: None,
        attention: false,
        spinner: false,
    };
    match plugin.state {
        TargetState::Available => {
            row.value = version_jump(&plugin.current, plugin.latest.as_deref());
            row.tone = ValueTone::Attention;
            row.action = Some(RowAction::Update(plugin.id.clone()));
            row.action_label = Some("Update");
            row.attention = true;
        }
        TargetState::Queued => {
            row.value = "Waiting".to_string();
            row.tone = ValueTone::Muted;
        }
        TargetState::Updating => {
            row.value = "Updating".to_string();
            row.tone = ValueTone::Muted;
            row.spinner = true;
        }
        TargetState::Failed => {
            row.value = "Update failed".to_string();
            row.tone = ValueTone::Danger;
            row.description = plugin.error.clone();
            row.action = Some(RowAction::Update(plugin.id.clone()));
            row.action_label = Some("Retry");
            row.attention = true;
        }
        _ => {}
    }
    row
}

fn finished_row(plugin: &UpdateTarget, version: &str) -> TargetRow {
    TargetRow {
        id: plugin.id.clone(),
        name: plugin.name.clone(),
        value: format!("Updated to {version}"),
        tone: ValueTone::Success,
        description: None,
        action: None,
        action_label: None,
        attention: false,
        spinner: false,
    }
}

pub(super) fn host_row(host: &UpdateTarget) -> TargetRow {
    let mut row = TargetRow {
        id: host.id.clone(),
        name: "Version".to_string(),
        value: host.current.clone(),
        tone: ValueTone::Normal,
        description: None,
        action: None,
        action_label: None,
        attention: false,
        spinner: false,
    };
    match host.state {
        TargetState::Available => {
            row.value = version_jump(&host.current, host.latest.as_deref());
            row.tone = ValueTone::Attention;
            row.description = Some("Restarts qol to finish".to_string());
            row.action = Some(RowAction::Update(host.id.clone()));
            row.action_label = Some("Update");
            row.attention = true;
        }
        TargetState::Queued => {
            row.value = "Waiting".to_string();
            row.tone = ValueTone::Muted;
            row.description = Some("Waits for the plugins, then restarts qol".to_string());
        }
        TargetState::Updating => {
            row.value = download_value(host.progress);
            row.tone = ValueTone::Muted;
            row.description = Some("Restarts qol to finish".to_string());
            row.spinner = true;
        }
        TargetState::Failed => {
            row.value = "Update failed".to_string();
            row.tone = ValueTone::Danger;
            row.description = host.error.clone();
            row.action = Some(RowAction::Update(host.id.clone()));
            row.action_label = Some("Retry");
            row.attention = true;
        }
        TargetState::DevBuild => {
            row.value = "Development build".to_string();
            row.tone = ValueTone::Muted;
            row.description = Some("Updates come from rebuilding".to_string());
        }
        TargetState::Unchecked => {
            row.description = Some("Not checked for updates".to_string());
        }
        TargetState::UpToDate | TargetState::DevLinked => {
            row.description = host
                .updated_from
                .as_ref()
                .map(|from| format!("Updated from {from}"));
        }
    }
    row
}

pub(super) fn summary(snapshot: &UpdatesSnapshot) -> Summary {
    if job_running(snapshot) {
        let host_updating = matches!(
            snapshot.host.state,
            TargetState::Queued | TargetState::Updating
        );
        let description = if host_updating {
            "qol restarts when the download is done".to_string()
        } else {
            pending_names(snapshot, &[TargetState::Queued, TargetState::Updating])
        };
        let plugins_queued = snapshot
            .plugins
            .iter()
            .any(|plugin| plugin.state == TargetState::Queued);
        return Summary {
            dot: None,
            busy: true,
            label: if host_updating {
                "Updating qol-tray"
            } else {
                "Updating plugins"
            }
            .to_string(),
            description,
            action: plugins_queued.then_some(RowAction::StopUpdates),
            action_label: plugins_queued.then_some("Stop"),
        };
    }
    let last_check = last_check_text(snapshot);
    let failed = pending_names(snapshot, &[TargetState::Failed]);
    let available = pending_names(snapshot, &[TargetState::Available]);
    let failed_count = pending_count(snapshot, TargetState::Failed);
    let available_count = pending_count(snapshot, TargetState::Available);
    if failed_count + available_count > 0 {
        let (label, names, dot) = match (failed_count, available_count) {
            (0, count) => (
                plural(count, "update waiting", "updates waiting"),
                available,
                SummaryDot::Warning,
            ),
            (count, 0) => (
                plural(count, "update failed", "updates failed"),
                failed,
                SummaryDot::Danger,
            ),
            (count, _) => (
                plural(count, "update failed", "updates failed"),
                format!("{failed} failed, {available} waiting"),
                SummaryDot::Danger,
            ),
        };
        let (action, action_label) = if failed_count + available_count >= 2 {
            let verb = if available_count > 0 {
                "Update all"
            } else {
                "Retry all"
            };
            (RowAction::UpdateAll, verb)
        } else {
            (RowAction::Check, check_label(snapshot))
        };
        return Summary {
            dot: (!snapshot.checking).then_some(dot),
            busy: snapshot.checking,
            label,
            description: sentence(&names, &last_check),
            action: (!snapshot.checking).then_some(action),
            action_label: (!snapshot.checking).then_some(action_label),
        };
    }
    let check_failed = snapshot.check_error.is_some() && !snapshot.checking;
    if snapshot.checking {
        return Summary {
            dot: None,
            busy: true,
            label: "Checking for updates".to_string(),
            description: last_check,
            action: None,
            action_label: None,
        };
    }
    if check_failed || !unchecked_names(snapshot).is_empty() {
        return Summary {
            dot: Some(SummaryDot::Warning),
            busy: false,
            label: if check_failed {
                "Couldn't check for updates"
            } else {
                "Not checked yet"
            }
            .to_string(),
            description: last_check,
            action: Some(RowAction::Check),
            action_label: Some(check_label(snapshot)),
        };
    }
    let plugins = plural(snapshot.plugins.len(), "plugin", "plugins");
    let (label, checked) = if snapshot.checks_enabled {
        (
            "Everything is up to date",
            format!("qol-tray {} and {plugins}", snapshot.host.current),
        )
    } else {
        ("Plugins are up to date", plugins)
    };
    Summary {
        dot: Some(SummaryDot::Success),
        busy: false,
        label: label.to_string(),
        description: sentence(&checked, &last_check),
        action: Some(RowAction::Check),
        action_label: Some("Check now"),
    }
}

pub(super) fn check_key_label(snapshot: &UpdatesSnapshot) -> Option<&'static str> {
    (summary(snapshot).action == Some(RowAction::UpdateAll)).then(|| check_label(snapshot))
}

fn check_label(snapshot: &UpdatesSnapshot) -> &'static str {
    if snapshot.check_error.is_some() {
        "Try again"
    } else {
        "Check now"
    }
}

fn sentence(lead: &str, last_check: &str) -> String {
    if last_check.is_empty() {
        format!("{lead}.")
    } else {
        format!("{lead}. {last_check}")
    }
}

fn last_check_text(snapshot: &UpdatesSnapshot) -> String {
    match snapshot.check_error.as_ref().filter(|_| !snapshot.checking) {
        Some(error) => match snapshot.last_success_secs {
            Some(secs) => format!("{error}. Last good check {} ago.", long_age(secs)),
            None => format!("{error}."),
        },
        None => match snapshot.last_checked_secs {
            Some(secs) => format!("Checked {} ago.", long_age(secs)),
            None if snapshot.checks_enabled => {
                "qol checks for updates by itself every 5 hours.".to_string()
            }
            None => String::new(),
        },
    }
}

fn unchecked_names(snapshot: &UpdatesSnapshot) -> String {
    let plugins = snapshot
        .plugins
        .iter()
        .filter(|plugin| plugin.state == TargetState::Unchecked)
        .count();
    let mut names = Vec::new();
    if snapshot.host.state == TargetState::Unchecked {
        names.push(snapshot.host.name.clone());
    }
    if plugins > 0 {
        names.push(plural(plugins, "plugin", "plugins"));
    }
    join_names(&names)
}

pub(super) fn fold_text(total: usize, shown: usize, checked: bool) -> Option<String> {
    let hidden = total.saturating_sub(shown);
    if hidden == 0 {
        return None;
    }
    if !checked {
        return Some(unchecked_fold_text(total, shown, hidden));
    }
    if total == 1 {
        return Some("Your plugin is up to date.".to_string());
    }
    if shown == 0 {
        return Some(format!("All {total} are up to date."));
    }
    if hidden == 1 {
        return Some("The other 1 is up to date.".to_string());
    }
    Some(format!("The other {hidden} are up to date."))
}

fn unchecked_fold_text(total: usize, shown: usize, hidden: usize) -> String {
    if total == 1 {
        "Your plugin wasn't checked.".to_string()
    } else if shown == 0 {
        format!("Your {total} plugins weren't checked.")
    } else if hidden == 1 {
        "The other 1 wasn't checked.".to_string()
    } else {
        format!("The other {hidden} weren't checked.")
    }
}

pub(in super::super) fn long_age(secs: u64) -> String {
    if secs < 60 {
        "less than a minute".to_string()
    } else if secs < 3600 {
        let minutes = secs / 60;
        plural(minutes as usize, "minute", "minutes")
    } else if secs < 2 * 86_400 {
        let hours = secs / 3600;
        plural(hours as usize, "hour", "hours")
    } else {
        plural((secs / 86_400) as usize, "day", "days")
    }
}

pub(super) fn join_names(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [single] => single.clone(),
        [head @ .., last] => format!("{} and {last}", head.join(", ")),
    }
}

fn pending_names(snapshot: &UpdatesSnapshot, states: &[TargetState]) -> String {
    let mut names = snapshot
        .plugins
        .iter()
        .filter(|plugin| states.contains(&plugin.state))
        .map(|plugin| plugin.name.clone())
        .collect::<Vec<_>>();
    names.sort_by_key(|name| name.to_lowercase());
    if states.contains(&snapshot.host.state) {
        names.insert(0, snapshot.host.name.clone());
    }
    join_names(&names)
}

fn pending_count(snapshot: &UpdatesSnapshot, state: TargetState) -> usize {
    usize::from(snapshot.host.state == state)
        + snapshot
            .plugins
            .iter()
            .filter(|plugin| plugin.state == state)
            .count()
}

fn job_running(snapshot: &UpdatesSnapshot) -> bool {
    snapshot.running
        || matches!(
            snapshot.host.state,
            TargetState::Queued | TargetState::Updating
        )
        || snapshot
            .plugins
            .iter()
            .any(|plugin| matches!(plugin.state, TargetState::Queued | TargetState::Updating))
}

fn version_jump(current: &str, latest: Option<&str>) -> String {
    match latest {
        Some(latest) => format!("{current} \u{2192} {latest}"),
        None => current.to_string(),
    }
}

fn download_value(progress: Option<f64>) -> String {
    let percent = progress.unwrap_or(0.0).clamp(0.0, 100.0).round() as i64;
    format!("Downloading {percent}%")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{
        check_key_label, fold_text, host_row, join_names, long_age, page, plugin_rows, summary,
        RowAction, SummaryDot, TargetState, UpdateTarget, UpdatesSnapshot, ValueTone,
    };

    #[test]
    fn the_snapshot_carries_the_tray_connection_health() {
        let host =
            r#""host":{"id":"qol-tray","name":"qol-tray","current":"1.0.0","state":"up_to_date"}"#;
        let older: UpdatesSnapshot = serde_json::from_str(&format!(
            r#"{{"checking":true,"checks_enabled":true,"running":false,{host}}}"#
        ))
        .unwrap();
        assert_eq!(
            older.connection.level,
            qol_connection_health::Level::Unknown
        );
        let slow: UpdatesSnapshot = serde_json::from_str(&format!(
            r#"{{"checking":true,"checks_enabled":true,"running":false,"connection":{{"level":"slow","cause":"waiting"}},{host}}}"#
        ))
        .unwrap();
        assert_eq!(slow.connection.level, qol_connection_health::Level::Slow);
        assert_eq!(
            slow.connection.cause,
            Some(qol_connection_health::Cause::Waiting)
        );
    }

    fn target(id: &str, name: &str, state: TargetState) -> UpdateTarget {
        UpdateTarget {
            id: id.to_string(),
            name: name.to_string(),
            current: "1.0.0".to_string(),
            latest: Some("1.1.0".to_string()),
            state,
            progress: None,
            error: None,
            updated_from: None,
        }
    }

    fn snapshot(host: UpdateTarget, plugins: Vec<UpdateTarget>) -> UpdatesSnapshot {
        UpdatesSnapshot {
            checking: false,
            checks_enabled: true,
            last_checked_secs: Some(120),
            last_success_secs: Some(120),
            check_error: None,
            connection: Default::default(),
            running: false,
            host,
            plugins,
        }
    }

    #[test]
    fn long_age_spells_out_the_unit_for_descriptions() {
        let cases = [
            (30, "less than a minute"),
            (60, "1 minute"),
            (720, "12 minutes"),
            (3600, "1 hour"),
            (21_600, "6 hours"),
            (86_400, "24 hours"),
            (4 * 86_400, "4 days"),
        ];
        for (secs, expected) in cases {
            assert_eq!(long_age(secs), expected, "secs={secs}");
        }
    }

    #[test]
    fn join_names_reads_like_a_sentence() {
        let names = |values: &[&str]| {
            values
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(join_names(&names(&[])), "");
        assert_eq!(join_names(&names(&["Bluetooth"])), "Bluetooth");
        assert_eq!(
            join_names(&names(&["Bluetooth", "Launcher"])),
            "Bluetooth and Launcher"
        );
        assert_eq!(
            join_names(&names(&["qol-tray", "CLI Sessions", "Launcher"])),
            "qol-tray, CLI Sessions and Launcher"
        );
    }

    #[test]
    fn fold_text_hides_only_when_rows_are_hidden() {
        let cases = [
            (0, 0, true, None),
            (3, 3, true, None),
            (3, 3, false, None),
            (1, 0, true, Some("Your plugin is up to date.")),
            (9, 1, true, Some("The other 8 are up to date.")),
            (2, 1, true, Some("The other 1 is up to date.")),
            (2, 0, true, Some("All 2 are up to date.")),
            (17, 0, true, Some("All 17 are up to date.")),
            (17, 2, true, Some("The other 15 are up to date.")),
            (1, 0, false, Some("Your plugin wasn't checked.")),
            (17, 0, false, Some("Your 17 plugins weren't checked.")),
            (2, 1, false, Some("The other 1 wasn't checked.")),
            (17, 2, false, Some("The other 15 weren't checked.")),
        ];
        for (total, shown, checked, expected) in cases {
            assert_eq!(
                fold_text(total, shown, checked).as_deref(),
                expected,
                "total={total} shown={shown} checked={checked}"
            );
        }
    }

    #[test]
    fn plugin_rows_keep_only_work_and_visited_finishes_in_name_order() {
        let plugins = vec![
            target("qol-launcher", "Launcher", TargetState::UpToDate),
            target("bluetooth", "Bluetooth", TargetState::Failed),
            target("cli", "CLI Sessions", TargetState::Available),
            target("os-themes", "OS Themes", TargetState::DevLinked),
            target("lights", "Lights", TargetState::UpToDate),
        ];
        let visited = BTreeSet::from(["qol-launcher".to_string()]);
        let rows = plugin_rows(&plugins, &visited);

        let names = rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>();
        assert_eq!(names, ["Bluetooth", "CLI Sessions", "Launcher"]);
        assert_eq!(rows[0].value, "Update failed");
        assert_eq!(rows[0].action, Some(RowAction::Update("bluetooth".into())));
        assert!(rows[0].attention);
        assert_eq!(rows[1].value, "1.0.0 \u{2192} 1.1.0");
        assert_eq!(rows[1].tone, ValueTone::Attention);
        assert_eq!(rows[2].value, "Updated to 1.0.0");
        assert_eq!(rows[2].tone, ValueTone::Success);
        assert_eq!(rows[2].action, None);
    }

    #[test]
    fn plugin_rows_never_show_an_unvisited_dev_linked_plugin() {
        let plugins = vec![target("os-themes", "OS Themes", TargetState::DevLinked)];
        assert!(plugin_rows(&plugins, &BTreeSet::new()).is_empty());
    }

    #[test]
    fn an_unchecked_plugin_folds_unless_it_just_finished_updating() {
        let plugins = vec![
            target("cli", "CLI Sessions", TargetState::Unchecked),
            target("lights", "Lights", TargetState::Unchecked),
        ];
        assert!(plugin_rows(&plugins, &BTreeSet::new()).is_empty());

        let visited = BTreeSet::from(["cli".to_string()]);
        let rows = plugin_rows(&plugins, &visited);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].value, "Updated to 1.0.0");
        assert_eq!(rows[0].tone, ValueTone::Success);

        let snapshot = snapshot(
            target("qol-tray", "qol-tray", TargetState::UpToDate),
            plugins,
        );
        assert_eq!(
            page(&snapshot, &visited).fold,
            Some("The other 1 wasn't checked.".to_string())
        );
        assert_eq!(
            page(&snapshot, &BTreeSet::new()).fold,
            Some("Your 2 plugins weren't checked.".to_string())
        );
    }

    #[test]
    fn summary_never_calls_unchecked_rows_up_to_date() {
        let plugins = vec![
            target("cli", "CLI Sessions", TargetState::Unchecked),
            target("lights", "Lights", TargetState::Unchecked),
        ];
        let mut offline = snapshot(
            target("qol-tray", "qol-tray", TargetState::Unchecked),
            plugins,
        );
        offline.check_error = Some("No connection to GitHub".to_string());
        let failed = summary(&offline);
        assert_eq!(failed.dot, Some(SummaryDot::Warning));
        assert_eq!(failed.label, "Couldn't check for updates");
        assert_eq!(
            failed.description,
            "No connection to GitHub. Last good check 2 minutes ago."
        );
        assert_eq!(failed.action, Some(RowAction::Check));
        assert_eq!(failed.action_label, Some("Try again"));

        offline.checking = true;
        let checking = summary(&offline);
        assert!(checking.busy);
        assert_eq!(checking.dot, None);
        assert_eq!(checking.label, "Checking for updates");
        assert_eq!(checking.description, "Checked 2 minutes ago.");
        assert_eq!(checking.action, None);

        let fresh = snapshot(
            target("qol-tray", "qol-tray", TargetState::Unchecked),
            vec![target("cli", "CLI Sessions", TargetState::UpToDate)],
        );
        let fresh = summary(&fresh);
        assert_eq!(fresh.label, "Not checked yet");
        assert_eq!(fresh.description, "Checked 2 minutes ago.");
        assert_eq!(fresh.action_label, Some("Check now"));

        let plugin_only = snapshot(
            target("qol-tray", "qol-tray", TargetState::UpToDate),
            vec![target("cli", "CLI Sessions", TargetState::Unchecked)],
        );
        assert_eq!(summary(&plugin_only).label, "Not checked yet");

        let waiting = snapshot(
            target("qol-tray", "qol-tray", TargetState::Unchecked),
            vec![target("cli", "CLI Sessions", TargetState::Available)],
        );
        assert_eq!(summary(&waiting).label, "1 update waiting");
    }

    #[test]
    fn the_check_key_stays_reachable_while_update_all_holds_the_button() {
        let mut waiting = snapshot(
            target("qol-tray", "qol-tray", TargetState::Available),
            vec![target("cli", "CLI Sessions", TargetState::Available)],
        );
        assert_eq!(summary(&waiting).action, Some(RowAction::UpdateAll));
        assert_eq!(check_key_label(&waiting), Some("Check now"));

        waiting.check_error = Some("No connection to GitHub".to_string());
        assert_eq!(check_key_label(&waiting), Some("Try again"));

        waiting.checking = true;
        assert_eq!(check_key_label(&waiting), None);

        let one_waiting = snapshot(
            target("qol-tray", "qol-tray", TargetState::Available),
            vec![target("cli", "CLI Sessions", TargetState::UpToDate)],
        );
        assert_eq!(summary(&one_waiting).action, Some(RowAction::Check));
        assert_eq!(check_key_label(&one_waiting), None);

        let mut running = waiting.clone();
        running.checking = false;
        running.running = true;
        assert_eq!(check_key_label(&running), None);
    }

    #[test]
    fn summary_running_wins_over_failures_and_waiting_updates() {
        let mut running = snapshot(
            target("qol-tray", "qol-tray", TargetState::Queued),
            vec![target("cli", "CLI Sessions", TargetState::Failed)],
        );
        running.running = true;
        let busy = summary(&running);
        assert!(busy.busy);
        assert_eq!(busy.label, "Updating qol-tray");
        assert_eq!(busy.description, "qol restarts when the download is done");
        assert_eq!(busy.action, None);

        let mut host_running = snapshot(
            target("qol-tray", "qol-tray", TargetState::Updating),
            vec![],
        );
        host_running.running = true;
        let host = summary(&host_running);
        assert_eq!(host.label, "Updating qol-tray");
        assert_eq!(host.description, "qol restarts when the download is done");

        let plugin_running = snapshot(
            target("qol-tray", "qol-tray", TargetState::UpToDate),
            vec![
                target("ln", "Launcher", TargetState::Queued),
                target("cli", "CLI Sessions", TargetState::Updating),
            ],
        );
        let busy = summary(&plugin_running);
        assert!(busy.busy);
        assert_eq!(busy.label, "Updating plugins");
        assert_eq!(busy.description, "CLI Sessions and Launcher");
        assert_eq!(busy.action, Some(RowAction::StopUpdates));
        assert_eq!(busy.action_label, Some("Stop"));

        let last_plugin_running = snapshot(
            target("qol-tray", "qol-tray", TargetState::UpToDate),
            vec![target("cli", "CLI Sessions", TargetState::Updating)],
        );
        let finishing = summary(&last_plugin_running);
        assert_eq!(finishing.description, "CLI Sessions");
        assert_eq!(finishing.action, None);
    }

    #[test]
    fn summary_prefers_failures_then_waiting_then_everything_current() {
        let failed = snapshot(
            target("qol-tray", "qol-tray", TargetState::Failed),
            vec![target("bt", "Bluetooth", TargetState::Failed)],
        );
        let failed_summary = summary(&failed);
        assert_eq!(failed_summary.dot, Some(SummaryDot::Danger));
        assert_eq!(failed_summary.label, "2 updates failed");
        assert_eq!(
            failed_summary.description,
            "qol-tray and Bluetooth. Checked 2 minutes ago."
        );
        assert_eq!(failed_summary.action, Some(RowAction::UpdateAll));
        assert_eq!(failed_summary.action_label, Some("Retry all"));

        let waiting = snapshot(
            target("qol-tray", "qol-tray", TargetState::Available),
            vec![target("cli", "CLI Sessions", TargetState::Available)],
        );
        let waiting_summary = summary(&waiting);
        assert_eq!(waiting_summary.dot, Some(SummaryDot::Warning));
        assert_eq!(waiting_summary.label, "2 updates waiting");
        assert_eq!(
            waiting_summary.description,
            "qol-tray and CLI Sessions. Checked 2 minutes ago."
        );
        assert_eq!(waiting_summary.action, Some(RowAction::UpdateAll));
        assert_eq!(waiting_summary.action_label, Some("Update all"));

        let current = snapshot(
            target("qol-tray", "qol-tray", TargetState::UpToDate),
            vec![target("cli", "CLI Sessions", TargetState::UpToDate)],
        );
        let current_summary = summary(&current);
        assert_eq!(current_summary.dot, Some(SummaryDot::Success));
        assert_eq!(current_summary.label, "Everything is up to date");
        assert_eq!(
            current_summary.description,
            "qol-tray 1.0.0 and 1 plugin. Checked 2 minutes ago."
        );
        assert_eq!(current_summary.action, Some(RowAction::Check));
        assert_eq!(current_summary.action_label, Some("Check now"));
    }

    #[test]
    fn one_pending_row_reads_in_the_singular() {
        let failed = snapshot(
            target("qol-tray", "qol-tray", TargetState::UpToDate),
            vec![target("bt", "Bluetooth", TargetState::Failed)],
        );
        let one_failed = summary(&failed);
        assert_eq!(one_failed.label, "1 update failed");
        assert_eq!(one_failed.description, "Bluetooth. Checked 2 minutes ago.");
        assert_eq!(one_failed.action, Some(RowAction::Check));
        assert_eq!(one_failed.action_label, Some("Check now"));

        let waiting = snapshot(
            target("qol-tray", "qol-tray", TargetState::UpToDate),
            vec![target("bt", "Bluetooth", TargetState::Available)],
        );
        let one_waiting = summary(&waiting);
        assert_eq!(one_waiting.label, "1 update waiting");
        assert_eq!(one_waiting.action, Some(RowAction::Check));
        assert_eq!(one_waiting.action_label, Some("Check now"));

        let offline = UpdatesSnapshot {
            check_error: Some("Couldn't reach GitHub".to_string()),
            ..waiting.clone()
        };
        assert_eq!(summary(&offline).action_label, Some("Try again"));

        let mixed = snapshot(
            target("qol-tray", "qol-tray", TargetState::UpToDate),
            vec![
                target("bt", "Bluetooth", TargetState::Failed),
                target("cli", "CLI Sessions", TargetState::Available),
            ],
        );
        let mixed = summary(&mixed);
        assert_eq!(mixed.label, "1 update failed");
        assert_eq!(
            mixed.description,
            "Bluetooth failed, CLI Sessions waiting. Checked 2 minutes ago."
        );
        assert_eq!(mixed.action, Some(RowAction::UpdateAll));
        assert_eq!(mixed.action_label, Some("Update all"));
    }

    #[test]
    fn host_row_derives_every_state() {
        let available = host_row(&target("qol-tray", "qol-tray", TargetState::Available));
        assert_eq!(available.name, "Version");
        assert_eq!(available.value, "1.0.0 \u{2192} 1.1.0");
        assert_eq!(available.tone, ValueTone::Attention);
        assert_eq!(available.action, Some(RowAction::Update("qol-tray".into())));
        assert_eq!(
            available.description.as_deref(),
            Some("Restarts qol to finish")
        );
        assert!(available.attention);

        let queued = host_row(&target("qol-tray", "qol-tray", TargetState::Queued));
        assert_eq!(queued.value, "Waiting");
        assert_eq!(queued.tone, ValueTone::Muted);
        assert_eq!(queued.action, None);
        assert_eq!(
            queued.description.as_deref(),
            Some("Waits for the plugins, then restarts qol")
        );

        let mut updating = target("qol-tray", "qol-tray", TargetState::Updating);
        updating.progress = Some(42.4);
        let updating = host_row(&updating);
        assert_eq!(updating.value, "Downloading 42%");
        assert!(updating.spinner);
        assert_eq!(updating.tone, ValueTone::Muted);

        let mut failed = target("qol-tray", "qol-tray", TargetState::Failed);
        failed.error = Some("The download stopped".to_string());
        let failed = host_row(&failed);
        assert_eq!(failed.value, "Update failed");
        assert_eq!(failed.tone, ValueTone::Danger);
        assert_eq!(failed.description.as_deref(), Some("The download stopped"));
        assert_eq!(failed.action_label, Some("Retry"));

        let dev = host_row(&target("qol-tray", "qol-tray", TargetState::DevBuild));
        assert_eq!(dev.value, "Development build");
        assert_eq!(
            dev.description.as_deref(),
            Some("Updates come from rebuilding")
        );
        assert_eq!(dev.action, None);

        let mut current = target("qol-tray", "qol-tray", TargetState::UpToDate);
        current.updated_from = Some("1.0.0".to_string());
        let current = host_row(&current);
        assert_eq!(current.value, "1.0.0");
        assert_eq!(current.description.as_deref(), Some("Updated from 1.0.0"));
        assert_eq!(current.tone, ValueTone::Normal);

        let unchecked = host_row(&target("qol-tray", "qol-tray", TargetState::Unchecked));
        assert_eq!(unchecked.value, "1.0.0");
        assert_eq!(
            unchecked.description.as_deref(),
            Some("Not checked for updates")
        );
        assert_eq!(unchecked.action, None);
    }

    #[test]
    fn summary_folds_the_last_check_into_one_row_with_one_button() {
        let mut disabled = snapshot(
            target("qol-tray", "qol-tray", TargetState::DevBuild),
            vec![target("cli", "CLI Sessions", TargetState::Unchecked)],
        );
        disabled.checks_enabled = false;
        disabled.last_checked_secs = None;
        disabled.check_error = Some("No connection to GitHub".to_string());
        let row = summary(&disabled);
        assert_eq!(row.label, "Couldn't check for updates");
        assert_eq!(row.action, Some(RowAction::Check));
        assert_eq!(row.action_label, Some("Try again"));

        disabled.check_error = None;
        disabled.plugins = vec![
            target("cli", "CLI Sessions", TargetState::Available),
            target("lights", "Lights", TargetState::Available),
        ];
        let row = summary(&disabled);
        assert_eq!(row.label, "2 updates waiting");
        assert_eq!(row.description, "CLI Sessions and Lights.");
        assert_eq!(row.action, Some(RowAction::UpdateAll));
        assert_eq!(row.action_label, Some("Update all"));

        disabled.plugins = vec![target("cli", "CLI Sessions", TargetState::UpToDate)];
        let row = summary(&disabled);
        assert_eq!(row.label, "Plugins are up to date");
        assert_eq!(row.description, "1 plugin.");
        assert_eq!(row.action, Some(RowAction::Check));

        let failed = UpdatesSnapshot {
            check_error: Some("Couldn't reach GitHub".to_string()),
            last_success_secs: Some(3 * 86_400),
            ..snapshot(
                target("qol-tray", "qol-tray", TargetState::Unchecked),
                vec![target("cli", "CLI Sessions", TargetState::Unchecked)],
            )
        };
        let row = summary(&failed);
        assert_eq!(row.label, "Couldn't check for updates");
        assert_eq!(
            row.description,
            "Couldn't reach GitHub. Last good check 3 days ago."
        );
        assert_eq!(row.action, Some(RowAction::Check));
        assert_eq!(row.action_label, Some("Try again"));

        let never_succeeded = UpdatesSnapshot {
            last_success_secs: None,
            ..failed.clone()
        };
        assert_eq!(
            summary(&never_succeeded).description,
            "Couldn't reach GitHub."
        );

        let stale_error_all_current = UpdatesSnapshot {
            check_error: Some("Couldn't reach GitHub".to_string()),
            ..snapshot(
                target("qol-tray", "qol-tray", TargetState::UpToDate),
                vec![],
            )
        };
        let row = summary(&stale_error_all_current);
        assert_eq!(row.label, "Couldn't check for updates");
        assert_eq!(row.action_label, Some("Try again"));

        let never = UpdatesSnapshot {
            last_checked_secs: None,
            last_success_secs: None,
            ..snapshot(
                target("qol-tray", "qol-tray", TargetState::Unchecked),
                vec![],
            )
        };
        let row = summary(&never);
        assert_eq!(row.label, "Not checked yet");
        assert_eq!(
            row.description,
            "qol checks for updates by itself every 5 hours."
        );
        assert_eq!(row.action_label, Some("Check now"));

        let rechecking = UpdatesSnapshot {
            checking: true,
            ..snapshot(
                target("qol-tray", "qol-tray", TargetState::UpToDate),
                vec![],
            )
        };
        let row = summary(&rechecking);
        assert!(row.busy);
        assert_eq!(row.label, "Checking for updates");
        assert_eq!(row.action, None);

        let pending_while_checking = UpdatesSnapshot {
            checking: true,
            ..snapshot(
                target("qol-tray", "qol-tray", TargetState::UpToDate),
                vec![target("cli", "CLI Sessions", TargetState::Available)],
            )
        };
        let row = summary(&pending_while_checking);
        assert!(row.busy);
        assert_eq!(row.dot, None);
        assert_eq!(row.label, "1 update waiting");
    }

    #[test]
    fn page_holds_the_groups_and_the_fold_count() {
        let snapshot = snapshot(
            target("qol-tray", "qol-tray", TargetState::Available),
            vec![
                target("bt", "Bluetooth", TargetState::UpToDate),
                target("cli", "CLI Sessions", TargetState::Available),
            ],
        );
        let model = page(&snapshot, &BTreeSet::new());
        assert_eq!(model.plugin_total, 2);
        assert_eq!(model.plugins.len(), 1);
        assert_eq!(model.fold, Some("The other 1 is up to date.".to_string()));
        assert_eq!(model.host.value, "1.0.0 \u{2192} 1.1.0");
        assert_eq!(model.summary.label, "2 updates waiting");
    }
}
