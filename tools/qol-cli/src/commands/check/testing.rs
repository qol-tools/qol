use super::{cargo_command, CheckContext, CheckReport};
use anyhow::{bail, Context, Result};
use std::ffi::OsString;
use std::io;
use std::process::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TestRunner {
    Cargo,
    Nextest,
}

pub(super) fn run(
    context: &CheckContext<'_>,
    args: &[OsString],
    doctest: bool,
    report: &mut CheckReport,
) -> Result<()> {
    let runner = TestRunner::discover(context)?;
    let mut tests = runner.command(context, args);
    context.run(report, "rust-tests", "test", runner.name(), &mut tests)?;
    if runner == TestRunner::Nextest && doctest {
        let mut docs = doctest_command(context, args);
        context.run(
            report,
            "rust-doctests",
            "doctest",
            "affected crates",
            &mut docs,
        )?;
    } else if runner == TestRunner::Nextest {
        report.skip(
            "rust-doctests",
            "affected crates have no documentation test targets",
        );
    }
    Ok(())
}

impl TestRunner {
    fn discover(context: &CheckContext<'_>) -> Result<Self> {
        let mut probe = context.command("cargo-nextest");
        probe.args(["nextest", "--version"]);
        context.prepare(&mut probe);
        Self::from_probe(probe.output().map(|output| output.status.success()))
    }

    fn from_probe(result: io::Result<bool>) -> Result<Self> {
        match result {
            Ok(true) => Ok(Self::Nextest),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::Cargo),
            Ok(false) => bail!("installed cargo-nextest failed its version probe"),
            Err(error) => Err(error).context("failed to inspect installed cargo-nextest"),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Cargo => "affected crates (Cargo)",
            Self::Nextest => "affected crates (Nextest)",
        }
    }

    fn command(self, context: &CheckContext<'_>, args: &[OsString]) -> Command {
        match self {
            Self::Cargo => cargo_command(context, &["test"], args),
            Self::Nextest => {
                let mut command = cargo_command(context, &["nextest", "run"], args);
                command.args([
                    "--no-fail-fast",
                    "--retries",
                    "0",
                    "--no-tests=pass",
                    "--ignore-default-filter",
                ]);
                command
            }
        }
    }
}

fn doctest_command(context: &CheckContext<'_>, args: &[OsString]) -> Command {
    let mut command = cargo_command(context, &["test"], args);
    command.arg("--doc");
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_discovery_falls_back_only_when_the_executable_is_absent() {
        assert_eq!(
            TestRunner::from_probe(Ok(true)).unwrap(),
            TestRunner::Nextest
        );
        assert_eq!(
            TestRunner::from_probe(Err(io::ErrorKind::NotFound.into())).unwrap(),
            TestRunner::Cargo
        );
        for probe in [
            Ok(false),
            Err(io::ErrorKind::PermissionDenied.into()),
            Err(io::ErrorKind::InvalidData.into()),
        ] {
            assert!(TestRunner::from_probe(probe).is_err());
        }
    }

    #[test]
    fn runners_preserve_cargo_selection_cache_and_documentation_coverage() {
        let directory = tempfile::tempdir().unwrap();
        let affected = directory.path().join("affected.json");
        let cancellation = qol_process::CancellationToken::new();
        let context =
            super::super::tests::test_context(directory.path(), &affected, &cancellation, true);
        for selection in [
            Vec::new(),
            vec!["-p", "qol", "--features", "qol/dev"],
            vec![
                "--workspace",
                "--exclude",
                "keyremap",
                "--no-default-features",
            ],
        ] {
            let selection = selection
                .into_iter()
                .map(OsString::from)
                .collect::<Vec<_>>();
            for (mut command, prefix, suffix) in [
                (
                    TestRunner::Cargo.command(&context, &selection),
                    vec!["test", "--locked"],
                    vec![],
                ),
                (
                    TestRunner::Nextest.command(&context, &selection),
                    vec!["nextest", "run", "--locked"],
                    vec![
                        "--no-fail-fast",
                        "--retries",
                        "0",
                        "--no-tests=pass",
                        "--ignore-default-filter",
                    ],
                ),
                (
                    doctest_command(&context, &selection),
                    vec!["test", "--locked"],
                    vec!["--doc"],
                ),
            ] {
                context.prepare(&mut command);
                let expected = prefix
                    .into_iter()
                    .map(OsString::from)
                    .chain(selection.iter().cloned())
                    .chain(suffix.into_iter().map(OsString::from))
                    .collect::<Vec<_>>();
                assert_eq!(
                    command.get_args().collect::<Vec<_>>(),
                    expected,
                    "{selection:?}"
                );
                let environment = command
                    .get_envs()
                    .collect::<std::collections::BTreeMap<_, _>>();
                assert_eq!(
                    environment.get(std::ffi::OsStr::new("CARGO_TARGET_DIR")),
                    Some(&Some(context.cargo_target.as_os_str()))
                );
                assert_eq!(
                    environment.get(std::ffi::OsStr::new("GIT_INDEX_FILE")),
                    Some(&None)
                );
            }
        }
    }
}
