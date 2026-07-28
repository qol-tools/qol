use std::process::ExitCode;

use qol_headless::{Command, CommandContext, CommandResult, DoctorCheck, HeadlessApp};

use super::operational::Operation;

const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
const BINARY_NAME: &str = "alt-tab";
const OPERATIONS: [OperationSpec; 5] = [
    OperationSpec::new(
        "daemon",
        Operation::Daemon,
        "Run the retained Alt Tab picker daemon.",
        "Lifecycle diagnostics are written to stderr.",
        "Runs until stopped; exits after the daemon listener or GPUI application stops.",
    ),
    OperationSpec::new(
        "--show",
        Operation::Show,
        "Show or advance the retained Alt Tab picker.",
        "No stdout on success.",
        "Exits after signaling the daemon, or runs the daemon when none is available.",
    ),
    OperationSpec::new(
        "--show-reverse",
        Operation::ShowReverse,
        "Show or move backward through the retained Alt Tab picker.",
        "No stdout on success.",
        "Exits after signaling the daemon, or starts the hidden daemon when none is available.",
    ),
    OperationSpec::new(
        "--settings",
        Operation::Settings,
        "Open the Alt Tab settings surface.",
        "No stdout on success; launch failures are written to stderr.",
        "Exits after the settings launcher returns.",
    ),
    OperationSpec::new(
        "--kill",
        Operation::Kill,
        "Ask the running Alt Tab daemon to stop.",
        "No output on success.",
        "Exits zero whether or not a daemon is currently running.",
    ),
];

#[derive(Clone, Copy)]
struct OperationSpec {
    name: &'static str,
    operation: Operation,
    about: &'static str,
    output: &'static str,
    exit_behavior: &'static str,
}

impl OperationSpec {
    const fn new(
        name: &'static str,
        operation: Operation,
        about: &'static str,
        output: &'static str,
        exit_behavior: &'static str,
    ) -> Self {
        Self {
            name,
            operation,
            about,
            output,
            exit_behavior,
        }
    }
}

trait Operations: Clone + Send + Sync + 'static {
    fn execute(&self, operation: Operation) -> CommandResult;
}

#[derive(Clone, Copy)]
struct ProductionOperations;

impl Operations for ProductionOperations {
    fn execute(&self, operation: Operation) -> CommandResult {
        super::operational::execute(operation)
    }
}

pub(crate) fn exit_code(args: impl IntoIterator<Item = String>) -> ExitCode {
    app(ProductionOperations, super::doctor::checks()).run(args)
}

fn app<O>(operations: O, doctor_checks: Vec<DoctorCheck>) -> HeadlessApp
where
    O: Operations,
{
    OPERATIONS
        .into_iter()
        .fold(
            HeadlessApp::new(PLUGIN_ID, BINARY_NAME)
                .about("Switch desktop windows through a retained native picker.")
                .default_command(["daemon"]),
            |app, spec| app.command(operation_command(spec, operations.clone())),
        )
        .doctor_checks(doctor_checks)
}

fn operation_command<O>(spec: OperationSpec, operations: O) -> Command
where
    O: Operations,
{
    let mut command = Command::new(spec.name)
        .about(spec.about)
        .usage(format!("{BINARY_NAME} {}", spec.name))
        .output(spec.output)
        .exit_behavior(spec.exit_behavior);
    if spec.operation == Operation::Daemon {
        command = command.detail("Running the binary without arguments selects this command.");
    }
    command.run_result(move |context| {
        let operation = select_operation(spec.operation, context);
        Ok(operations.execute(operation))
    })
}

fn select_operation(primary: Operation, context: &CommandContext) -> Operation {
    context
        .args()
        .iter()
        .filter_map(|argument| operation_for_argument(argument))
        .fold(primary, |selected, candidate| {
            if operation_priority(candidate) > operation_priority(selected) {
                candidate
            } else {
                selected
            }
        })
}

fn operation_for_argument(argument: &str) -> Option<Operation> {
    match argument {
        "--show" => Some(Operation::Show),
        "--show-reverse" => Some(Operation::ShowReverse),
        "--settings" => Some(Operation::Settings),
        "--kill" => Some(Operation::Kill),
        _ => None,
    }
}

fn operation_priority(operation: Operation) -> u8 {
    match operation {
        Operation::Daemon => 0,
        Operation::Show => 1,
        Operation::ShowReverse => 2,
        Operation::Kill => 3,
        Operation::Settings => 4,
    }
}

#[cfg(test)]
mod tests;
