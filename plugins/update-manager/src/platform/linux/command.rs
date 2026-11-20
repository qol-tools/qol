use std::process::Command;
use std::time::Duration;

use qol_process::{run_owned_with_output_timeout, BoundedCommandOutput};

use super::{LABEL, SHELL};

pub(crate) const READ: Duration = Duration::from_secs(120);
const OUTPUT_LIMIT: usize = 8 * 1024 * 1024;

pub(crate) struct Done {
    pub(crate) code: Option<i32>,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

impl Done {
    pub(crate) fn reason(&self) -> String {
        [&self.stderr, &self.stdout]
            .into_iter()
            .find_map(|text| {
                text.lines()
                    .rev()
                    .map(str::trim)
                    .find(|line| !line.is_empty())
                    .map(String::from)
            })
            .unwrap_or_else(|| match self.code {
                Some(code) => format!("exited with code {code}"),
                None => "stopped by a signal".into(),
            })
    }
}

pub(crate) fn run(program: &str, args: &[&str], timeout: Duration) -> Result<Done, String> {
    let mut command = Command::new(program);
    command.args(args).env("LANG", "C").env("LC_ALL", "C");
    match run_owned_with_output_timeout(command, timeout, OUTPUT_LIMIT) {
        Ok(BoundedCommandOutput::Completed(output)) => Ok(Done {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(output.stdout.as_bytes()).into_owned(),
            stderr: String::from_utf8_lossy(output.stderr.as_bytes()).into_owned(),
        }),
        Ok(BoundedCommandOutput::TimedOut { .. }) => Err(format!(
            "{program} did not finish within {} minutes",
            timeout.as_secs() / 60
        )),
        Err(error) => Err(format!("{program} could not start: {error}")),
    }
}

pub(crate) fn script(script: &str, args: &[String], timeout: Duration) -> Result<Done, String> {
    let mut all = vec!["-c", script, LABEL];
    all.extend(args.iter().map(String::as_str));
    run(SHELL, &all, timeout)
}

pub(crate) fn output(program: &str, args: &[&str]) -> Result<String, String> {
    let done = run(program, args, READ)?;
    if done.code == Some(0) {
        Ok(done.stdout)
    } else {
        Err(format!("{program}: {}", done.reason()))
    }
}

pub(crate) fn lenient(program: &str, args: &[&str]) -> String {
    run(program, args, READ)
        .map(|done| done.stdout)
        .unwrap_or_default()
}

pub(crate) fn found(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}
