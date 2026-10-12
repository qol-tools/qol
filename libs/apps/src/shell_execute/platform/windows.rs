use std::ffi::{OsStr, OsString};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::mpsc;
use std::time::Duration;

use qol_platform::native::com::{Apartment, ComApartment};
use qol_platform::native::wide::wide_nul;
use windows_sys::w;
use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, CREATE_NEW_CONSOLE, CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION,
    STARTF_TITLEISLINKNAME, STARTF_USESHOWWINDOW, STARTUPINFOW,
};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::shell_link::{LinkTarget, ShellLink};

const SHELL_EXECUTE_FIRST_SUCCESS: usize = 33;
const SHELL_EXECUTE_WAIT: Duration = Duration::from_secs(2);
const SE_ERR_FNF: usize = 2;
const SE_ERR_PNF: usize = 3;
const SE_ERR_ACCESSDENIED: usize = 5;
const ERROR_ELEVATION_REQUIRED: i32 = 740;

pub fn shell_execute(target: &OsStr, args: &[String]) -> io::Result<()> {
    let launch = Launch {
        target: target.to_os_string(),
        args: args.to_vec(),
        directory: qol_platform::launch_working_dir(),
    };
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name("shell-execute".into())
        .spawn(move || {
            let result = launch.run();
            if let Err(error) = &result {
                log::warn!(
                    "launch of {} failed: {error}",
                    Path::new(&launch.target).display()
                );
            }
            let _ = sender.send(result);
        })?;
    match receiver.recv_timeout(SHELL_EXECUTE_WAIT) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            log::info!("launch still starting after {SHELL_EXECUTE_WAIT:?}");
            Ok(())
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(io::Error::other(
            "the launch stopped before it reported a result",
        )),
    }
}

struct Launch {
    target: OsString,
    args: Vec<String>,
    directory: Option<PathBuf>,
}

impl Launch {
    fn run(&self) -> io::Result<()> {
        let Some(start) = self.direct_start() else {
            return self.shell_open();
        };
        let environment = child_environment(std::env::vars_os());
        match start.create_process(&environment) {
            Err(error) if error.raw_os_error() == Some(ERROR_ELEVATION_REQUIRED) => {
                self.shell_open()
            }
            Err(error) if start.link.is_some() => {
                log::info!(
                    "{} did not start directly ({error}); opening its link through the shell",
                    start.program
                );
                self.shell_open()
            }
            result => result,
        }
    }

    fn direct_start(&self) -> Option<DirectStart> {
        if let Some(program) = program_path(&self.target) {
            return Some(DirectStart {
                program: program.to_owned(),
                arguments: super::super::join_args(&self.args),
                directory: self.directory.clone(),
                show: SW_SHOWNORMAL as u32,
                link: None,
            });
        }
        let path = Path::new(&self.target);
        let is_link = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"));
        if !is_link {
            return None;
        }
        let link = ShellLink::read(path)?;
        let env = |name: &str| std::env::var(name).ok();
        let LinkTarget::Path(target) = link.target(path, env) else {
            return None;
        };
        let link_arguments = link.arguments.clone().unwrap_or_default();
        if link.run_as_user || link_arguments.contains('%') {
            return None;
        }
        let program = program_path(target.as_os_str())?.to_owned();
        let arguments = [link_arguments, super::super::join_args(&self.args)]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        Some(DirectStart {
            program,
            arguments,
            directory: link
                .resolved_working_dir(env)
                .or_else(|| self.directory.clone()),
            show: link.show_command,
            link: Some(self.target.clone()),
        })
    }

    fn shell_open(&self) -> io::Result<()> {
        let file = wide_nul(&self.target);
        let parameters =
            (!self.args.is_empty()).then(|| wide_nul(super::super::join_args(&self.args)));
        let directory = self.directory.as_ref().map(wide_nul);
        let _com = ComApartment::enter(Apartment::SingleThreaded);
        let result = unsafe {
            ShellExecuteW(
                null_mut(),
                w!("open"),
                file.as_ptr(),
                parameters.as_deref().map_or(null(), <[u16]>::as_ptr),
                directory.as_deref().map_or(null(), <[u16]>::as_ptr),
                SW_SHOWNORMAL,
            )
        } as usize;
        shell_execute_result(result)
    }
}

fn program_path(target: &OsStr) -> Option<&str> {
    let target = target.to_str()?;
    let path = Path::new(target);
    let program = path.extension().is_some_and(|extension| {
        extension.eq_ignore_ascii_case("exe") || extension.eq_ignore_ascii_case("com")
    });
    (path.is_absolute() && program).then_some(target)
}

