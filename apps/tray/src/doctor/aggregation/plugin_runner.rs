use crate::plugins::resolver::PluginSource;
use qol_process::{run_guarded_with_output_timeout, BoundedCommandOutput, CapturedOutput};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

const PLUGIN_DOCTOR_TIMEOUT: Duration = Duration::from_secs(5);
const PLUGIN_DOCTOR_OUTPUT_LIMIT: usize = 1024 * 1024;

#[derive(Clone, Debug)]
pub(super) struct PluginDoctorTarget {
    pub(super) id: String,
    pub(super) plugin_dir: PathBuf,
    pub(super) source: PluginSource,
    pub(super) command: String,
}

pub(super) trait PluginDoctorRunner: Sync {
    fn invoke(&self, target: &PluginDoctorTarget) -> Invocation;
}

#[derive(Default)]
pub(super) struct ProcessPluginDoctorRunner;

impl PluginDoctorRunner for ProcessPluginDoctorRunner {
    fn invoke(&self, target: &PluginDoctorTarget) -> Invocation {
        let Some(command_path) = crate::plugins::resolve_plugin_command_path_for_source(
            &target.plugin_dir,
            &target.command,
            Some(&target.source),
        ) else {
            return Invocation::Failed(format!(
                "Could not resolve runtime command {:?} inside the plugin directory.",
                target.command
            ));
        };

        let source = source_label(&target.source);
        qol_runtime::probe!(
            "PLUGIN_DOCTOR",
            "plugin={} source={} stage=spawn",
            target.id,
            source
        );
        #[cfg(not(debug_assertions))]
        let _ = source;
        let command = doctor_command(target, &command_path);
        let guardian = match std::env::current_exe() {
            Ok(executable) => qol_process::process_tree_guardian_command(&executable),
            Err(error) => {
                trace_process_result(target, "guardian-error", None);
                return Invocation::Failed(format!(
                    "Could not resolve the plugin doctor process guardian: {error}"
                ));
            }
        };

        match run_guarded_with_output_timeout(
            command,
            guardian,
            PLUGIN_DOCTOR_TIMEOUT,
            PLUGIN_DOCTOR_OUTPUT_LIMIT,
        ) {
            Ok(BoundedCommandOutput::Completed(output)) => {
                let success = output.status.success();
                let exit_code = output.status.code();
                trace_process_result(
                    target,
                    if success { "success" } else { "failure" },
                    exit_code,
                );
                Invocation::Completed {
                    success,
                    exit_code,
                    stdout: output.stdout.into(),
                    stderr: output.stderr.into(),
                }
            }
            Ok(BoundedCommandOutput::TimedOut { stderr, .. }) => {
                trace_process_result(target, "timeout", None);
                Invocation::TimedOut {
                    stderr: stderr.into(),
                }
            }
            Err(error) => {
                trace_process_result(target, "spawn-error", None);
                Invocation::Failed(format!("Could not run plugin doctor: {error}"))
            }
        }
    }
}

fn doctor_command(target: &PluginDoctorTarget, command_path: &std::path::Path) -> Command {
    let mut command = Command::new(command_path);
    command
        .args(["doctor", "--json"])
        .current_dir(&target.plugin_dir)
        .env(qol_conventions::ENV_PLUGIN_ID, &target.id);
    command
}

fn trace_process_result(target: &PluginDoctorTarget, outcome: &str, exit_code: Option<i32>) {
    let source = source_label(&target.source);
    qol_runtime::probe!(
        "PLUGIN_DOCTOR",
        "plugin={} source={} stage=exit outcome={} code={:?}",
        target.id,
        source,
        outcome,
        exit_code
    );
    #[cfg(not(debug_assertions))]
    let _ = (target, source, outcome, exit_code);
}

pub(super) enum Invocation {
    Completed {
        success: bool,
        exit_code: Option<i32>,
        stdout: CapturedStream,
        stderr: CapturedStream,
    },
    TimedOut {
        stderr: CapturedStream,
    },
    Failed(String),
}

#[derive(Default)]
pub(super) struct CapturedStream {
    pub(super) bytes: Vec<u8>,
    pub(super) truncated: bool,
}

impl From<CapturedOutput> for CapturedStream {
    fn from(output: CapturedOutput) -> Self {
        Self {
            truncated: output.is_truncated(),
            bytes: output.into_bytes(),
        }
    }
}

pub(super) fn source_label(source: &PluginSource) -> &'static str {
    match source {
        PluginSource::Installed => "installed",
        PluginSource::DevLinked => "dev-linked",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::path::Path;

    #[test]
    fn doctor_command_uses_the_standalone_runtime_contract() {
        let target = PluginDoctorTarget {
            id: "plugin-test".to_string(),
            plugin_dir: PathBuf::from("/plugins/plugin-test"),
            source: PluginSource::Installed,
            command: "plugin-runtime".to_string(),
        };
        let command = doctor_command(&target, Path::new("/plugins/plugin-test/plugin-runtime"));

        assert_eq!(
            command.get_program(),
            OsStr::new("/plugins/plugin-test/plugin-runtime")
        );
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            vec![OsStr::new("doctor"), OsStr::new("--json")]
        );
        assert_eq!(
            command.get_current_dir(),
            Some(Path::new("/plugins/plugin-test"))
        );
        assert_eq!(
            command
                .get_envs()
                .find(|(key, _)| *key == OsStr::new(qol_conventions::ENV_PLUGIN_ID))
                .and_then(|(_, value)| value),
            Some(OsStr::new("plugin-test"))
        );
    }
}
