use std::fs;

use plugin_cli_sessions::anomaly::observe;
use plugin_cli_sessions::host::kitty_session_id;
use plugin_cli_sessions::status::Status;
use plugin_cli_sessions::strategy::Phase;

#[test]
fn a_needs_you_flap_is_recorded_to_disk_when_enabled() {
    let dir = std::env::temp_dir().join(format!("cli-sessions-anomaly-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    std::env::set_var("CLI_SESSIONS_RECORD_ANOMALIES", "1");
    std::env::set_var("CLI_SESSIONS_ANOMALY_DIR", &dir);

    let screen = "How should this run?\n\u{276F} 1. Yes\n  2. No\n  enter to confirm";
    observe(
        kitty_session_id(42),
        0,
        "working",
        Some("busy screen"),
        Phase::Busy,
        Status::Working,
    );
    observe(
        kitty_session_id(42),
        3,
        "x",
        Some(screen),
        Phase::Blocked,
        Status::NeedsYou,
    );
    observe(
        kitty_session_id(42),
        6,
        "working",
        Some("busy again"),
        Phase::Busy,
        Status::Working,
    );

    let captured: Vec<_> = fs::read_dir(&dir)
        .expect("recorder must create the anomaly dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    assert_eq!(captured.len(), 1, "exactly one flap dir, got {captured:?}");

    let report = fs::read_to_string(captured[0].join("report.json")).expect("report.json written");
    assert!(report.contains("needs_you_flap"), "report kind: {report}");
    assert!(
        report.contains("NeedsYou"),
        "report retains the offending status frame: {report}"
    );
    let frames: Vec<_> = fs::read_dir(&captured[0])
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("frame_"))
        .collect();
    assert!(!frames.is_empty(), "ring frames are dumped for review");

    let _ = fs::remove_dir_all(&dir);
}
