use super::*;

fn record(from: &str, attempts: u32, plugins: &[&str]) -> RestartRecord {
    RestartRecord {
        from_version: from.to_string(),
        attempts,
        update_plugins: plugins.iter().map(|id| id.to_string()).collect(),
    }
}

fn plugins(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| id.to_string()).collect()
}

#[test]
fn write_then_begin_counts_the_attempt_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let written = record("3.66.1", 0, &["qol-launcher", "qol-alt-tab"]);
    let path = write(dir.path(), &written).unwrap();
    assert_eq!(path, record_path(dir.path()));

    let first = begin(dir.path()).unwrap();
    assert_eq!(first, record("3.66.1", 1, &["qol-launcher", "qol-alt-tab"]));
    let second = begin(dir.path()).unwrap();
    assert_eq!(second.attempts, 2);

    finish(dir.path());
    assert!(!path.exists());
    assert_eq!(begin(dir.path()), None);
    finish(dir.path());
}

#[test]
fn write_leaves_no_temp_files_behind() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), &record("1.0.0", 0, &[])).unwrap();
    write(dir.path(), &record("1.0.1", 0, &["a"])).unwrap();
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from(RECORD_FILE)]);
}

#[test]
fn begin_reads_the_legacy_update_marker_once() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(LEGACY_UPDATE_MARKER), "3.60.0\n").unwrap();

    assert_eq!(begin(dir.path()), Some(record("3.60.0", 1, &[])));
    assert!(!dir.path().join(LEGACY_UPDATE_MARKER).exists());
    assert_eq!(begin(dir.path()).map(|record| record.attempts), Some(2));
}

#[test]
fn begin_discards_an_unreadable_record() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(record_path(dir.path()), "{not json").unwrap();

    assert_eq!(begin(dir.path()), None);
    assert!(!record_path(dir.path()).exists());
}

#[test]
fn a_record_from_a_later_tray_keeps_the_fields_this_tray_knows() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        record_path(dir.path()),
        r#"{"from_version":"3.66.1","attempts":0,"update_plugins":["a"],"reopen_windows":[{"id":"x"}]}"#,
    )
    .unwrap();

    assert_eq!(begin(dir.path()), Some(record("3.66.1", 1, &["a"])));
}

#[test]
fn resume_plan_table() {
    let cases = [
        (
            record("3.66.1", 1, &["a", "b"]),
            Some("3.66.1"),
            plugins(&["a", "b"]),
            plugins(&[]),
        ),
        (
            record(" 3.66.1\n", 1, &[]),
            Some("3.66.1"),
            plugins(&[]),
            plugins(&[]),
        ),
        (
            record("3.66.1", 2, &["a", "b"]),
            Some("3.66.1"),
            plugins(&[]),
            plugins(&["a", "b"]),
        ),
        (
            record("3.67.0", 1, &["a"]),
            None,
            plugins(&[]),
            plugins(&[]),
        ),
        (record("", 1, &["a"]), None, plugins(&[]), plugins(&[])),
    ];
    for (record, updated_from, update_plugins, dropped_plugins) in cases {
        let expected = Resume {
            updated_from: updated_from.map(str::to_string),
            update_plugins,
            dropped_plugins,
        };
        assert_eq!(resume_plan(&record, "3.67.0"), expected, "{record:?}");
    }
}