fn launched_child_inherits(key: &OsStr) -> bool {
    key.to_str().is_none_or(|key| {
        key != qol_conventions::ENV_DAEMON_SOCKET
            && key != qol_conventions::ENV_INSTALL_ID
            && !qol_conventions::is_daemon_handoff_env(key)
    })
}

fn child_environment(variables: impl IntoIterator<Item = (OsString, OsString)>) -> Vec<u16> {
    let mut block = Vec::new();
    for (key, value) in variables {
        if !launched_child_inherits(&key) {
            continue;
        }
        block.extend(key.encode_wide());
        block.push(u16::from(b'='));
        block.extend(value.encode_wide());
        block.push(0);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    block
}

struct DirectStart {
    program: String,
    arguments: String,
    directory: Option<PathBuf>,
    show: u32,
    link: Option<OsString>,
}

impl DirectStart {
    fn create_process(&self, environment: &[u16]) -> io::Result<()> {
        let application = wide_nul(&self.program);
        let program = super::super::join_args(std::slice::from_ref(&self.program));
        let command_line = if self.arguments.is_empty() {
            program
        } else {
            format!("{program} {}", self.arguments)
        };
        let mut command_line = wide_nul(command_line);
        let directory = self.directory.as_ref().map(wide_nul);
        let mut title = self.link.as_ref().map(wide_nul);
        let link_flag = if title.is_some() {
            STARTF_TITLEISLINKNAME
        } else {
            0
        };
        let startup = STARTUPINFOW {
            cb: std::mem::size_of::<STARTUPINFOW>() as u32,
            dwFlags: STARTF_USESHOWWINDOW | link_flag,
            wShowWindow: self.show as u16,
            lpTitle: title
                .as_mut()
                .map_or(null_mut(), |title| title.as_mut_ptr()),
            ..unsafe { std::mem::zeroed() }
        };
        let mut process: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        let created = unsafe {
            CreateProcessW(
                application.as_ptr(),
                command_line.as_mut_ptr(),
                null(),
                null(),
                0,
                CREATE_NEW_CONSOLE | CREATE_UNICODE_ENVIRONMENT,
                environment.as_ptr().cast(),
                directory.as_deref().map_or(null(), <[u16]>::as_ptr),
                &startup,
                &mut process,
            )
        };
        if created == 0 {
            return Err(io::Error::last_os_error());
        }
        unsafe {
            CloseHandle(process.hThread);
            CloseHandle(process.hProcess);
        }
        Ok(())
    }
}

fn shell_execute_result(code: usize) -> io::Result<()> {
    match code {
        SHELL_EXECUTE_FIRST_SUCCESS.. => Ok(()),
        SE_ERR_FNF | SE_ERR_PNF => Err(io::ErrorKind::NotFound.into()),
        SE_ERR_ACCESSDENIED => Err(io::ErrorKind::PermissionDenied.into()),
        _ => Err(io::Error::other(format!(
            "ShellExecute failed with code {code}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    const LINK_FLAGS: std::ops::Range<usize> = 20..24;
    const RUN_AS_USER: u32 = 1 << 13;

    fn variable(key: &str, value: &str) -> (OsString, OsString) {
        (OsString::from(key), OsString::from(value))
    }

    fn system_program(relative: &str) -> PathBuf {
        Path::new(&std::env::var_os("SystemRoot").unwrap()).join(relative)
    }

    fn launch(target: &Path, args: &[&str]) -> Launch {
        Launch {
            target: target.as_os_str().to_owned(),
            args: args.iter().map(|arg| arg.to_string()).collect(),
            directory: None,
        }
    }

    fn write_links(links: &[(&Path, &Path, &str, &Path)]) {
        let script: String = links
            .iter()
            .map(|(link, target, arguments, folder)| {
                format!(
                    "$link = $shell.CreateShortcut('{}'); $link.TargetPath = '{}'; $link.Arguments = '{}'; $link.WorkingDirectory = '{}'; $link.Save(); ",
                    link.display(),
                    target.display(),
                    arguments.replace('\'', "''"),
                    folder.display()
                )
            })
            .collect();
        let script = format!("$shell = New-Object -ComObject WScript.Shell; {script}");
        let status = std::process::Command::new(system_program(
            r"System32\WindowsPowerShell\v1.0\powershell.exe",
        ))
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .status()
        .unwrap();
        assert!(status.success(), "{script}");
    }

    fn launched_environment() -> Vec<u16> {
        child_environment(std::env::vars_os().chain([
            variable(qol_conventions::ENV_DAEMON_SOCKET, "socket"),
            variable("QOL_LAUNCH_PROBE", "kept"),
        ]))
    }

    fn listing(output: &Path) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let listed = std::fs::read_to_string(output).unwrap_or_default();
            if listed.contains("QOL_LAUNCH_PROBE=kept") || Instant::now() >= deadline {
                return listed;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn only_absolute_executables_start_without_the_shell() {
        let cases = [
            (r"C:\Program Files\App\app.exe", true),
            (r"C:\Tools\legacy.COM", true),
            (r"\\server\share\tool.exe", true),
            ("notepad.exe", false),
            (r"C:\Users\me\Desktop\App.lnk", false),
            (r"C:\Users\me\notes.txt", false),
            (r"C:\Scripts\build.cmd", false),
            (
                "shell:AppsFolder\\Microsoft.WindowsTerminal_8wekyb3d8bbwe!App",
                false,
            ),
            ("https://example.com/", false),
        ];
        for (target, direct) in cases {
            assert_eq!(
                program_path(OsStr::new(target)).is_some(),
                direct,
                "{target}"
            );
        }
    }

    #[test]
    fn a_launched_program_gets_no_daemon_variables() {
        let block = child_environment([
            variable("PATH", r"C:\Windows"),
            variable(qol_conventions::ENV_DAEMON_SOCKET, "socket"),
            variable(qol_conventions::ENV_INSTALL_ID, "install"),
            variable(qol_conventions::ENV_DAEMON_LISTENER_FD, "7"),
            variable("=C:", r"C:\Users"),
        ]);
        let text = String::from_utf16(&block).unwrap();
        assert_eq!(text, "PATH=C:\\Windows\0=C:=C:\\Users\0\0");
    }

    #[test]
    fn an_empty_environment_is_still_a_terminated_block() {
        assert_eq!(child_environment([]), [0, 0]);
    }

    #[test]
    fn a_started_program_sees_the_environment_it_was_given() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("environment.txt");
        let listing_command = format!("set > {}", output.display());
        let start = launch(
            &system_program(r"System32\cmd.exe"),
            &["/c", &listing_command],
        )
        .direct_start()
        .unwrap();

        start.create_process(&launched_environment()).unwrap();

        let listed = listing(&output);
        assert!(listed.contains("QOL_LAUNCH_PROBE=kept"), "{listed}");
        assert!(
            !listed.contains(qol_conventions::ENV_DAEMON_SOCKET),
            "{listed}"
        );
    }

    #[test]
    fn a_link_to_a_program_starts_it_in_the_links_folder_without_the_daemon_variables() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("Listing.lnk");
        let shell = system_program(r"System32\cmd.exe");
        write_links(&[(&link, &shell, "/c set > launched.txt", dir.path())]);

        let start = launch(&link, &[])
            .direct_start()
            .expect("a link to a program starts directly");
        assert!(
            start
                .program
                .to_ascii_lowercase()
                .ends_with(r"\system32\cmd.exe"),
            "{}",
            start.program
        );
        assert_eq!(start.arguments, "/c set > launched.txt");
        assert_eq!(start.link.as_deref(), Some(link.as_os_str()));
        let with_extra = launch(&link, &["two words"]).direct_start().unwrap();
        assert_eq!(with_extra.arguments, r#"/c set > launched.txt "two words""#);

        start.create_process(&launched_environment()).unwrap();

        let listed = listing(&dir.path().join("launched.txt"));
        assert!(listed.contains("QOL_LAUNCH_PROBE=kept"), "{listed}");
        assert!(
            !listed.contains(qol_conventions::ENV_DAEMON_SOCKET),
            "{listed}"
        );
    }

    #[test]
    fn links_only_the_shell_can_honor_are_left_to_it() {
        let dir = tempfile::tempdir().unwrap();
        let shell = system_program(r"System32\cmd.exe");
        let notes = dir.path().join("notes.txt");
        std::fs::write(&notes, "notes").unwrap();
        let elevated = dir.path().join("Elevated.lnk");
        let variable = dir.path().join("Variable.lnk");
        let document = dir.path().join("Notes.lnk");
        write_links(&[
            (&elevated, &shell, "/c exit", dir.path()),
            (&variable, &shell, "/c echo %USERPROFILE%", dir.path()),
            (&document, &notes, "", dir.path()),
        ]);
        let mut bytes = std::fs::read(&elevated).unwrap();
        let flags = u32::from_le_bytes(bytes[LINK_FLAGS].try_into().unwrap()) | RUN_AS_USER;
        bytes[LINK_FLAGS].copy_from_slice(&flags.to_le_bytes());
        std::fs::write(&elevated, bytes).unwrap();

        for link in [
            &elevated,
            &variable,
            &document,
            &dir.path().join("Missing.lnk"),
        ] {
            assert!(
                launch(link, &[]).direct_start().is_none(),
                "{}",
                link.display()
            );
        }
    }
}
