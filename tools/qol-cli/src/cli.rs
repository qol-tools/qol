use anyhow::{anyhow, bail, Result};
use std::ffi::OsString;

pub(crate) struct CliArgs {
    pub(crate) values: Vec<OsString>,
    pub(crate) verbose: bool,
    pub(crate) skip_plugins: bool,
}

pub(crate) fn parse_cli(args: Vec<OsString>) -> CliArgs {
    let mut values = Vec::new();
    let mut verbose = false;
    let mut skip_plugins = false;
    for arg in args {
        match arg.to_str() {
            Some("--verbose" | "-v") => {
                verbose = true;
                continue;
            }
            Some("--no-plugins" | "-n") => {
                skip_plugins = true;
                continue;
            }
            _ => {}
        }
        values.push(arg);
    }
    CliArgs {
        values,
        verbose,
        skip_plugins,
    }
}

pub(crate) fn print_help() -> Result<()> {
    print!("{}", help_text());
    Ok(())
}

pub(crate) fn help_text() -> &'static str {
    "qol commands:\n  qol setup\n  qol dev [worktree]\n  qol build [name]\n  qol clean [name]\n  qol install\n  qol sync\n\nOptions:\n  -v, --verbose     show child command output\n  -n, --no-plugins  qol dev: skip plugin rebuilds\n"
}

pub(crate) fn optional_single_arg<'a>(
    args: &'a [OsString],
    usage: &str,
) -> Result<Option<&'a str>> {
    if args.len() > 1 {
        bail!("usage: {usage}");
    }
    let arg = match args.first() {
        Some(arg) => arg,
        None => return Ok(None),
    };
    let value = arg
        .to_str()
        .ok_or_else(|| anyhow!("argument is not valid UTF-8"))?;
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_verbose_flag_before_command() {
        let args = parse_cli(vec!["--verbose".into(), "install".into()]);
        assert!(args.verbose);
        assert_eq!(args.values, vec![OsString::from("install")]);
    }

    #[test]
    fn parses_verbose_flag_after_command() {
        let args = parse_cli(vec!["install".into(), "-v".into()]);
        assert!(args.verbose);
        assert_eq!(args.values, vec![OsString::from("install")]);
    }
}
