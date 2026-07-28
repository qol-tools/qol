use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use qol_headless::{DoctorCheckResult, DoctorReport, DoctorStatus, EXIT_SUCCESS, EXIT_USAGE};
use qol_plugin_api::manifest::{ActionType, PluginManifest};

use super::*;

#[derive(Clone)]
struct SentinelOperations {
    calls: Arc<Mutex<Vec<Operation>>>,
}

impl Operations for SentinelOperations {
    fn execute(&self, operation: Operation) -> CommandResult {
        self.calls
            .lock()
            .expect("operation calls poisoned")
            .push(operation);
        CommandResult::success("")
    }
}

fn sentinel() -> (HeadlessApp, Arc<Mutex<Vec<Operation>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let operations = SentinelOperations {
        calls: Arc::clone(&calls),
    };
    (
        app(operations, sentinel_doctor_checks()),
        Arc::clone(&calls),
    )
}

fn sentinel_doctor_checks() -> Vec<DoctorCheck> {
    super::super::doctor::check_ids()
        .iter()
        .map(|id| {
            let id = *id;
            DoctorCheck::new(id, format!("Sentinel {id} check."), move || {
                Ok(DoctorCheckResult::ok(id, format!("{id} is healthy")))
            })
        })
        .collect()
}

#[test]
fn legacy_routes_and_priority_reach_only_the_selected_operation() {
    let cases = [
        (Vec::<String>::new(), Operation::Daemon),
        (vec!["daemon".into()], Operation::Daemon),
        (vec!["--show".into()], Operation::Show),
        (vec!["--show-reverse".into()], Operation::ShowReverse),
        (vec!["--settings".into()], Operation::Settings),
        (vec!["--kill".into()], Operation::Kill),
        (vec!["--show".into(), "--kill".into()], Operation::Kill),
        (
            vec!["--show".into(), "--show-reverse".into()],
            Operation::ShowReverse,
        ),
        (
            vec!["--kill".into(), "--settings".into()],
            Operation::Settings,
        ),
        (
            vec!["--settings".into(), "--kill".into()],
            Operation::Settings,
        ),
        (
            vec!["--show-reverse".into(), "--show".into()],
            Operation::ShowReverse,
        ),
        (
            vec!["--show-reverse".into(), "--kill".into()],
            Operation::Kill,
        ),
        (
            vec!["--show-reverse".into(), "legacy-tail".into()],
            Operation::ShowReverse,
        ),
    ];

    for (args, expected) in cases {
        let (app, calls) = sentinel();
        let execution = app.execute(args);

        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert_eq!(
            calls.lock().expect("operation calls poisoned").as_slice(),
            [expected]
        );
    }
}

#[test]
fn manifest_actions_are_unchanged_and_have_contextual_help() {
    let manifest = PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
    let expected = BTreeMap::from([
        ("open", (ActionType::Run, vec!["--show"])),
        ("open-reverse", (ActionType::Run, vec!["--show-reverse"])),
        ("settings", (ActionType::Settings, vec!["--settings"])),
    ]);

    assert!(manifest.capabilities.doctor);
    assert_eq!(manifest.actions.len(), expected.len());
    for action in manifest.executable_actions() {
        let (kind, args) = expected
            .get(action.id.as_str())
            .unwrap_or_else(|| panic!("unexpected manifest action {}", action.id));
        assert_eq!(&action.kind, kind, "action={}", action.id);
        assert_eq!(
            manifest.catalog_runtime_args(&action.id),
            Some(args.iter().map(|arg| (*arg).to_string()).collect()),
            "action={}",
            action.id
        );

        let execution = sentinel()
            .0
            .execute(["help".to_string(), args[0].to_string()]);
        assert_eq!(
            execution.exit_code, EXIT_SUCCESS,
            "action={} stderr={}",
            action.id, execution.stderr
        );
    }
}

#[test]
fn contextual_help_is_equivalent_in_both_positions() {
    for command in [
        "daemon",
        "--show",
        "--show-reverse",
        "--settings",
        "--kill",
        "doctor",
    ] {
        let first = sentinel()
            .0
            .execute(["help".to_string(), command.to_string()]);
        let final_token = sentinel()
            .0
            .execute([command.to_string(), "help".to_string()]);

        assert_eq!(first.exit_code, EXIT_SUCCESS, "command={command}");
        assert_eq!(first.stdout, final_token.stdout, "command={command}");
        assert!(first.stdout.contains("Output:"), "command={command}");
        assert!(first.stdout.contains("Exit:"), "command={command}");
    }
}

#[test]
fn doctor_json_has_the_shared_schema_in_both_flag_positions() {
    let before = sentinel()
        .0
        .execute(["--json".to_string(), "doctor".to_string()]);
    let after = sentinel()
        .0
        .execute(["doctor".to_string(), "--json".to_string()]);

    assert_eq!(before.exit_code, EXIT_SUCCESS);
    assert_eq!(before.stdout, after.stdout);
    assert!(before.stderr.is_empty());
    assert!(after.stderr.is_empty());

    let report: DoctorReport =
        serde_json::from_str(&before.stdout).expect("doctor output must be valid JSON");
    assert_eq!(report.plugin_id, PLUGIN_ID);
    assert_eq!(report.status, DoctorStatus::Ok);
    assert_eq!(
        report
            .checks
            .iter()
            .map(|check| check.id.as_str())
            .collect::<Vec<_>>(),
        super::super::doctor::check_ids()
    );
}

#[test]
fn help_and_doctor_never_invoke_operational_handlers() {
    let cases = [
        vec!["help"],
        vec!["--help"],
        vec!["help", "daemon"],
        vec!["--show", "help"],
        vec!["help", "--show-reverse"],
        vec!["--settings", "help"],
        vec!["help", "--kill"],
        vec!["doctor"],
        vec!["--json", "doctor"],
        vec!["doctor", "--json"],
        vec!["help", "doctor"],
        vec!["doctor", "help"],
    ];

    for args in cases {
        let (app, calls) = sentinel();
        let execution = app.execute(args.iter().map(|argument| (*argument).to_string()));

        assert_eq!(execution.exit_code, EXIT_SUCCESS, "args={args:?}");
        assert!(
            calls.lock().expect("operation calls poisoned").is_empty(),
            "args={args:?}"
        );
    }
}

#[test]
fn unsupported_json_is_rejected_before_any_operation() {
    for args in [
        vec!["--json"],
        vec!["--json", "--show"],
        vec!["--show", "--json"],
        vec!["--json", "--settings"],
        vec!["--kill", "--json"],
    ] {
        let (app, calls) = sentinel();
        let execution = app.execute(args.iter().map(|argument| (*argument).to_string()));

        assert_eq!(execution.exit_code, EXIT_USAGE, "args={args:?}");
        assert!(
            execution.stderr.contains("does not support --json"),
            "args={args:?}"
        );
        assert!(
            calls.lock().expect("operation calls poisoned").is_empty(),
            "args={args:?}"
        );
    }
}
