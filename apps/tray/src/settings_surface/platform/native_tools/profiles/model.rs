use qol_gpui::settings_panel::SettingsValueTone;

use super::super::updates::model::long_age;
use super::data::{Backup, GitHubConnect, Health, Profile, Snapshot, SyncStatus};

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Level {
    Main,
    Profiles,
    Backups,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Action {
    OpenProfiles,
    Use(String),
    NewProfile,
    Sync,
    Connect,
    OpenGitHub,
    Auto(bool),
    Disconnect,
    OpenBackups,
    OpenBackup(String),
    Export,
    Import,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Dot {
    Idle,
    Success,
    Warning,
    Danger,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Value {
    None,
    Text(String, SettingsValueTone),
    Choice {
        word: String,
        art: String,
        chevron: bool,
    },
    Open(String),
    Toggle(bool),
    Chip,
    Name,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Row {
    pub(super) label: String,
    pub(super) detail: String,
    pub(super) header: bool,
    pub(super) dot: Option<Dot>,
    pub(super) spinner: bool,
    pub(super) value: Value,
    pub(super) verb: Option<&'static str>,
    pub(super) chip_when_selected: bool,
    pub(super) action: Option<Action>,
}

impl Row {
    fn new(label: impl Into<String>, detail: impl Into<String>, action: Option<Action>) -> Self {
        Self {
            label: label.into(),
            detail: detail.into(),
            header: false,
            dot: None,
            spinner: false,
            value: Value::None,
            verb: None,
            chip_when_selected: false,
            action,
        }
    }

    fn header(title: &str, colophon: &str) -> Self {
        Self {
            header: true,
            ..Self::new(title, colophon, None)
        }
    }

    fn value(mut self, value: Value) -> Self {
        self.value = value;
        self
    }

    fn verb(mut self, verb: &'static str) -> Self {
        self.verb = Some(verb);
        self
    }

    fn chip(self, verb: &'static str) -> Self {
        self.value(Value::Chip).verb(verb)
    }

    fn dot(mut self, dot: Dot) -> Self {
        self.dot = Some(dot);
        self
    }

    fn spinner(mut self) -> Self {
        self.spinner = true;
        self
    }

    fn when_selected(mut self) -> Self {
        self.chip_when_selected = true;
        self
    }
}

pub(super) struct Page<'a> {
    pub(super) snapshot: &'a Snapshot,
    pub(super) syncing: bool,
    pub(super) note: Option<&'a str>,
    pub(super) naming: bool,
    pub(super) now_secs: i64,
}

pub(super) fn main_rows(page: &Page<'_>) -> Vec<Row> {
    let snapshot = page.snapshot;
    let sync = &snapshot.sync;
    let mut rows = vec![Row::header(
        "profile",
        "the settings this computer runs with",
    )];
    let arts = letter_arts(&snapshot.profiles);
    let active = snapshot.profiles.iter().position(|profile| profile.active);
    let (word, art) = match active {
        Some(index) => (snapshot.profiles[index].name.clone(), arts[index].clone()),
        None => (String::new(), "letters:?".to_string()),
    };
    rows.push(
        Row::new(
            "Profile",
            page.note.unwrap_or_default(),
            Some(Action::OpenProfiles),
        )
        .value(Value::Choice {
            word,
            art,
            chevron: true,
        })
        .verb("open"),
    );
    rows.push(Row::header("sync", "keep your profiles on every computer"));
    rows.push(status_row(sync, page.syncing, page.now_secs));
    if sync.configured {
        let on = sync.pull_on_launch && sync.push_on_change;
        rows.push(
            Row::new(
                "Auto sync",
                "Sync when qol starts and after every change",
                Some(Action::Auto(!on)),
            )
            .value(Value::Toggle(on))
            .verb(if on { "turn off" } else { "turn on" }),
        );
        rows.push(
            Row::new(
                "Stop syncing",
                "Your setup stays here and on GitHub",
                Some(Action::Disconnect),
            )
            .chip("disconnect"),
        );
    }
    rows.push(Row::header("this computer", "stays here"));
    rows.push(backups_row(sync));
    rows.push(
        Row::new(
            "Export to a file",
            "Every setting, hotkey and plugin in one file",
            Some(Action::Export),
        )
        .chip("export"),
    );
    rows.push(
        Row::new(
            "Import from a file",
            "Replace this setup with a saved one",
            Some(Action::Import),
        )
        .chip("import"),
    );
    rows
}

pub(super) fn profile_rows(page: &Page<'_>) -> Vec<Row> {
    let profiles = &page.snapshot.profiles;
    let arts = letter_arts(profiles);
    let mut rows = vec![Row::header("profiles", "one set of settings each")];
    for (profile, art) in profiles.iter().zip(arts) {
        let plugins = plugins_text(profile.plugins);
        let row = if profile.active {
            Row::new(
                &profile.name,
                format!("In use on this computer · {plugins}"),
                None,
            )
        } else {
            Row::new(
                &profile.name,
                plugins,
                Some(Action::Use(profile.name.clone())),
            )
            .verb("use here")
            .when_selected()
        };
        rows.push(row.value(Value::Choice {
            word: if profile.active {
                "in use here".to_string()
            } else {
                String::new()
            },
            art,
            chevron: false,
        }));
    }
    let active = profiles
        .iter()
        .find(|profile| profile.active)
        .map(|profile| profile.name.as_str())
        .unwrap_or("default");
    rows.push(if page.naming {
        Row::new(
            "New profile",
            format!("Type a name, then Enter. It starts as a copy of {active}."),
            Some(Action::NewProfile),
        )
        .value(Value::Name)
        .verb("create")
    } else {
        Row::new(
            "New profile",
            "A new set of settings",
            Some(Action::NewProfile),
        )
        .chip("new")
        .when_selected()
    });
    rows
}

pub(super) fn backup_rows(backups: Option<&[Backup]>) -> Vec<Row> {
    let Some(backups) = backups else {
        return vec![Row::header("backups", "reading the list")];
    };
    if backups.is_empty() {
        return vec![
            Row::header("backups", "saved before sync replaces anything"),
            Row::new(
                "No backups yet",
                "One is saved before sync replaces anything",
                None,
            ),
        ];
    }
    let mut rows = vec![Row::header(
        "backups",
        "saved before sync replaced something",
    )];
    rows.extend(backups.iter().map(|backup| {
        Row::new(
            backup_when(&backup.file_name).unwrap_or_else(|| backup.file_name.clone()),
            backup_kind(&backup.file_name),
            Some(Action::OpenBackup(backup.file_name.clone())),
        )
        .value(Value::Text(
            qol_gpui::format::format_bytes(backup.size_bytes),
            SettingsValueTone::Muted,
        ))
        .verb("open")
    }));
    rows
}

fn status_row(sync: &SyncStatus, syncing: bool, now: i64) -> Row {
    if syncing {
        return Row::new("Syncing", repo_text(sync), None).spinner();
    }
    match &sync.github_connect {
        GitHubConnect::Waiting {
            user_code,
            verification_uri,
        } => {
            return Row::new(
                "Waiting for GitHub",
                format!(
                    "Enter {user_code} at {}. The code is copied.",
                    strip_scheme(verification_uri)
                ),
                Some(Action::OpenGitHub),
            )
            .dot(Dot::Warning)
            .chip("open github")
        }
        GitHubConnect::Connecting => {
            return Row::new(
                "Connecting to GitHub",
                "Bringing your profiles to this computer",
                None,
            )
            .spinner()
        }
        GitHubConnect::Idle | GitHubConnect::Failed { .. } => {}
    }
    if !sync.configured {
        return match &sync.github_connect {
            GitHubConnect::Failed { message } => {
                Row::new("Not syncing", message.clone(), Some(Action::Connect)).dot(Dot::Danger)
            }
            _ => Row::new(
                "Not syncing",
                "Sign in to GitHub to keep this setup on every computer",
                Some(Action::Connect),
            )
            .dot(Dot::Idle),
        }
        .chip("connect");
    }
    match sync.health {
        Health::Error => Row::new(
            "Sync failed",
            sync.last_error
                .clone()
                .unwrap_or_else(|| "The last sync stopped with an error".to_string()),
            Some(Action::Sync),
        )
        .dot(Dot::Danger)
        .chip("sync"),
        Health::Attention => Row::new(
            "Sync needs a look",
            sync.incident
                .as_ref()
                .map(|incident| incident.message.clone())
                .unwrap_or_else(|| "Your changes are kept as the newest backup".to_string()),
            Some(Action::OpenBackups),
        )
        .dot(Dot::Warning)
        .chip("backups"),
        Health::Healthy | Health::NotConfigured => {
            Row::new(synced_label(sync, now), repo_text(sync), Some(Action::Sync))
                .dot(Dot::Success)
                .chip("sync")
        }
    }
}

fn backups_row(sync: &SyncStatus) -> Row {
    if sync.backup_count == 0 {
        return Row::new(
            "Backups",
            "None yet. One is saved before sync replaces anything.",
            None,
        )
        .value(Value::Text("0".to_string(), SettingsValueTone::Muted));
    }
    let newest = sync
        .latest_backup_file
        .as_deref()
        .and_then(backup_day)
        .map(|day| format!(" · newest {day}"))
        .unwrap_or_default();
    Row::new(
        "Backups",
        format!("Saved before sync replaced anything{newest}"),
        Some(Action::OpenBackups),
    )
    .value(Value::Open(sync.backup_count.to_string()))
    .verb("open")
}

fn synced_label(sync: &SyncStatus, now: i64) -> String {
    let Some(at) = sync
        .last_sync_at
        .as_deref()
        .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
    else {
        return "Synced".to_string();
    };
    let secs = now.saturating_sub(at.timestamp()).max(0) as u64;
    if secs < 60 {
        "Synced just now".to_string()
    } else {
        format!("Synced {} ago", long_age(secs))
    }
}

fn repo_text(sync: &SyncStatus) -> String {
    sync.repo_url
        .as_deref()
        .map(|url| strip_scheme(url.trim_end_matches(".git")).to_string())
        .unwrap_or_default()
}

fn strip_scheme(url: &str) -> &str {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
}

pub(super) fn letter_arts(profiles: &[Profile]) -> Vec<String> {
    let names = profiles
        .iter()
        .map(|profile| profile.name.as_str())
        .collect::<Vec<_>>();
    qol_gpui::pictures::letters_for(&names)
        .into_iter()
        .map(|letters| format!("letters:{letters}"))
        .collect()
}

fn plugins_text(count: usize) -> String {
    match count {
        0 => "Nothing set up yet".to_string(),
        1 => "1 plugin set up".to_string(),
        count => format!("{count} plugins set up"),
    }
}

fn backup_parts(file_name: &str) -> Option<(usize, u32, &str, &str, &str)> {
    let (date, rest) = file_name.split_once('-')?;
    let (time, _) = rest.split_once('-')?;
    if date.len() != 8 || time.len() != 6 {
        return None;
    }
    let month = date.get(4..6)?.parse::<usize>().ok()?.checked_sub(1)?;
    let day = date.get(6..8)?.parse::<u32>().ok()?;
    MONTHS.get(month)?;
    Some((
        month,
        day,
        date.get(0..4)?,
        time.get(0..2)?,
        time.get(2..4)?,
    ))
}

fn backup_day(file_name: &str) -> Option<String> {
    let (month, day, _, _, _) = backup_parts(file_name)?;
    Some(format!("{day} {}", MONTHS[month]))
}

fn backup_when(file_name: &str) -> Option<String> {
    let (month, day, year, hour, minute) = backup_parts(file_name)?;
    Some(format!("{day} {} {year}, {hour}:{minute}", MONTHS[month]))
}

fn backup_kind(file_name: &str) -> &'static str {
    if file_name.ends_with("-conflict.json") {
        "Saved when both sides had changes"
    } else {
        "Saved before sync replaced it"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_surface::platform::native_tools::profiles::data::Incident;

    fn profile(name: &str, active: bool, plugins: usize) -> Profile {
        Profile {
            name: name.to_string(),
            active,
            plugins,
        }
    }

    fn sync(configured: bool, health: Health) -> SyncStatus {
        SyncStatus {
            configured,
            repo_url: configured
                .then(|| "https://github.com/KMRH47/qol-tray-profiles.git".to_string()),
            health,
            pull_on_launch: true,
            push_on_change: true,
            has_github_token: configured,
            last_sync_at: Some("2026-10-05T10:00:00+00:00".to_string()),
            incident: None,
            last_error: None,
            backup_count: 9,
            latest_backup_file: Some("20260811-103430-conflict.json".to_string()),
            github_connect: GitHubConnect::Idle,
        }
    }

    fn snapshot(configured: bool) -> Snapshot {
        Snapshot {
            profiles: vec![profile("default", true, 11), profile("work", false, 1)],
            sync: sync(
                configured,
                if configured {
                    Health::Healthy
                } else {
                    Health::NotConfigured
                },
            ),
        }
    }

    fn page(snapshot: &Snapshot) -> Page<'_> {
        Page {
            snapshot,
            syncing: false,
            note: None,
            naming: false,
            now_secs: chrono::DateTime::parse_from_rfc3339("2026-10-05T10:04:00+00:00")
                .unwrap()
                .timestamp(),
        }
    }

    fn labels(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|row| row.label.as_str()).collect()
    }

    #[test]
    fn a_synced_page_reads_profile_then_sync_then_this_computer() {
        let snapshot = snapshot(true);
        let rows = main_rows(&page(&snapshot));
        assert_eq!(
            labels(&rows),
            [
                "profile",
                "Profile",
                "sync",
                "Synced 4 minutes ago",
                "Auto sync",
                "Stop syncing",
                "this computer",
                "Backups",
                "Export to a file",
                "Import from a file",
            ]
        );
        assert_eq!(rows[3].detail, "github.com/KMRH47/qol-tray-profiles");
        assert_eq!(rows[3].action, Some(Action::Sync));
        assert_eq!(rows[4].value, Value::Toggle(true));
        assert_eq!(rows[4].action, Some(Action::Auto(false)));
        assert_eq!(
            rows[7].detail,
            "Saved before sync replaced anything · newest 11 Aug"
        );
        assert_eq!(rows[7].value, Value::Open("9".to_string()));
    }

    #[test]
    fn the_profile_row_shows_the_profile_in_use_with_its_letter() {
        let snapshot = snapshot(true);
        let rows = main_rows(&page(&snapshot));
        assert_eq!(
            rows[1].value,
            Value::Choice {
                word: "default".to_string(),
                art: "letters:D".to_string(),
                chevron: true,
            }
        );
        assert_eq!(rows[1].action, Some(Action::OpenProfiles));
    }

    #[test]
    fn an_unsynced_page_offers_connect_and_hides_the_sync_switches() {
        let snapshot = snapshot(false);
        let rows = main_rows(&page(&snapshot));
        assert!(!labels(&rows).contains(&"Auto sync"));
        assert!(!labels(&rows).contains(&"Stop syncing"));
        assert_eq!(rows[3].label, "Not syncing");
        assert_eq!(rows[3].action, Some(Action::Connect));
        assert_eq!(rows[3].verb, Some("connect"));
    }

    #[test]
    fn a_pending_sign_in_shows_the_code_and_opens_github() {
        let mut snapshot = snapshot(false);
        snapshot.sync.github_connect = GitHubConnect::Waiting {
            user_code: "WDJB-MJHT".to_string(),
            verification_uri: "https://github.com/login/device".to_string(),
        };
        let rows = main_rows(&page(&snapshot));
        assert_eq!(rows[3].label, "Waiting for GitHub");
        assert_eq!(
            rows[3].detail,
            "Enter WDJB-MJHT at github.com/login/device. The code is copied."
        );
        assert_eq!(rows[3].action, Some(Action::OpenGitHub));
        snapshot.sync.github_connect = GitHubConnect::Connecting;
        let rows = main_rows(&page(&snapshot));
        assert_eq!(rows[3].label, "Connecting to GitHub");
        assert!(rows[3].spinner);
    }

    #[test]
    fn a_failed_sign_in_says_why_and_offers_connect_again() {
        let mut snapshot = snapshot(false);
        snapshot.sync.github_connect = GitHubConnect::Failed {
            message: "The code expired.".to_string(),
        };
        let rows = main_rows(&page(&snapshot));
        assert_eq!(rows[3].label, "Not syncing");
        assert_eq!(rows[3].detail, "The code expired.");
        assert_eq!(rows[3].dot, Some(Dot::Danger));
        assert_eq!(rows[3].action, Some(Action::Connect));
    }

    #[test]
    fn sync_trouble_turns_the_status_row_into_the_next_step() {
        let mut snapshot = snapshot(true);
        snapshot.sync.health = Health::Error;
        snapshot.sync.last_error = Some("GitHub refused the push".to_string());
        let rows = main_rows(&page(&snapshot));
        assert_eq!(rows[3].label, "Sync failed");
        assert_eq!(rows[3].dot, Some(Dot::Danger));
        assert_eq!(rows[3].action, Some(Action::Sync));
        snapshot.sync.health = Health::Attention;
        snapshot.sync.incident = Some(Incident {
            message: "Another computer replaced your changes".to_string(),
        });
        let rows = main_rows(&page(&snapshot));
        assert_eq!(rows[3].label, "Sync needs a look");
        assert_eq!(rows[3].action, Some(Action::OpenBackups));
    }

    #[test]
    fn the_list_marks_the_profile_in_use_and_switches_to_the_others() {
        let snapshot = snapshot(true);
        let rows = profile_rows(&page(&snapshot));
        assert_eq!(
            labels(&rows),
            ["profiles", "default", "work", "New profile"]
        );
        assert_eq!(
            rows[1].detail,
            "In use on this computer · 11 plugins set up"
        );
        assert_eq!(rows[1].action, None);
        assert_eq!(rows[2].detail, "1 plugin set up");
        assert_eq!(rows[2].action, Some(Action::Use("work".to_string())));
        assert_eq!(rows[2].verb, Some("use here"));
        assert_eq!(rows[3].value, Value::Chip);
    }

    #[test]
    fn naming_turns_new_profile_into_a_field_that_copies_the_profile_in_use() {
        let snapshot = snapshot(true);
        let mut page = page(&snapshot);
        page.naming = true;
        let rows = profile_rows(&page);
        let last = rows.last().unwrap();
        assert_eq!(last.value, Value::Name);
        assert_eq!(
            last.detail,
            "Type a name, then Enter. It starts as a copy of default."
        );
    }

    #[test]
    fn backups_read_their_date_and_why_they_were_saved() {
        let rows = backup_rows(Some(&[
            Backup {
                file_name: "20260811-103430-conflict.json".to_string(),
                size_bytes: 97_280,
            },
            Backup {
                file_name: "20260426-075742-pushed.json".to_string(),
                size_bytes: 7_900,
            },
        ]));
        assert_eq!(
            labels(&rows),
            ["backups", "11 Aug 2026, 10:34", "26 Apr 2026, 07:57"]
        );
        assert_eq!(rows[1].detail, "Saved when both sides had changes");
        assert_eq!(rows[2].detail, "Saved before sync replaced it");
        assert_eq!(
            rows[1].action,
            Some(Action::OpenBackup(
                "20260811-103430-conflict.json".to_string()
            ))
        );
    }
}
