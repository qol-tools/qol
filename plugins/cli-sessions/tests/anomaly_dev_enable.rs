use std::fs;

use plugin_cli_sessions::anomaly::{enable, observe};
use plugin_cli_sessions::attention::Phase;
use plugin_cli_sessions::host::kitty_session_id;
use plugin_cli_sessions::status::Status;

#[test]
fn enable_records_without_the_env_flag() {
    let dir = std::env::temp_dir().join(format!("cli-sessions-dev-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    std::env::remove_var("CLI_SESSIONS_RECORD_ANOMALIES");
    std::env::set_var("CLI_SESSIONS_ANOMALY_DIR", &dir);

    enable();

    observe(
        kitty_session_id(7),
        0,
        "x",
        Some("busy"),
        Phase::Busy,
        Status::Working,
    );
    observe(
        kitty_session_id(7),
        3,
        "x",
        Some("picker"),
        Phase::Blocked,
        Status::NeedsYou,
    );
    observe(
        kitty_session_id(7),
        6,
        "x",
        Some("busy"),
        Phase::Busy,
        Status::Working,
    );

    let dirs: Vec<_> = fs::read_dir(&dir)
        .expect("enable() must record without the env flag")
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(dirs.len(), 1, "one flap recorded, got {dirs:?}");

    let _ = fs::remove_dir_all(&dir);
}
