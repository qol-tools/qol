mod contracts;
mod daemon;
mod install;
mod manifest;
mod platform;
mod source;
mod unpack;

use anyhow::{Context, Result};
use install::Restored;
use qol_conventions::pr_try::short_sha;
use std::fmt;
use std::path::{Path, PathBuf};

pub use daemon::runs_binary;
pub use install::{Places, RecordFile, State};
pub use platform::{host_build, Build};
pub use source::{
    ready, succeeded, MissingArtifact, OpenPull, PullRequest, Run, Source, LIST_LIMIT, RUN_LIMIT,
    WORKFLOW,
};
pub use unpack::unpack_zip;

pub const REPO: &str = "qol-tools/qol";
pub const TRAY_BINARY: &str = qol_conventions::artifact::TRAY_HOST_BINARY_NAME;
const INSTALLED_TRAY_STARTS: &str = "the installed tray starts it";
const TRAY_BUILD_STARTS: &str = "the tray build starts it";

pub trait Host {
    fn say(&mut self, line: &str);
    fn run_tray(&mut self, binary: &Path) -> Result<String>;
    fn put_back_tray(
        &mut self,
        try_binary: &Path,
        installed: &Path,
        settle: &mut dyn FnMut() -> Result<()>,
    ) -> Result<TrayBack>;
    fn running_install(&self, downloads: &Path) -> Result<Option<PathBuf>>;
    fn catch_up(&mut self, put_back: &[String]);
}

#[derive(Debug, PartialEq, Eq)]
pub struct TrayBack {
    pub said: String,
    pub started: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RestartsTray {
    Carries(u64),
    Running { tried: u64, running: u64 },
}

impl fmt::Display for RestartsTray {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Carries(number) => write!(
                formatter,
                "pull request #{number} also replaces qol-tray, so nothing was installed; `qol pr try {number}` without --keep-tray restarts the tray on its build"
            ),
            Self::Running { tried, running } => write!(
                formatter,
                "the tray runs pull request #{running}'s build, so nothing was installed; `qol pr try {tried}` without --keep-tray restarts the tray"
            ),
        }
    }
}

