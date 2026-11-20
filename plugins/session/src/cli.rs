use std::process::ExitCode;
use std::sync::Arc;

use anyhow::Result;
use qol_headless::{
    Command, CommandContext, DoctorCheck, DoctorCheckResult, HeadlessApp, PlainTextOutput,
};

use crate::lock::Availability;

const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");

pub fn exit_code(args: impl IntoIterator<Item = String>) -> ExitCode {
    app().run(args)
}

fn app() -> HeadlessApp {
    app_with_handlers(
        |_| {
            crate::lock::lock_screen()?;
            Ok(PlainTextOutput::empty())
        },
        crate::lock::availability,
    )
}

fn app_with_handlers<Lock, Probe>(lock: Lock, probe: Probe) -> HeadlessApp
where
    Lock: Fn(&CommandContext) -> Result<PlainTextOutput> + Send + Sync + 'static,
    Probe: Fn() -> Availability + Send + Sync + 'static,
{
    HeadlessApp::new(PLUGIN_ID, PLUGIN_ID)
        .about("Lock the screen on Linux, macOS and Windows.")
        .command(
            Command::new("lock")
                .about("Lock the screen now.")
                .usage(format!("{PLUGIN_ID} lock"))
                .output("No stdout on success.")
                .exit_behavior("Exits non-zero if no screen locker accepted the request.")
                .run_plain_text(lock),
        )
        .doctor_checks(doctor_checks(Arc::new(probe)))
}

fn doctor_checks(probe: Arc<dyn Fn() -> Availability + Send + Sync>) -> Vec<DoctorCheck> {
    vec![
        DoctorCheck::new(
            "config_readable",
            "Verify the plugin can run without persistent config.",
            || {
                Ok(DoctorCheckResult::ok(
                    "config_readable",
                    "No persistent config is required.",
                ))
            },
        ),
        DoctorCheck::new(
            "screen_locker",
            "Verify this machine has a way to lock the screen.",
            move || Ok(screen_locker_result(probe())),
        ),
    ]
}

fn screen_locker_result(availability: Availability) -> DoctorCheckResult {
    match availability {
        Availability::Ready(detail) => DoctorCheckResult::ok("screen_locker", detail),
        Availability::Missing { detail, fix } => {
            DoctorCheckResult::fail("screen_locker", detail).with_fix(fix)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use qol_headless::{DoctorReport, DoctorStatus, EXIT_SUCCESS};

    use super::*;

    fn sentinel_app(locks: Arc<AtomicUsize>, availability: Availability) -> HeadlessApp {
        app_with_handlers(
            move |_| {
                locks.fetch_add(1, Ordering::SeqCst);
                Ok(PlainTextOutput::empty())
            },
            move || availability.clone(),
        )
    }

    fn ready() -> Availability {
        Availability::Ready("test locker will lock the screen.".into())
    }

    #[test]
    fn lock_runs_the_lock_handler_once() {
        let locks = Arc::new(AtomicUsize::new(0));
        let execution = sentinel_app(Arc::clone(&locks), ready()).execute(["lock".to_string()]);

        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert_eq!(execution.stdout, "");
        assert_eq!(locks.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn doctor_and_help_never_lock_the_screen() {
        let cases = [
            vec![],
            vec!["help"],
            vec!["help", "lock"],
            vec!["lock", "help"],
            vec!["doctor"],
            vec!["--json", "doctor"],
            vec!["doctor", "help"],
        ];

        for args in cases {
            let locks = Arc::new(AtomicUsize::new(0));
            sentinel_app(Arc::clone(&locks), ready())
                .execute(args.iter().map(|arg| (*arg).to_string()));
            assert_eq!(locks.load(Ordering::SeqCst), 0, "args: {args:?}");
        }
    }

    #[test]
    fn doctor_reports_the_screen_locker_state() {
        let missing = Availability::Missing {
            detail: "No screen locker is on PATH.".into(),
            fix: "Install one.".into(),
        };
        let cases = [
            (ready(), DoctorStatus::Ok, None),
            (missing, DoctorStatus::Fail, Some("Install one.")),
        ];

        for (availability, status, fix) in cases {
            let locks = Arc::new(AtomicUsize::new(0));
            let execution = sentinel_app(locks, availability.clone())
                .execute(["--json".to_string(), "doctor".to_string()]);
            let report: DoctorReport =
                serde_json::from_str(&execution.stdout).expect("doctor output must be JSON");
            let check = report
                .checks
                .iter()
                .find(|check| check.id == "screen_locker")
                .expect("screen_locker check must exist");

            assert_eq!(report.plugin_id, PLUGIN_ID);
            assert_eq!(check.status, status, "availability: {availability:?}");
            assert_eq!(check.fix.as_deref(), fix, "availability: {availability:?}");
        }
    }
}
