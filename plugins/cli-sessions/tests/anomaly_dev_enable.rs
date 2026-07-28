use std::fs;

use plugin_cli_sessions::anomaly::{enable, observe};
use plugin_cli_sessions::status::Status;
use plugin_cli_sessions::strategy::Phase;

// The dev build calls enable() at startup. Recording must turn on with NO
// CLI_SESSIONS_RECORD_ANOMALIES flag set - that is the whole point of the
// dev-gating: a launcher-started dev build just records.
#[test]
fn enable_records_without_the_env_flag() {
    let dir = std::env::temp_dir().join(format!("cli-sessions-dev-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    std::env::remove_var("CLI_SESSIONS_RECORD_ANOMALIES");
    std::env::set_var("CLI_SESSIONS_ANOMALY_DIR", &dir);

    enable();

    observe(7, 0, "x", Some("busy"), Phase::Busy, Status::Working);
    observe(7, 3, "x", Some("picker"), Phase::Blocked, Status::NeedsYou);
    observe(7, 6, "x", Some("busy"), Phase::Busy, Status::Working);

    let dirs: Vec<_> = fs::read_dir(&dir)
        .expect("enable() must record without the env flag")
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(dirs.len(), 1, "one flap recorded, got {dirs:?}");

    let _ = fs::remove_dir_all(&dir);
}
