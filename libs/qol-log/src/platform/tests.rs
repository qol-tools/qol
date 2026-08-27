fn assert_log_dir(dir: std::path::PathBuf) {
    assert!(dir.is_absolute(), "log dir {dir:?} should be absolute");
    assert!(
        dir.to_string_lossy().contains("qol-tray"),
        "log dir {dir:?} should contain qol-tray"
    );
}

#[test]
fn active_log_dir_is_absolute_and_contains_app_name() {
    assert_log_dir(super::log_dir());
}

#[test]
fn fallback_log_dir_is_absolute_and_contains_app_name() {
    assert_log_dir(super::fallback::log_dir());
}
