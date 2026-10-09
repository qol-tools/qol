use std::collections::BTreeSet;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use qol_app_icon::ProcessEntry;
use qol_windowing::platform::windows::Window;

use super::launch::{
    launcher_args, plain_path, resolve_program, terminal_args, LaunchSpec, LaunchTag,
};
use super::report::{candidate_roots, image_stem, listed, tagged, Root};
use super::{probe_consoles, tabs, BACKEND_ID, CREATE_NO_WINDOW};
use crate::{SpawnRequest, SpawnSurface, TerminalError};

const TERMINAL: &str = "wt.exe";
const TERMINAL_HOST: &str = "windowsterminal";
const SPEC_PREFIX: &str = "qol-console-launch-";
const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
const ERROR_ACCESS_DENIED: i32 = 5;
const TERMINAL_TIMEOUT: Duration = Duration::from_secs(10);
const APPEAR_TIMEOUT: Duration = Duration::from_secs(20);
const APPEAR_POLL: Duration = Duration::from_millis(200);

pub(super) fn spawn(request: &SpawnRequest) -> Result<i32, TerminalError> {
    let executable = std::env::current_exe()
        .map_err(|error| failed(format!("cannot locate this binary: {error}")))?;
    let executable_text = utf8(&executable, "binary path")?;
    let cwd = plain_path(utf8(&request.cwd, "cwd")?);
    let tag = LaunchTag {
        nonce: nonce(),
        identity: request.identity.clone(),
    };
    let token = tag.encode();
    let spec_path = std::env::temp_dir().join(format!("{SPEC_PREFIX}{}.json", tag.nonce));
    write_spec(&spec_path, &spec(request))
        .map_err(|error| failed(format!("cannot write the launch spec: {error}")))?;
    let launcher = launcher_args(&token, utf8(&spec_path, "spec path")?);
    let own = i32::try_from(std::process::id()).unwrap_or_default();
    let before: BTreeSet<i32> = candidate_roots(&qol_app_icon::processes(), own)
        .into_iter()
        .map(|root| root.pid)
        .collect();
    let previous = Window::foreground();
    let terminal = terminal();
    let keep_tab = previous
        .filter(|_| terminal.is_some() && request.identity.surface == SpawnSurface::Tab)
        .filter(|window| is_terminal_window(*window))
        .and_then(|window| window.id().as_u32());
    let launch = || {
        let mut child = match &terminal {
            Some(terminal) => {
                let args = terminal_args(
                    request.identity.surface,
                    request.title.as_deref(),
                    &cwd,
                    executable_text,
                    &launcher,
                );
                open_terminal(terminal, &args).map(|()| None)
            }
            None => open_console(&executable, &launcher, &cwd).map(Some),
        }?;
        appeared(
            &tag,
            &before,
            &image_stem(&file_name(&executable)),
            child.as_mut(),
        )
    };
    let outcome = match keep_tab {
        Some(window) => tabs::keeping_selection(window, launch),
        None => launch(),
    };
    if outcome.is_err() {
        let _ = std::fs::remove_file(&spec_path);
    }
    if let Some(previous) = previous {
        if outcome.is_ok()
            && Window::foreground().and_then(|now| now.id().as_u32()) != previous.id().as_u32()
        {
            previous.activate();
        }
    }
    outcome
}

fn spec(request: &SpawnRequest) -> LaunchSpec {
    let path = std::env::var("PATH")
        .ok()
        .map(|path| ("PATH".to_owned(), path));
    LaunchSpec {
        program: request.launch.program.clone(),
        args: request.launch.args.clone(),
        env: path
            .into_iter()
            .chain(request.launch.env.iter().cloned())
            .collect(),
        title: request.title.clone(),
    }
}

fn write_spec(path: &Path, spec: &LaunchSpec) -> io::Result<()> {
    let text = serde_json::to_string(spec).map_err(io::Error::other)?;
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(text.as_bytes())
}

fn terminal() -> Option<PathBuf> {
    let path = std::env::var("PATH").ok();
    resolve_program(TERMINAL, path.as_deref(), None, |file| {
        std::fs::symlink_metadata(file).is_ok()
    })
}

fn is_terminal_window(window: Window) -> bool {
    window
        .pid()
        .and_then(|pid| i32::try_from(pid).ok())
        .and_then(qol_app_icon::process_executable)
        .is_some_and(|path| image_stem(&file_name(&path)) == TERMINAL_HOST)
}

fn open_terminal(terminal: &Path, args: &[String]) -> Result<(), TerminalError> {
    let mut child = Command::new(terminal)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|error| failed(format!("cannot run {}: {error}", terminal.display())))?;
    let status = qol_process::wait_for_exit_or_terminate(&mut child, TERMINAL_TIMEOUT)
        .map_err(|error| failed(format!("{TERMINAL} did not finish: {error}")))?;
    if status.success() {
        return Ok(());
    }
    Err(failed(format!("{TERMINAL} exited with {status}")))
}

