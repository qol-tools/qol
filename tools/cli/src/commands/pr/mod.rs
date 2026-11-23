mod approve;
mod gh;
mod tray;
mod updates;

use anyhow::{bail, Context, Result};
use qol_conventions::pr_try::{CARRIES_TRAY_EXIT_CODE, KEEP_TRAY_FLAG};
use qol_pr_try::{
    Build, Host, Places, RestartsTray, Source, TrayBack, Tried, Try, REPO, TRAY_BINARY,
};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub(crate) const USAGE: &str =
    "qol pr list | qol pr try <number> [--keep-tray] | qol pr off | qol pr approve [number]";

#[derive(Debug, PartialEq, Eq)]
enum Request {
    List,
    Try { number: u64, keep_tray: bool },
    Off,
    Approve(Option<u64>),
}

pub(crate) fn exit_code(error: &anyhow::Error) -> i32 {
    if error.downcast_ref::<RestartsTray>().is_some() {
        CARRIES_TRAY_EXIT_CODE
    } else {
        1
    }
}

pub(crate) fn run(args: &[OsString], json: bool) -> Result<()> {
    let request = parse(args)?;
    if let Request::Approve(number) = request {
        println!("{}", approve::run(REPO, number)?);
        return Ok(());
    }
    let build = qol_pr_try::host_build()?;
    let source = gh::Gh::new(REPO);
    if request == Request::List {
        println!("{}", list(&source, build, json)?);
        return Ok(());
    }
    let places = Places::current()?;
    let mut console = Console;
    match request {
        Request::Try { number, keep_tray } => {
            let tried = Try { number, keep_tray };
            let outcome = qol_pr_try::try_build(&source, &mut console, tried, build, &places)?;
            println!("{}", summary(&outcome));
            Ok(())
        }
        Request::Approve(_) | Request::List => Ok(()),
        Request::Off => qol_pr_try::turn_off(&mut console, &places).map(|mut put_back| {
            if !put_back.is_empty() && !put_back.iter().any(|id| id == TRAY_BINARY) {
                put_back.push(TRAY_BINARY.to_string());
            }
            updates::catch_up(&put_back);
        }),
    }
}

fn summary(outcome: &Tried) -> &'static str {
    match outcome {
        Tried::Installed => "`qol pr off` puts the replaced builds and the installed tray back",
        Tried::Nothing => "nothing was installed",
    }
}

fn list(source: &dyn Source, build: &Build, json: bool) -> Result<String> {
    let ready = qol_pr_try::ready(source, build)?;
    if json {
        return serde_json::to_string(&ready).context("writing the pull request list as JSON");
    }
    if ready.is_empty() {
        return Ok(format!(
            "no open pull request has a build for {}",
            build.label()
        ));
    }
    Ok(ready
        .iter()
        .map(|pull| format!("#{} {} {}", pull.number, pull.head, pull.title))
        .collect::<Vec<_>>()
        .join("\n"))
}

struct Console;

impl Host for Console {
    fn say(&mut self, line: &str) {
        println!("{line}");
    }

    fn run_tray(&mut self, binary: &Path) -> Result<String> {
        tray::run_in_place(binary).map(|started| started.describe().to_string())
    }

    fn put_back_tray(
        &mut self,
        try_binary: &Path,
        installed: &Path,
        settle: &mut dyn FnMut() -> Result<()>,
    ) -> Result<TrayBack> {
        let put_back = tray::put_back(try_binary, installed)?;
        settle()?;
        Ok(TrayBack {
            said: put_back.describe().to_string(),
            started: matches!(put_back, tray::PutBack::Started(_)),
        })
    }

    fn running_install(&self, downloads: &Path) -> Result<Option<PathBuf>> {
        tray::running_install(downloads)
    }

    fn catch_up(&mut self, put_back: &[String]) {
        updates::catch_up(put_back);
    }
}

fn parse(args: &[OsString]) -> Result<Request> {
    let words = args
        .iter()
        .map(|arg| arg.to_str().context("non-UTF-8 argument for qol pr"))
        .collect::<Result<Vec<_>>>()?;
    match words.as_slice() {
        ["list"] => Ok(Request::List),
        ["off"] | ["try", "--off"] => Ok(Request::Off),
        ["try", number] => pull_number(number).map(|number| Request::Try {
            number,
            keep_tray: false,
        }),
        ["try", first, second] if *first == KEEP_TRAY_FLAG || *second == KEEP_TRAY_FLAG => {
            let number = if *first == KEEP_TRAY_FLAG {
                second
            } else {
                first
            };
            pull_number(number).map(|number| Request::Try {
                number,
                keep_tray: true,
            })
        }
        ["approve"] => Ok(Request::Approve(None)),
        ["approve", number] => pull_number(number).map(|number| Request::Approve(Some(number))),
        _ => bail!("usage: {USAGE}"),
    }
}

fn pull_number(word: &str) -> Result<u64> {
    word.trim_start_matches('#')
        .parse()
        .with_context(|| format!("`{word}` is not a pull request number\nusage: {USAGE}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn parse_reads_a_number_or_off() {
        let tried = |number, keep_tray| Request::Try { number, keep_tray };
        assert_eq!(parse(&words(&["try", "412"])).unwrap(), tried(412, false));
        assert_eq!(parse(&words(&["try", "#412"])).unwrap(), tried(412, false));
        assert_eq!(
            parse(&words(&["try", "412", "--keep-tray"])).unwrap(),
            tried(412, true)
        );
        assert_eq!(
            parse(&words(&["try", "--keep-tray", "#412"])).unwrap(),
            tried(412, true)
        );
        assert_eq!(parse(&words(&["list"])).unwrap(), Request::List);
        assert_eq!(parse(&words(&["off"])).unwrap(), Request::Off);
        assert_eq!(parse(&words(&["approve"])).unwrap(), Request::Approve(None));
        assert_eq!(
            parse(&words(&["approve", "#500"])).unwrap(),
            Request::Approve(Some(500))
        );
        assert_eq!(parse(&words(&["try", "--off"])).unwrap(), Request::Off);
        let refused: [&[&str]; 10] = [
            &["try"],
            &["list", "412"],
            &["try", "--keep-tray"],
            &["off", "412"],
            &["try", "abc"],
            &["approve", "abc"],
            &["approve", "5", "6"],
            &["try", "412", "413"],
            &["get", "412"],
            &[],
        ];
        for bad in refused {
            assert!(parse(&words(bad)).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn only_a_try_that_restarts_the_tray_exits_with_its_own_code() {
        let carries = RestartsTray::Carries(412);
        let running = RestartsTray::Running {
            tried: 413,
            running: 412,
        };
        assert_eq!(exit_code(&carries.into()), CARRIES_TRAY_EXIT_CODE);
        assert_eq!(exit_code(&running.into()), CARRIES_TRAY_EXIT_CODE);
        assert_eq!(exit_code(&anyhow::anyhow!("gh failed")), 1);
    }

    #[test]
    fn each_outcome_ends_with_its_own_line() {
        assert!(summary(&Tried::Installed).contains("qol pr off"));
        assert_eq!(summary(&Tried::Nothing), "nothing was installed");
    }
}
