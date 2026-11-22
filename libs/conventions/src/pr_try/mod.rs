use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const ROOT_DIR: &str = "pr-try";
pub const RECORD_FILE: &str = "state.json";
pub const KEEP_TRAY_FLAG: &str = "--keep-tray";
pub const BUILDS_QUERY: &str = "pull_request_builds";
pub const TRY_ACTION: &str = "try_pull_request";
pub const TRY_WITH_TRAY_ACTION: &str = "try_pull_request_with_tray";
pub const END_ACTION: &str = "end_pull_request_try";
pub const DISMISS_ACTION: &str = "dismiss_pull_request_try";
pub const CARRIES_TRAY_EXIT_CODE: i32 = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListedBuild {
    pub number: u64,
    pub title: String,
    pub head: String,
}

pub fn record_path(data_dir: &Path) -> PathBuf {
    data_dir.join(ROOT_DIR).join(RECORD_FILE)
}

pub fn short_sha(sha: &str) -> &str {
    sha.get(..9).unwrap_or(sha)
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TryRecord {
    pub pull_request: Option<u64>,
    pub head_sha: Option<String>,
    pub plugins: Vec<SwappedPlugin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tray: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_tray: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SwappedPlugin {
    pub id: String,
    pub target: PathBuf,
    pub backup: PathBuf,
}

impl TryRecord {
    pub fn is_active(&self) -> bool {
        self.pull_request.is_some() && (self.tray.is_some() || !self.plugins.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::{record_path, short_sha, ListedBuild, SwappedPlugin, TryRecord};
    use std::path::{Path, PathBuf};

    #[test]
    fn a_listed_build_names_number_title_and_head() {
        let build = ListedBuild {
            number: 621,
            title: "Load a build".into(),
            head: "d3b28323f".into(),
        };
        let text = r#"[{"number":621,"title":"Load a build","head":"d3b28323f"}]"#;
        assert_eq!(serde_json::to_string(&[&build]).unwrap(), text);
        assert_eq!(
            serde_json::from_str::<Vec<ListedBuild>>(text).unwrap(),
            vec![build]
        );
    }

    #[test]
    fn the_record_lives_under_the_data_dir() {
        assert_eq!(
            record_path(Path::new("/d")),
            PathBuf::from("/d/pr-try/state.json")
        );
    }

    #[test]
    fn short_sha_keeps_nine_characters_and_leaves_short_text_whole() {
        assert_eq!(short_sha("eb1f5793b0123"), "eb1f5793b");
        assert_eq!(short_sha("abc"), "abc");
    }

    #[test]
    fn a_try_is_active_only_while_it_replaced_something() {
        let swapped = SwappedPlugin {
            id: "qol-launcher".into(),
            target: "/p".into(),
            backup: "/b".into(),
        };
        for (record, active) in [
            (TryRecord::default(), false),
            (
                TryRecord {
                    pull_request: Some(1),
                    ..TryRecord::default()
                },
                false,
            ),
            (
                TryRecord {
                    pull_request: Some(1),
                    plugins: vec![swapped.clone()],
                    ..TryRecord::default()
                },
                true,
            ),
            (
                TryRecord {
                    pull_request: Some(1),
                    tray: Some("/t".into()),
                    ..TryRecord::default()
                },
                true,
            ),
            (
                TryRecord {
                    plugins: vec![swapped],
                    ..TryRecord::default()
                },
                false,
            ),
        ] {
            assert_eq!(record.is_active(), active, "{record:?}");
        }
    }

    #[test]
    fn a_record_written_before_tray_tries_still_loads() {
        let record: TryRecord =
            serde_json::from_str(r#"{"pull_request":412,"head_sha":"abc","plugins":[]}"#).unwrap();
        assert_eq!(record.pull_request, Some(412));
        assert_eq!(record.tray, None);
        assert_eq!(record.installed_tray, None);
    }

    #[test]
    fn the_installed_tray_round_trips_and_stays_out_when_unknown() {
        let record = TryRecord {
            pull_request: Some(1),
            tray: Some("/t/qol-tray".into()),
            installed_tray: Some("/Applications/QoL Tray.app/Contents/MacOS/qol-tray".into()),
            ..TryRecord::default()
        };
        let text = serde_json::to_string(&record).unwrap();
        assert_eq!(serde_json::from_str::<TryRecord>(&text).unwrap(), record);
        let unknown = serde_json::to_string(&TryRecord::default()).unwrap();
        assert!(!unknown.contains("installed_tray"), "{unknown}");
    }
}
