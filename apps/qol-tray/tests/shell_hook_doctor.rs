use qol_tray::doctor::{self, OutcomeStatus};
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use tempfile::TempDir;

const BEGIN_MARKER: &str = "# >>> qol-tools shell hook >>>";
const END_MARKER: &str = "# <<< qol-tools shell hook <<<";
const HOOK_BODY: &str = "[ -f \"$HOME/repos/private/qol-tools/qol-cicd/bin/activate.sh\" ] && \\\n  source \"$HOME/repos/private/qol-tools/qol-cicd/bin/activate.sh\"";

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct HomeGuard {
    previous: Option<OsString>,
    _tempdir: TempDir,
    home: std::path::PathBuf,
}

impl HomeGuard {
    fn new() -> Self {
        let tempdir = TempDir::new().unwrap();
        let previous = std::env::var_os("HOME");
        std::env::set_var("HOME", tempdir.path());
        let home = tempdir.path().to_path_buf();
        Self {
            previous,
            _tempdir: tempdir,
            home,
        }
    }
}

impl Drop for HomeGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
    }
}

fn canonical_block() -> String {
    format!("{BEGIN_MARKER}\n{HOOK_BODY}\n{END_MARKER}")
}

fn write_rc(path: &Path, content: &str) {
    fs::write(path, content).unwrap();
}

fn shell_hook_outcome(report: &doctor::Report) -> &doctor::Outcome {
    report
        .outcomes()
        .find(|o| o.id == "shell_hook_present")
        .expect("expected shell_hook_present in report")
}

#[test]
fn doctor_warns_when_block_missing_from_existing_zshrc() {
    let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());
    let home = HomeGuard::new();
    write_rc(&home.home.join(".zshrc"), "alias g=git\n");

    let report = doctor::check();
    let outcome = shell_hook_outcome(&report);
    assert_eq!(outcome.status, OutcomeStatus::Warn);
    assert!(outcome.fix_available);
}

#[test]
fn doctor_ok_when_block_present_in_zshrc() {
    let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());
    let home = HomeGuard::new();
    write_rc(
        &home.home.join(".zshrc"),
        &format!("alias g=git\n\n{}\n", canonical_block()),
    );

    let report = doctor::check();
    let outcome = shell_hook_outcome(&report);
    assert_eq!(outcome.status, OutcomeStatus::Ok);
    assert!(!outcome.fix_available);
}

#[test]
fn doctor_ok_when_no_rc_files_exist() {
    let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());
    let _home = HomeGuard::new();

    let report = doctor::check();
    let outcome = shell_hook_outcome(&report);
    assert_eq!(outcome.status, OutcomeStatus::Ok);
    assert!(!outcome.fix_available);
}

#[test]
fn doctor_fix_safe_installs_block_into_existing_zshrc() {
    let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());
    let home = HomeGuard::new();
    let zshrc = home.home.join(".zshrc");
    write_rc(&zshrc, "alias g=git\n");

    let fix_report = doctor::fix_safe();
    let outcome = shell_hook_outcome(&fix_report.after);
    assert_eq!(outcome.status, OutcomeStatus::Ok);

    let content = fs::read_to_string(&zshrc).unwrap();
    assert!(content.starts_with("alias g=git\n"));
    assert!(content.contains(BEGIN_MARKER));
    assert!(content.contains(END_MARKER));
    assert!(content.contains(HOOK_BODY));
}
