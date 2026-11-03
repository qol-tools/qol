use std::path::Path;

use qol_conventions::pr_try::{record_path, short_sha, TryRecord};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trial {
    pub pull_request: u64,
    pub head: String,
    pub tray: bool,
    pub plugins: Vec<String>,
}

pub fn current() -> Option<Trial> {
    let data = crate::paths::base_data_dir().ok()?;
    read(&data, super::stand_in::is_stand_in())
}

fn read(data_dir: &Path, standing_in: bool) -> Option<Trial> {
    let text = std::fs::read_to_string(record_path(data_dir)).ok()?;
    let record: TryRecord = serde_json::from_str(&text).ok()?;
    from_record(&record, standing_in)
}

fn from_record(record: &TryRecord, standing_in: bool) -> Option<Trial> {
    if !record.is_active() {
        return None;
    }
    Some(Trial {
        pull_request: record.pull_request?,
        head: record
            .head_sha
            .as_deref()
            .map(short_sha)
            .unwrap_or_default()
            .to_string(),
        tray: standing_in && record.tray.is_some(),
        plugins: record
            .plugins
            .iter()
            .map(|swapped| swapped.id.clone())
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_conventions::pr_try::SwappedPlugin;

    fn record(tray: bool, plugins: &[&str]) -> TryRecord {
        TryRecord {
            pull_request: Some(435),
            head_sha: Some("eb1f5793b0123456".into()),
            plugins: plugins
                .iter()
                .map(|id| SwappedPlugin {
                    id: (*id).into(),
                    target: "/p".into(),
                    backup: "/b".into(),
                })
                .collect(),
            tray: tray.then(|| "/cache/tray/qol-tray".into()),
            installed_tray: None,
        }
    }

    #[test]
    fn a_try_reads_its_number_short_head_and_replaced_plugins() {
        assert_eq!(
            from_record(&record(true, &["qol-launcher"]), true),
            Some(Trial {
                pull_request: 435,
                head: "eb1f5793b".into(),
                tray: true,
                plugins: vec!["qol-launcher".into()],
            })
        );
    }

    #[test]
    fn the_tray_counts_as_tried_only_while_the_try_build_runs() {
        let trial = from_record(&record(true, &[]), false).unwrap();
        assert!(!trial.tray);
        assert!(from_record(&record(false, &[]), true).is_none());
    }

    #[test]
    fn no_record_or_a_broken_one_means_no_try() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(read(root.path(), false), None);
        let path = record_path(root.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(read(root.path(), false), None);
        std::fs::write(
            &path,
            serde_json::to_string(&record(false, &["qol-x"])).unwrap(),
        )
        .unwrap();
        assert_eq!(read(root.path(), false).unwrap().pull_request, 435);
    }
}
