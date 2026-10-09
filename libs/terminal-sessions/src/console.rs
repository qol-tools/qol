use std::ffi::OsStr;
use std::io;

use qol_headless::CommandResult;

pub const PROBE_COMMAND: &str = "console-probe";
pub const SEND_COMMAND: &str = "console-send";
pub const LAUNCH_COMMAND: &str = "console-launch";
pub const TITLE_COMMAND: &str = "console-title";

pub fn helper(args: &[impl AsRef<OsStr>]) -> Option<CommandResult> {
    let (command, rest) = args.split_first()?;
    let command = command.as_ref().to_str()?;
    let run: fn(&[String]) -> io::Result<String> = match command {
        PROBE_COMMAND => crate::platform::console_probe,
        SEND_COMMAND => crate::platform::console_send,
        LAUNCH_COMMAND => crate::platform::console_launch,
        TITLE_COMMAND => crate::platform::console_title,
        _ => return None,
    };
    let outcome = rest
        .iter()
        .map(|arg| arg.as_ref().to_str().map(str::to_owned))
        .collect::<Option<Vec<String>>>()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "arguments must be UTF-8"))
        .and_then(|rest| run(&rest));
    Some(match outcome {
        Ok(stdout) => CommandResult::success(stdout),
        Err(error) => CommandResult::runtime_error(format!("{command}: {error}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_helper_commands_are_claimed() {
        let cases: [(&[&str], bool); 8] = [
            (&[], false),
            (&["doctor"], false),
            (&["console-probes"], false),
            (&[PROBE_COMMAND, "not-a-pid"], true),
            (&[SEND_COMMAND, "not-a-pid", "insert"], true),
            (&[LAUNCH_COMMAND, "not-a-tag"], true),
            (&[TITLE_COMMAND, "not-a-pid"], true),
            (&["status", PROBE_COMMAND], false),
        ];
        for (args, claimed) in cases {
            let result = helper(args);
            assert_eq!(result.is_some(), claimed, "{args:?}");
            if let Some(result) = result {
                assert_ne!(result.exit_code, qol_headless::EXIT_SUCCESS, "{args:?}");
            }
        }
    }
}