fn open_console(executable: &Path, launcher: &[String], cwd: &str) -> Result<Child, TerminalError> {
    let start = |flags: u32| {
        Command::new(executable)
            .args(launcher)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(flags)
            .spawn()
    };
    start(CREATE_NEW_CONSOLE | CREATE_BREAKAWAY_FROM_JOB)
        .or_else(|error| match error.raw_os_error() {
            Some(ERROR_ACCESS_DENIED) => start(CREATE_NEW_CONSOLE),
            _ => Err(error),
        })
        .map_err(|error| failed(format!("cannot open a console window: {error}")))
}

fn appeared(
    tag: &LaunchTag,
    before: &BTreeSet<i32>,
    launcher: &str,
    mut child: Option<&mut Child>,
) -> Result<i32, TerminalError> {
    let own = i32::try_from(std::process::id()).unwrap_or_default();
    let started = Instant::now();
    loop {
        let table = qol_app_icon::processes();
        let roots = candidate_roots(&table, own);
        let fresh = fresh_roots(&table, &roots, before, launcher);
        if !fresh.is_empty() {
            if let Ok(reports) = probe_consoles(&fresh) {
                if let Some(report) = listed(&reports, &roots)
                    .into_iter()
                    .find(|report| tagged(report).as_ref() == Some(tag))
                {
                    return Ok(report.root);
                }
            }
        }
        if let Some(status) = child
            .as_mut()
            .and_then(|child| child.try_wait().ok().flatten())
        {
            return Err(failed(format!(
                "the console launcher exited with {status} before the session appeared"
            )));
        }
        if started.elapsed() >= APPEAR_TIMEOUT {
            return Err(failed(format!(
                "the spawned session did not appear within {}s",
                APPEAR_TIMEOUT.as_secs()
            )));
        }
        std::thread::sleep(APPEAR_POLL);
    }
}

fn fresh_roots(
    table: &[ProcessEntry],
    roots: &[Root],
    before: &BTreeSet<i32>,
    launcher: &str,
) -> Vec<i32> {
    roots
        .iter()
        .map(|root| root.pid)
        .filter(|pid| !before.contains(pid))
        .filter(|pid| {
            table
                .iter()
                .any(|process| process.pid == *pid && image_stem(&process.name) == launcher)
        })
        .collect()
}

fn nonce() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    format!("{}-{nanos}", std::process::id())
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn utf8<'a>(path: &'a Path, what: &str) -> Result<&'a str, TerminalError> {
    path.to_str()
        .ok_or_else(|| failed(format!("{what} `{}` is not valid UTF-8", path.display())))
}

fn failed(message: String) -> TerminalError {
    TerminalError::SpawnFailed {
        backend: BACKEND_ID.clone(),
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{CliLaunchProgram, CliToolId};
    use crate::{SpawnIdentity, SpawnKey};

    fn process(pid: i32, parent_pid: i32, name: &str) -> ProcessEntry {
        ProcessEntry {
            pid,
            parent_pid,
            name: name.to_owned(),
        }
    }

    fn identity() -> SpawnIdentity {
        SpawnIdentity {
            key: SpawnKey::new("lane-1").unwrap(),
            tool: CliToolId::new("claude").unwrap(),
            surface: SpawnSurface::Tab,
        }
    }

    #[test]
    fn only_new_launcher_roots_are_probed() {
        let table = [
            process(10, 1, "WindowsTerminal.exe"),
            process(11, 10, "pwsh.exe"),
            process(12, 10, "qol.exe"),
            process(13, 10, "QOL.EXE"),
            process(14, 10, "cmd.exe"),
        ];
        let roots: Vec<Root> = [11, 12, 13, 14]
            .into_iter()
            .map(|pid| Root { pid, hosted: true })
            .collect();
        let before = BTreeSet::from([12]);
        assert_eq!(fresh_roots(&table, &roots, &before, "qol"), vec![13]);
    }

    #[test]
    fn the_spec_carries_the_caller_path_before_the_launch_environment() {
        let request = SpawnRequest {
            identity: identity(),
            launch: CliLaunchProgram {
                program: "claude".to_owned(),
                args: vec!["say \"hi\"; & done".to_owned()],
                env: vec![("CLAUDE_CONFIG_DIR".to_owned(), r"C:\c".to_owned())],
            },
            cwd: PathBuf::from(r"C:\work"),
            title: Some("Lane".to_owned()),
        };
        let spec = spec(&request);
        assert_eq!(spec.program, "claude");
        assert_eq!(spec.args, request.launch.args);
        assert_eq!(spec.title.as_deref(), Some("Lane"));
        assert_eq!(spec.env.last(), request.launch.env.last());
        if std::env::var("PATH").is_ok() {
            assert_eq!(spec.env[0].0, "PATH");
        }
    }
}
