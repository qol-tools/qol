mod approve;
mod contracts;
mod daemon;
mod download;
mod install;
mod list;
mod manifest;
mod platform;
mod tray;
mod updates;

use anyhow::{bail, Context, Result};
use install::RecordFile;
use qol_conventions::pr_try::KEEP_TRAY_FLAG;
use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

pub(crate) const USAGE: &str =
    "qol pr list | qol pr try <number> [--keep-tray] | qol pr off | qol pr approve [number]";
const REPO: &str = "qol-tools/qol";
const EXIT_CARRIES_TRAY: i32 = 3;

#[derive(Debug, PartialEq, Eq)]
enum Request {
    List,
    Try { number: u64, keep_tray: bool },
    Off,
    Approve(Option<u64>),
}

#[derive(Debug)]
struct CarriesTray(u64);

impl fmt::Display for CarriesTray {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "pull request #{} also replaces qol-tray, so nothing was installed; `qol pr try {}` without --keep-tray restarts the tray on its build",
            self.0, self.0
        )
    }
}

impl std::error::Error for CarriesTray {}

pub(crate) fn exit_code(error: &anyhow::Error) -> i32 {
    if error.downcast_ref::<CarriesTray>().is_some() {
        EXIT_CARRIES_TRAY
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
    let build = platform::host_build()?;
    if request == Request::List {
        println!("{}", list::run(REPO, build, json)?);
        return Ok(());
    }
    let places = install::Places::current()?;
    match request {
        Request::Try { number, keep_tray } => try_build(number, keep_tray, build, &places),
        Request::Approve(_) | Request::List => Ok(()),
        Request::Off => turn_off(&places).map(|mut put_back| {
            if !put_back.is_empty() && !put_back.iter().any(|id| id == tray::BINARY) {
                put_back.push(tray::BINARY.to_string());
            }
            updates::catch_up(&put_back);
        }),
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

fn try_build(
    number: u64,
    keep_tray: bool,
    platform: &platform::Build,
    places: &install::Places,
) -> Result<()> {
    let pull = download::pull_request(REPO, number)?;
    let head = pull.head_ref_oid.clone();
    let run = download::live_test_run(REPO, &pull, platform)?;
    let dir = places.download_dir(number, &head);
    download::fetch(REPO, &run, platform, &dir)?;
    let build = manifest::read(&dir.join(manifest::FILE))?;
    build.check(number, &head, platform.target)?;
    if keep_tray && !build.tray.is_empty() {
        return Err(CarriesTray(number).into());
    }
    let tray_build = if build.tray.is_empty() {
        None
    } else {
        Some(platform::tray_binary(&dir.join("tray"))?)
    };
    if let Some(source) = &tray_build {
        install::verify_binary(source, tray::BINARY)?;
        platform::prepare_tray(source)?;
    }
    let mut state = install::State::load(&places.state)?;
    if state.pull_request.is_some_and(|other| other != number) {
        let kept = turn_off(places)?
            .into_iter()
            .filter(|id| id != tray::BINARY && !build.replaces(id))
            .collect::<Vec<_>>();
        updates::catch_up(&kept);
        state = install::State::load(&places.state)?;
    }

    println!("pull request #{number} at {}", short(&head));
    if tray_build.is_none() {
        if let Some(previous) = state.tray.clone() {
            let put_back = tray::put_back(&previous, &installed_tray(&state)?)?;
            state.tray = None;
            state.installed_tray = None;
            state.save(&places.state)?;
            println!(
                "  {}: no longer changed; {}",
                tray::BINARY,
                put_back.describe()
            );
        }
    }
    for plugin in &build.plugins {
        let source = dir.join("plugins").join(&plugin.tag).join(&plugin.file);
        install::verify_binary(&source, &plugin.bin_name)?;
        platform::prepare_plugin(&source, &plugin.bin_name)?;
        let target = places.plugins.join(&plugin.id).join(&plugin.bin_name);
        if !target.is_file() {
            println!("  {}: not installed here, skipped", plugin.id);
            continue;
        }
        let backup = places.backup_for(&plugin.id, &plugin.bin_name);
        state.pull_request = Some(number);
        state.head_sha = Some(head.clone());
        let swap = install::Swap {
            id: &plugin.id,
            source: &source,
            target: &target,
            backup: &backup,
        };
        install::replace(&swap, &mut state, &places.state)?;
        if let Some(carried) = &plugin.contract_files {
            contracts::replace(&contracts::Swap {
                build: &dir.join("plugins").join(&plugin.tag),
                installed: folder(&target)?,
                backups: folder(&backup)?,
                carried,
            })?;
        }
        if tray_build.is_some() {
            println!(
                "  {} {}: installed; the tray build starts it",
                plugin.id, plugin.version
            );
            continue;
        }
        let restart = daemon::restart(&places.pids, &plugin.id, &target)?;
        println!("  {} {}: {}", plugin.id, plugin.version, restart.describe());
    }
    if let Some(source) = tray_build {
        state.pull_request = Some(number);
        state.head_sha = Some(head.clone());
        state.tray = Some(source.clone());
        if state.installed_tray.is_none() {
            state.installed_tray = tray::running_install(places.downloads())?
                .or_else(|| first_installed(&qol_apps::tray_install::production_binaries()));
        }
        state.save(&places.state)?;
        let started = tray::run_in_place(&source)?;
        println!("  {}: {}", build.tray, started.describe());
    }
    if state.plugins.is_empty() && state.tray.is_none() {
        println!("nothing was installed");
    } else {
        println!("`qol pr off` puts the replaced builds and the installed tray back");
    }
    Ok(())
}

fn turn_off(places: &install::Places) -> Result<Vec<String>> {
    let mut state = install::State::load(&places.state)?;
    let Some(number) = state.pull_request else {
        println!("no pull request build is installed");
        return Ok(Vec::new());
    };
    println!("putting back the builds pull request #{number} replaced");
    let mut put_back = Vec::new();
    match put_everything_back(places, &mut state, &mut put_back) {
        Ok(()) => Ok(put_back),
        Err(error) => {
            updates::catch_up(&put_back);
            Err(error)
        }
    }
}

fn put_everything_back(
    places: &install::Places,
    state: &mut install::State,
    put_back: &mut Vec<String>,
) -> Result<()> {
    let tray_build = state.tray.clone();
    while let Some(swapped) = state.plugins.last().cloned() {
        contracts::restore(folder(&swapped.target)?, folder(&swapped.backup)?)?;
        let restored = install::restore(&swapped)?;
        state.plugins.pop();
        put_back.push(swapped.id.clone());
        state.save(&places.state)?;
        if restored != install::Restored::PutBack {
            println!("  {}: {}", swapped.id, restored.describe());
        } else if tray_build.is_some() {
            println!("  {}: put back; the installed tray starts it", swapped.id);
        } else {
            let restart = daemon::restart(&places.pids, &swapped.id, &swapped.target)?;
            println!("  {}: put back; {}", swapped.id, restart.describe());
        }
    }
    if let Some(try_binary) = tray_build {
        put_back.push(tray::BINARY.to_string());
        let restarted = tray::put_back(&try_binary, &installed_tray(state)?)?;
        println!("  {}: {}", tray::BINARY, restarted.describe());
    }
    install::State::clear(&places.state)
}

fn installed_tray(state: &install::State) -> Result<PathBuf> {
    state
        .installed_tray
        .clone()
        .or_else(|| first_installed(&qol_apps::tray_install::production_binaries()))
        .context("could not find where the installed tray lives")
}

fn first_installed(binaries: &[PathBuf]) -> Option<PathBuf> {
    binaries
        .iter()
        .find(|binary| binary.is_file())
        .or_else(|| binaries.first())
        .cloned()
}

fn folder(path: &Path) -> Result<&Path> {
    path.parent()
        .with_context(|| format!("{} has no folder", path.display()))
}

fn short(sha: &str) -> &str {
    qol_conventions::pr_try::short_sha(sha)
}

fn plain_name(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && !value.contains(['/', '\\'])
        && Path::new(value).file_name().is_some()
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
    fn only_a_build_that_carries_the_tray_exits_with_its_own_code() {
        assert_eq!(exit_code(&CarriesTray(412).into()), EXIT_CARRIES_TRAY);
        assert_eq!(exit_code(&anyhow::anyhow!("gh failed")), 1);
        let message = CarriesTray(412).to_string();
        assert!(message.contains("#412 also replaces qol-tray"), "{message}");
    }

    #[test]
    fn the_installed_tray_is_the_first_one_on_disk() {
        let root = tempfile::tempdir().unwrap();
        let user = root
            .path()
            .join("home/Applications/QoL Tray.app/Contents/MacOS/qol-tray");
        let system = root
            .path()
            .join("Applications/QoL Tray.app/Contents/MacOS/qol-tray");
        let binaries = [user.clone(), system.clone()];
        assert_eq!(first_installed(&binaries), Some(user.clone()));
        std::fs::create_dir_all(system.parent().unwrap()).unwrap();
        std::fs::write(&system, b"tray").unwrap();
        assert_eq!(first_installed(&binaries), Some(system.clone()));
        std::fs::create_dir_all(user.parent().unwrap()).unwrap();
        std::fs::write(&user, b"tray").unwrap();
        assert_eq!(first_installed(&binaries), Some(user));
        assert_eq!(first_installed(&[]), None);
    }

    #[test]
    fn a_recorded_install_wins_over_the_default_places() {
        let recorded = PathBuf::from("/Applications/QoL Tray.app/Contents/MacOS/qol-tray");
        let state = install::State {
            installed_tray: Some(recorded.clone()),
            ..install::State::default()
        };
        assert_eq!(installed_tray(&state).unwrap(), recorded);
    }

    #[test]
    fn plain_name_refuses_paths() {
        for good in ["qol-cli-sessions", "qol-cli-sessions-v0.127.1"] {
            assert!(plain_name(good), "{good}");
        }
        for bad in ["", ".", "..", "a/b", "../a", "a\\b", "/abs"] {
            assert!(!plain_name(bad), "{bad}");
        }
    }
}
