mod daemon;
mod download;
mod install;
mod manifest;
mod platform;
mod tray;

use anyhow::{bail, Context, Result};
use install::RecordFile;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub(crate) const USAGE: &str = "qol pr try <number> | qol pr try --off";
const REPO: &str = "qol-tools/qol";

#[derive(Debug, PartialEq, Eq)]
enum Request {
    Try(u64),
    Off,
}

pub(crate) fn run(args: &[OsString]) -> Result<()> {
    let request = parse(args)?;
    let build = platform::host_build()?;
    let places = install::Places::current()?;
    match request {
        Request::Try(number) => try_build(number, build, &places),
        Request::Off => turn_off(&places),
    }
}

fn parse(args: &[OsString]) -> Result<Request> {
    let words = args
        .iter()
        .map(|arg| arg.to_str().context("non-UTF-8 argument for qol pr"))
        .collect::<Result<Vec<_>>>()?;
    match words.as_slice() {
        ["try", "--off"] => Ok(Request::Off),
        ["try", number] => number
            .trim_start_matches('#')
            .parse()
            .map(Request::Try)
            .with_context(|| format!("`{number}` is not a pull request number\nusage: {USAGE}")),
        _ => bail!("usage: {USAGE}"),
    }
}

fn try_build(number: u64, platform: &platform::Build, places: &install::Places) -> Result<()> {
    let mut state = install::State::load(&places.state)?;
    if let Some(other) = state.pull_request.filter(|other| *other != number) {
        bail!("pull request #{other} is still installed; run `qol pr try --off` first");
    }
    let pull = download::pull_request(REPO, number)?;
    let head = pull.head_ref_oid.clone();
    let run = download::live_test_run(REPO, &pull)?;
    let dir = places.download_dir(number, &head);
    download::fetch(REPO, &run, platform, &dir)?;
    let build = manifest::read(&dir.join(manifest::FILE))?;
    build.check(number, &head, platform.target)?;
    let tray_build = if build.tray.is_empty() {
        None
    } else {
        Some(platform::tray_binary(&dir.join("tray"))?)
    };
    if let Some(source) = &tray_build {
        install::verify_binary(source, tray::BINARY)?;
        platform::prepare_tray(source)?;
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
        println!("`qol pr try --off` puts the replaced builds and the installed tray back");
    }
    Ok(())
}

fn turn_off(places: &install::Places) -> Result<()> {
    let mut state = install::State::load(&places.state)?;
    let Some(number) = state.pull_request else {
        println!("no pull request build is installed");
        return Ok(());
    };
    println!("putting back the builds pull request #{number} replaced");
    let tray_build = state.tray.clone();
    while let Some(swapped) = state.plugins.last().cloned() {
        let restored = install::restore(&swapped)?;
        state.plugins.pop();
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
        let put_back = tray::put_back(&try_binary, &installed_tray(&state)?)?;
        println!("  {}: {}", tray::BINARY, put_back.describe());
    }
    install::State::clear(&places.state)?;
    Ok(())
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
        assert_eq!(parse(&words(&["try", "412"])).unwrap(), Request::Try(412));
        assert_eq!(parse(&words(&["try", "#412"])).unwrap(), Request::Try(412));
        assert_eq!(parse(&words(&["try", "--off"])).unwrap(), Request::Off);
        let refused: [&[&str]; 5] = [
            &["try"],
            &["try", "abc"],
            &["try", "412", "413"],
            &["get", "412"],
            &[],
        ];
        for bad in refused {
            assert!(parse(&words(bad)).is_err(), "{bad:?} must be refused");
        }
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