impl std::error::Error for RestartsTray {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Try {
    pub number: u64,
    pub keep_tray: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Tried {
    Installed,
    Nothing,
}

struct Waiting {
    id: String,
    target: PathBuf,
}

pub fn try_build(
    source: &dyn Source,
    host: &mut dyn Host,
    request: Try,
    build: &Build,
    places: &Places,
) -> Result<Tried> {
    let number = request.number;
    if let Some(refusal) =
        running_tray_refusal(number, request.keep_tray, &State::load(&places.state)?)
    {
        return Err(refusal.into());
    }
    let pull = source::trusted(source.pull_request(number)?, REPO, number)?;
    let head = pull.head_sha.clone();
    let run = source::live_test_run(source, &pull, build)?;
    let dir = places.download_dir(number, &head);
    source::fetch(source, &run, build, &dir)?;
    let carried = manifest::read(&dir.join(manifest::FILE))?;
    carried.check(number, &head, build.target)?;
    if request.keep_tray && !carried.tray.is_empty() {
        return Err(RestartsTray::Carries(number).into());
    }
    let tray_build = if carried.tray.is_empty() {
        None
    } else {
        Some(platform::tray_binary(&dir.join("tray"))?)
    };
    if let Some(binary) = &tray_build {
        install::verify_binary(binary, TRAY_BINARY)?;
        platform::prepare_tray(binary)?;
    }
    let mut state = State::load(&places.state)?;
    let previous_tray = state.tray.clone().filter(|_| tray_build.is_none());
    let tray_starts = if tray_build.is_some() {
        Some(TRAY_BUILD_STARTS)
    } else {
        previous_tray.as_ref().map(|_| INSTALLED_TRAY_STARTS)
    };
    let mut waiting = Vec::new();
    let switching = state.pull_request.is_some_and(|other| other != number);
    let dropped = |swapped: &install::Swapped| switching || !carried.replaces(&swapped.id);
    if switching || state.plugins.iter().any(dropped) {
        let replaced = state.pull_request.unwrap_or(number);
        host.say(&if switching {
            format!("putting back the builds pull request #{replaced} replaced")
        } else {
            format!("putting back the plugins #{number}'s new build no longer changes")
        });
        let mut put_back = Vec::new();
        let restore_starts = state.tray.as_ref().and(tray_starts);
        let restored = restore_plugins(
            places,
            &mut state,
            host,
            &mut put_back,
            restore_starts,
            &mut waiting,
            &dropped,
        );
        let kept = put_back
            .into_iter()
            .filter(|id| !carried.replaces(id))
            .collect::<Vec<_>>();
        host.catch_up(&kept);
        restored?;
    }

    host.say(&format!("pull request #{number} at {}", short_sha(&head)));
    for plugin in &carried.plugins {
        let source = dir.join("plugins").join(&plugin.tag).join(&plugin.file);
        install::verify_binary(&source, &plugin.bin_name)?;
        platform::prepare_plugin(&source, &plugin.bin_name)?;
        let target = places.plugins.join(&plugin.id).join(&plugin.bin_name);
        if !target.is_file() {
            host.say(&format!("  {}: not installed here, skipped", plugin.id));
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
        if let Some(starts) = tray_starts {
            host.say(&format!(
                "  {} {}: installed; {starts}",
                plugin.id, plugin.version
            ));
            waiting.retain(|waits| waits.id != plugin.id);
            waiting.push(Waiting {
                id: plugin.id.clone(),
                target: target.clone(),
            });
            continue;
        }
        let restart = daemon::restart(&places.pids, &plugin.id, &target)?;
        host.say(&format!(
            "  {} {}: {}",
            plugin.id,
            plugin.version,
            restart.describe()
        ));
    }
    if let Some(binary) = tray_build {
        state.pull_request = Some(number);
        state.head_sha = Some(head.clone());
        state.tray = Some(binary.clone());
        if state.installed_tray.is_none() {
            state.installed_tray = host
                .running_install(places.downloads())?
                .or_else(|| first_installed(&qol_apps::tray_install::production_binaries()));
        }
        state.save(&places.state)?;
        let started = host.run_tray(&binary)?;
        host.say(&format!("  {}: {started}", carried.tray));
    } else if let Some(previous) = previous_tray {
        let installed = installed_tray(&state)?;
        let kept = state.clone();
        let state_path = places.state.clone();
        let back = keep_on_failure(
            host.put_back_tray(&previous, &installed, &mut || {
                state.tray = None;
                state.installed_tray = None;
                state.save(&state_path)
            }),
            &kept,
            &places.state,
        )?;
        host.say(&format!(
            "  {TRAY_BINARY}: no longer changed; {}",
            back.said
        ));
        restart_unless_started(places, host, &back, &waiting)?;
    }
    let state = State::load(&places.state)?;
    if state.plugins.is_empty() && state.tray.is_none() {
        State::clear(&places.state)?;
        return Ok(Tried::Nothing);
    }
    Ok(Tried::Installed)
}

pub fn newer_build(
    source: &dyn Source,
    build: &Build,
    number: u64,
    tried_head: &str,
) -> Result<Option<String>> {
    let pull = source::trusted(source.pull_request(number)?, REPO, number)?;
    if short_sha(&pull.head_sha) == short_sha(tried_head) {
        return Ok(None);
    }
    let built = source::live_test_run(source, &pull, build).is_ok();
    Ok(built.then_some(pull.head_sha))
}

pub fn turn_off(host: &mut dyn Host, places: &Places) -> Result<Vec<String>> {
    let mut state = State::load(&places.state)?;
    let Some(number) = state.pull_request else {
        host.say("no pull request build is installed");
        return Ok(Vec::new());
    };
    host.say(&format!(
        "putting back the builds pull request #{number} replaced"
    ));
    let mut put_back = Vec::new();
    let mut waiting = Vec::new();
    let tray_starts = state.tray.as_ref().map(|_| INSTALLED_TRAY_STARTS);
    let result = restore_plugins(
        places,
        &mut state,
        host,
        &mut put_back,
        tray_starts,
        &mut waiting,
        &|_| true,
    )
    .and_then(|()| put_back_tray(places, &state, host, &mut put_back, &waiting));
    match result {
        Ok(()) => Ok(put_back),
        Err(error) => {
            host.catch_up(&put_back);
            Err(error)
        }
    }
}

fn restore_plugins(
    places: &Places,
    state: &mut State,
    host: &mut dyn Host,
    put_back: &mut Vec<String>,
    tray_starts: Option<&str>,
    waiting: &mut Vec<Waiting>,
    restores: &dyn Fn(&install::Swapped) -> bool,
) -> Result<()> {
    let restoring: Vec<install::Swapped> = state
        .plugins
        .iter()
        .rev()
        .filter(|swapped| restores(swapped))
        .cloned()
        .collect();
    for swapped in restoring {
        contracts::restore(folder(&swapped.target)?, folder(&swapped.backup)?)?;
        let restored = install::restore(&swapped)?;
        state.plugins.retain(|kept| kept.id != swapped.id);
        put_back.push(swapped.id.clone());
        state.save(&places.state)?;
        if restored != Restored::PutBack {
            host.say(&format!("  {}: {}", swapped.id, restored.describe()));
        } else if let Some(starts) = tray_starts {
            host.say(&format!("  {}: put back; {starts}", swapped.id));
            waiting.push(Waiting {
                id: swapped.id.clone(),
                target: swapped.target.clone(),
            });
        } else {
            let restart = daemon::restart(&places.pids, &swapped.id, &swapped.target)?;
            host.say(&format!(
                "  {}: put back; {}",
                swapped.id,
                restart.describe()
            ));
        }
    }
    Ok(())
}

fn put_back_tray(
    places: &Places,
    state: &State,
    host: &mut dyn Host,
    put_back: &mut Vec<String>,
    waiting: &[Waiting],
) -> Result<()> {
    let Some(try_binary) = state.tray.clone() else {
        return State::clear(&places.state);
    };
    put_back.push(TRAY_BINARY.to_string());
    let installed = installed_tray(state)?;
    let back = keep_on_failure(
        host.put_back_tray(&try_binary, &installed, &mut || State::clear(&places.state)),
        state,
        &places.state,
    )?;
    host.say(&format!("  {TRAY_BINARY}: {}", back.said));
    restart_unless_started(places, host, &back, waiting)
}

fn restart_unless_started(
    places: &Places,
    host: &mut dyn Host,
    back: &TrayBack,
    waiting: &[Waiting],
) -> Result<()> {
    if back.started {
        return Ok(());
    }
    for plugin in waiting {
        let restart = daemon::restart(&places.pids, &plugin.id, &plugin.target)?;
        host.say(&format!("  {}: {}", plugin.id, restart.describe()));
    }
    Ok(())
}

fn keep_on_failure<T>(put_back: Result<T>, kept: &State, path: &Path) -> Result<T> {
    let Err(error) = put_back else {
        return put_back;
    };
    match kept.save(path) {
        Ok(()) => Err(error),
        Err(saved) => Err(error.context(format!(
            "the record of the tray build could not be kept either: {saved:#}"
        ))),
    }
}

fn running_tray_refusal(number: u64, keep_tray: bool, state: &State) -> Option<RestartsTray> {
    state
        .tray
        .as_ref()
        .filter(|_| keep_tray)
        .map(|_| RestartsTray::Running {
            tried: number,
            running: state.pull_request.unwrap_or(number),
        })
}

fn installed_tray(state: &State) -> Result<PathBuf> {
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

fn plain_name(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && !value.contains(['/', '\\'])
        && Path::new(value).file_name().is_some()
}

#[cfg(test)]
mod tests;
