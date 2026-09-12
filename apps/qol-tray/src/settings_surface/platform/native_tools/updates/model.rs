use std::collections::BTreeSet;

use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum TargetState {
    UpToDate,
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
pub(super) struct CheckRow {
    pub(super) label: &'static str,
    pub(super) value: String,
    pub(super) tone: ValueTone,
    pub(super) description: String,
    pub(super) action: Option<RowAction>,
    pub(super) action_label: Option<&'static str>,
    pub(super) checking: bool,
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
    pub(super) check: CheckRow,
    pub(super) host: TargetRow,
    pub(super) plugins: Vec<TargetRow>,
    pub(super) fold: Option<String>,
    pub(super) plugin_total: usize,
}

pub(super) fn page(snapshot: &UpdatesSnapshot, visited: &BTreeSet<String>) -> PageModel {
    let plugins = plugin_rows(&snapshot.plugins, visited);
    PageModel {
        summary: summary(snapshot),
        check: check_row(snapshot),
        host: host_row(&snapshot.host),
        fold: fold_text(snapshot.plugins.len(), plugins.len()),
        plugin_total: snapshot.plugins.len(),
        plugins,
    }
}

pub(super) fn plugin_rows(plugins: &[UpdateTarget], visited: &BTreeSet<String>) -> Vec<TargetRow> {
    let mut rows = plugins
        .iter()
        .filter_map(|plugin| match plugin.state {
            TargetState::UpToDate | TargetState::DevLinked | TargetState::DevBuild => visited
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
            row.description = Some("Restarts qol-tray to finish".to_string());
            row.action = Some(RowAction::Update(host.id.clone()));
            row.action_label = Some("Update");
            row.attention = true;
        }
        TargetState::Queued => {
            row.value = "Waiting".to_string();
            row.tone = ValueTone::Muted;
            row.description = Some("Waits for the plugins, then restarts qol-tray".to_string());
        }
        TargetState::Updating => {
            row.value = download_value(host.progress);
            row.tone = ValueTone::Muted;
            row.description = Some("Restarts qol-tray to finish".to_string());
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
            row.description = Some("Updates arrive through Recompile".to_string());
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
        let host_updating = snapshot.host.state == TargetState::Updating;
        let description = if host_updating {
            "qol-tray restarts when the download is done".to_string()
        } else if snapshot.host.state == TargetState::Queued {
            "qol-tray goes last, then restarts.".to_string()
        } else {
            pending_names(snapshot, &[TargetState::Queued, TargetState::Updating])
        };
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
            action: None,
            action_label: None,
        };
    }
    let failed = pending_names(snapshot, &[TargetState::Failed]);
    if !failed.is_empty() {
        let count = pending_count(snapshot, TargetState::Failed);
        return Summary {
            dot: Some(SummaryDot::Danger),
            busy: false,
            label: update_count_label(count, "update failed", "updates failed"),
            description: failed,
            action: Some(RowAction::UpdateAll),
            action_label: Some("Retry"),
        };
    }
    let available = pending_names(snapshot, &[TargetState::Available]);
    if !available.is_empty() {
        let count = pending_count(snapshot, TargetState::Available);
        return Summary {
            dot: Some(SummaryDot::Warning),
            busy: false,
            label: update_count_label(count, "update waiting", "updates waiting"),
            description: available,
            action: Some(RowAction::UpdateAll),
            action_label: Some("Update all"),
        };
    }
    Summary {
        dot: Some(SummaryDot::Success),
        busy: false,
        label: "Everything is up to date".to_string(),
        description: format!(
            "qol-tray {} and {} {}",
            snapshot.host.current,
            snapshot.plugins.len(),
            plural(snapshot.plugins.len(), "plugin")
        ),
        action: None,
        action_label: None,
    }
}

pub(super) fn check_row(snapshot: &UpdatesSnapshot) -> CheckRow {
    if !snapshot.checks_enabled {
        return CheckRow {
            label: "Update checks",
            value: "Off".to_string(),
            tone: ValueTone::Muted,
            description: "Development builds do not check for updates.".to_string(),
            action: None,
            action_label: None,
            checking: false,
        };
    }
    match &snapshot.check_error {
        Some(error) => CheckRow {
            label: "Last checked",
            value: "Couldn't check".to_string(),
            tone: ValueTone::Danger,
            description: match snapshot.last_success_secs {
                Some(secs) => format!("{error}. Last good check {} ago.", long_age(secs)),
                None => format!("{error}."),
            },
            action: Some(RowAction::Check),
            action_label: Some("Try again"),
            checking: snapshot.checking,
        },
        None => CheckRow {
            label: "Last checked",
            value: age_text(snapshot.last_checked_secs),
            tone: ValueTone::Normal,
            description: "qol-tray checks on its own every 5 hours".to_string(),
            action: Some(RowAction::Check),
            action_label: Some("Check now"),
            checking: snapshot.checking,
        },
    }
}

pub(super) fn fold_text(total: usize, shown: usize) -> Option<String> {
    let hidden = total.saturating_sub(shown);
    if hidden == 0 {
        return None;
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

pub(super) fn age_text(secs: Option<u64>) -> String {
    let Some(secs) = secs else {
        return "Never".to_string();
    };
    if secs < 60 {
        "Just now".to_string()
    } else if secs < 3600 {
        format!("{} min ago", secs / 60)
    } else if secs < 2 * 86_400 {
        format!("{} h ago", secs / 3600)
    } else {
        let days = secs / 86_400;
        format!("{days} {} ago", plural(days as usize, "day"))
    }
}

pub(super) fn long_age(secs: u64) -> String {
    if secs < 60 {
        "less than a minute".to_string()
    } else if secs < 3600 {
        let minutes = secs / 60;
        format!("{minutes} {}", plural(minutes as usize, "minute"))
    } else if secs < 2 * 86_400 {
        let hours = secs / 3600;
        format!("{hours} {}", plural(hours as usize, "hour"))
    } else {
        let days = secs / 86_400;
        format!("{days} {}", plural(days as usize, "day"))
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

fn update_count_label(count: usize, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("1 {singular}")
    } else {
        format!("{count} {plural}")
    }
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

fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        noun.to_string()
    } else {
        format!("{noun}s")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{
        age_text, check_row, fold_text, host_row, join_names, long_age, page, plugin_rows, summary,
        RowAction, SummaryDot, TargetState, UpdateTarget, UpdatesSnapshot, ValueTone,
    };

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
            running: false,
            host,
            plugins,
        }
    }

    #[test]
    fn age_text_follows_the_display_tiers() {
        let cases = [
            (None, "Never"),
            (Some(0), "Just now"),
            (Some(59), "Just now"),
            (Some(60), "1 min ago"),
            (Some(719), "11 min ago"),
            (Some(3600), "1 h ago"),
            (Some(3 * 3600), "3 h ago"),
            (Some(47 * 3600), "47 h ago"),
            (Some(48 * 3600), "2 days ago"),
            (Some(86_400), "24 h ago"),
            (Some(4 * 86_400), "4 days ago"),
        ];
        for (secs, expected) in cases {
            assert_eq!(age_text(secs), expected, "secs={secs:?}");
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
        assert_eq!(fold_text(0, 0), None::<String>);
        assert_eq!(fold_text(3, 3), None::<String>);
        assert_eq!(
            fold_text(1, 0),
            Some("Your plugin is up to date.".to_string())
        );
        assert_eq!(
            fold_text(9, 1),
            Some("The other 8 are up to date.".to_string())
        );
        assert_eq!(
            fold_text(2, 1),
            Some("The other 1 is up to date.".to_string())
        );
        assert_eq!(fold_text(2, 0), Some("All 2 are up to date.".to_string()));
        assert_eq!(fold_text(17, 0), Some("All 17 are up to date.".to_string()));
        assert_eq!(
            fold_text(17, 2),
            Some("The other 15 are up to date.".to_string())
        );
    }

    #[test]
    fn plugin_rows_keep_only_work_and_visited_finishes_in_name_order() {
        let plugins = vec![
            target("launcher", "Launcher", TargetState::UpToDate),
            target("bluetooth", "Bluetooth", TargetState::Failed),
            target("cli", "CLI Sessions", TargetState::Available),
            target("ides", "IDE Checkout", TargetState::DevLinked),
            target("lights", "Lights", TargetState::UpToDate),
        ];
        let visited = BTreeSet::from(["launcher".to_string()]);
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
        let plugins = vec![target("ides", "IDE Checkout", TargetState::DevLinked)];
        assert!(plugin_rows(&plugins, &BTreeSet::new()).is_empty());
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
        assert_eq!(busy.label, "Updating plugins");
        assert_eq!(busy.description, "qol-tray goes last, then restarts.");
        assert_eq!(busy.action, None);

        let mut host_running = snapshot(
            target("qol-tray", "qol-tray", TargetState::Updating),
            vec![],
        );
        host_running.running = true;
        let host = summary(&host_running);
        assert_eq!(host.label, "Updating qol-tray");
        assert_eq!(
            host.description,
            "qol-tray restarts when the download is done"
        );

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
        assert_eq!(failed_summary.description, "qol-tray and Bluetooth");
        assert_eq!(failed_summary.action_label, Some("Retry"));

        let waiting = snapshot(
            target("qol-tray", "qol-tray", TargetState::Available),
            vec![target("cli", "CLI Sessions", TargetState::Available)],
        );
        let waiting_summary = summary(&waiting);
        assert_eq!(waiting_summary.dot, Some(SummaryDot::Warning));
        assert_eq!(waiting_summary.label, "2 updates waiting");
        assert_eq!(waiting_summary.description, "qol-tray and CLI Sessions");
        assert_eq!(waiting_summary.action, Some(RowAction::UpdateAll));
        assert_eq!(waiting_summary.action_label, Some("Update all"));

        let current = snapshot(
            target("qol-tray", "qol-tray", TargetState::UpToDate),
            vec![target("cli", "CLI Sessions", TargetState::UpToDate)],
        );
        let current_summary = summary(&current);
        assert_eq!(current_summary.dot, Some(SummaryDot::Success));
        assert_eq!(current_summary.label, "Everything is up to date");
        assert_eq!(current_summary.description, "qol-tray 1.0.0 and 1 plugin");
        assert_eq!(current_summary.action, None);
    }

    #[test]
    fn one_pending_row_reads_in_the_singular() {
        let failed = snapshot(
            target("qol-tray", "qol-tray", TargetState::UpToDate),
            vec![target("bt", "Bluetooth", TargetState::Failed)],
        );
        assert_eq!(summary(&failed).label, "1 update failed");

        let waiting = snapshot(
            target("qol-tray", "qol-tray", TargetState::UpToDate),
            vec![target("bt", "Bluetooth", TargetState::Available)],
        );
        assert_eq!(summary(&waiting).label, "1 update waiting");
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
            Some("Restarts qol-tray to finish")
        );
        assert!(available.attention);

        let queued = host_row(&target("qol-tray", "qol-tray", TargetState::Queued));
        assert_eq!(queued.value, "Waiting");
        assert_eq!(queued.tone, ValueTone::Muted);
        assert_eq!(queued.action, None);
        assert_eq!(
            queued.description.as_deref(),
            Some("Waits for the plugins, then restarts qol-tray")
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
            Some("Updates arrive through Recompile")
        );
        assert_eq!(dev.action, None);

        let mut current = target("qol-tray", "qol-tray", TargetState::UpToDate);
        current.updated_from = Some("1.0.0".to_string());
        let current = host_row(&current);
        assert_eq!(current.value, "1.0.0");
        assert_eq!(current.description.as_deref(), Some("Updated from 1.0.0"));
        assert_eq!(current.tone, ValueTone::Normal);
    }

    #[test]
    fn check_row_covers_disabled_error_and_fresh_checks() {
        let mut disabled = snapshot(
            target("qol-tray", "qol-tray", TargetState::UpToDate),
            vec![],
        );
        disabled.checks_enabled = false;
        let row = check_row(&disabled);
        assert_eq!(row.label, "Update checks");
        assert_eq!(row.value, "Off");
        assert_eq!(row.tone, ValueTone::Muted);
        assert_eq!(
            row.description,
            "Development builds do not check for updates."
        );
        assert_eq!(row.action, None);

        let failed = snapshot(
            target("qol-tray", "qol-tray", TargetState::UpToDate),
            vec![],
        );
        let failed = UpdatesSnapshot {
            check_error: Some("No connection to GitHub".to_string()),
            last_success_secs: Some(21_600),
            ..failed
        };
        let row = check_row(&failed);
        assert_eq!(row.label, "Last checked");
        assert_eq!(row.value, "Couldn't check");
        assert_eq!(row.tone, ValueTone::Danger);
        assert_eq!(
            row.description,
            "No connection to GitHub. Last good check 6 hours ago."
        );
        assert_eq!(row.action, Some(RowAction::Check));
        assert_eq!(row.action_label, Some("Try again"));

        let never = snapshot(
            target("qol-tray", "qol-tray", TargetState::UpToDate),
            vec![],
        );
        let never = UpdatesSnapshot {
            check_error: Some("No connection to GitHub".to_string()),
            last_success_secs: None,
            ..never
        };
        assert_eq!(check_row(&never).description, "No connection to GitHub.");

        let fresh = snapshot(
            target("qol-tray", "qol-tray", TargetState::UpToDate),
            vec![],
        );
        let row = check_row(&fresh);
        assert_eq!(row.value, "2 min ago");
        assert_eq!(row.description, "qol-tray checks on its own every 5 hours");
        assert_eq!(row.action_label, Some("Check now"));
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
