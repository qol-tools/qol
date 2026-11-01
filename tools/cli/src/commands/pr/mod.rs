mod daemon;
mod download;
mod install;
mod manifest;
mod tray;

use anyhow::{bail, Context, Result};
use qol_platform::Os;
use std::ffi::OsString;
use std::path::Path;

pub(crate) const USAGE: &str = "qol pr try <number> | qol pr try --off";
const REPO: &str = "qol-tools/qol";
const BUILD_ARCH: &str = "x86_64";

#[derive(Debug, PartialEq, Eq)]
enum Request {
    Try(u64),
    Off,
}

pub(crate) fn run(args: &[OsString]) -> Result<()> {
    let request = parse(args)?;
    if Os::current() != Some(Os::Linux) || std::env::consts::ARCH != BUILD_ARCH {
        bail!("pull request builds are made for Linux {BUILD_ARCH} only");
    }
    let places = install::Places::current()?;
    match request {
        Request::Try(number) => try_build(number, &places),
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

fn try_build(number: u64, places: &install::Places) -> Result<()> {
    let mut state = install::State::load(&places.state)?;
    if let Some(other) = state.pull_request.filter(|other| *other != number) {
        bail!("pull request #{other} is still installed; run `qol pr try --off` first");
    }
    let pull = download::pull_request(REPO, number)?;
    let head = pull.head_ref_oid.clone();
    let run = download::live_test_run(REPO, &pull)?;
    let dir = places.download_dir(number, &head);
    download::fetch(REPO, &run, &dir)?;
    let build = manifest::read(&dir.join(manifest::FILE))?;
    build.check(number, &head)?;
    let tray_build = (!build.tray.is_empty()).then(|| dir.join("tray").join(tray::BINARY));
    if let Some(source) = &tray_build {
        install::verify_binary(source, tray::BINARY)?;
    }

    println!("pull request #{number} at {}", short(&head));
    if tray_build.is_none() {
        if let Some(previous) = state.tray.clone() {
            let put_back = tray::put_back(&previous, &installed_tray()?)?;
            state.tray = None;
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
        let put_back = tray::put_back(&try_binary, &installed_tray()?)?;
        println!("  {}: {}", tray::BINARY, put_back.describe());
    }
    install::State::clear(&places.state)?;
    Ok(())
}

fn installed_tray() -> Result<std::path::PathBuf> {
    qol_apps::tray_install::InstallProfile::Production
        .installed_binary()
        .context("could not find where the installed tray lives")
}

fn short(sha: &str) -> &str {
    sha.get(..9).unwrap_or(sha)
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
    fn plain_name_refuses_paths() {
        for good in ["qol-cli-sessions", "qol-cli-sessions-v0.127.1"] {
            assert!(plain_name(good), "{good}");
        }
        for bad in ["", ".", "..", "a/b", "../a", "a\\b", "/abs"] {
            assert!(!plain_name(bad), "{bad}");
        }
    }
}
