mod platform;

use std::ffi::OsString;
use std::process::Command;

use anyhow::{bail, Context, Result};

const USAGE: &str = "lane-exec --secret NAME --env VAR -- PROGRAM [ARGS...]";

#[derive(Debug, PartialEq, Eq)]
struct LaneExec {
    secret: String,
    env: String,
    program: OsString,
    args: Vec<OsString>,
}

pub fn run(args: &[OsString]) -> Result<()> {
    let lane = parse(args)?;
    let token = qol_secrets::read(&lane.secret)
        .with_context(|| format!("lane-exec could not read secret `{}`", lane.secret))?;
    let mut command = Command::new(&lane.program);
    command.args(&lane.args).env(&lane.env, token);
    platform::run_in_place(command).with_context(|| {
        format!(
            "lane-exec failed to start {}",
            lane.program.to_string_lossy()
        )
    })
}

fn parse(args: &[OsString]) -> Result<LaneExec> {
    let (mut secret, mut env) = (None, None);
    let mut rest = args.iter();
    loop {
        let Some(flag) = rest.next() else {
            bail!("lane-exec needs `-- PROGRAM`; usage: {USAGE}");
        };
        let slot = match flag.to_str() {
            Some("--") => break,
            Some("--secret") => &mut secret,
            Some("--env") => &mut env,
            _ => bail!(
                "unexpected lane-exec argument `{}`; usage: {USAGE}",
                flag.to_string_lossy()
            ),
        };
        let value = rest
            .next()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .with_context(|| format!("{} needs a value; usage: {USAGE}", flag.to_string_lossy()))?;
        *slot = Some(value.to_owned());
    }
    let (Some(secret), Some(env)) = (secret, env) else {
        bail!("lane-exec needs both --secret and --env; usage: {USAGE}");
    };
    let Some(program) = rest.next().cloned() else {
        bail!("lane-exec needs a program after `--`; usage: {USAGE}");
    };
    Ok(LaneExec {
        secret,
        env,
        program,
        args: rest.cloned().collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn everything_after_the_separator_is_the_lane_command_verbatim() {
        let lane = parse(&argv(&[
            "--secret", "s", "--env", "TOKEN", "--", "claude", "--secret", "--", "task",
        ]))
        .unwrap();
        assert_eq!(
            lane,
            LaneExec {
                secret: "s".to_owned(),
                env: "TOKEN".to_owned(),
                program: "claude".into(),
                args: argv(&["--secret", "--", "task"]),
            }
        );
    }

    #[test]
    fn a_missing_flag_value_or_program_is_refused() {
        for args in [
            argv(&["--env", "TOKEN", "--", "claude"]),
            argv(&["--secret", "s", "--env", "TOKEN", "--"]),
            argv(&["--secret", "--env", "TOKEN", "--", "claude"]),
            argv(&["--secret", "s", "--env", "TOKEN", "claude"]),
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
    }
}
