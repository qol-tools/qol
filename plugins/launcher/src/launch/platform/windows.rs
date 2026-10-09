use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::{mpsc, Mutex};
use std::time::Duration;

use windows_sys::w;
use windows_sys::Win32::System::Com::{
    CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

const SHELL_EXECUTE_FIRST_SUCCESS: usize = 33;
const SHELL_EXECUTE_WAIT: Duration = Duration::from_secs(2);
const SE_ERR_FNF: usize = 2;
const SE_ERR_PNF: usize = 3;
const SE_ERR_ACCESSDENIED: usize = 5;

static LAUNCH_ENV: Mutex<()> = Mutex::new(());

pub(crate) fn daemon_action_args(_path: &Path, exec: &[String]) -> Option<(String, String)> {
    super::daemon_exec_args(exec).map(|(target, action)| (target.to_string(), action.to_string()))
}

pub(crate) fn launch_app(_path: &Path, exec: &[String]) -> io::Result<()> {
    let Some((target, args)) = exec.split_first() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "application has no executable",
        ));
    };
    let file = wide(OsStr::new(target));
    let parameters = (!args.is_empty()).then(|| wide(OsStr::new(&join_args(args))));
    let directory = qol_platform::launch_working_dir().map(|dir| wide(dir.as_os_str()));
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name("shell-execute".into())
        .spawn(move || {
            let result = shell_execute_scrubbed(&file, parameters.as_deref(), directory.as_deref());
            if let Err(error) = &result {
                log::warn!("launch failed: {error}");
            }
            let _ = sender.send(result);
        })?;
    receiver.recv_timeout(SHELL_EXECUTE_WAIT).unwrap_or(Ok(()))
}

fn shell_execute_scrubbed(
    file: &[u16],
    parameters: Option<&[u16]>,
    directory: Option<&[u16]>,
) -> io::Result<()> {
    let _env = LAUNCH_ENV
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut keys = qol_conventions::daemon_handoff_env_keys();
    keys.push(qol_conventions::ENV_DAEMON_SOCKET.into());
    keys.push(qol_conventions::ENV_INSTALL_ID.into());
    let saved: Vec<_> = keys
        .into_iter()
        .filter_map(|key| std::env::var_os(&key).map(|value| (key, value)))
        .collect();
    for (key, _) in &saved {
        std::env::remove_var(key);
    }
    let result = shell_execute(file, parameters, directory);
    for (key, value) in saved {
        std::env::set_var(key, value);
    }
    result
}

fn shell_execute(
    file: &[u16],
    parameters: Option<&[u16]>,
    directory: Option<&[u16]>,
) -> io::Result<()> {
    let com = unsafe {
        CoInitializeEx(
            null(),
            (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
        )
    };
    let result = unsafe {
        ShellExecuteW(
            null_mut(),
            w!("open"),
            file.as_ptr(),
            parameters.map_or(null(), <[u16]>::as_ptr),
            directory.map_or(null(), <[u16]>::as_ptr),
            SW_SHOWNORMAL,
        )
    } as usize;
    if com >= 0 {
        unsafe { CoUninitialize() };
    }
    shell_execute_result(result)
}

fn join_args(args: &[String]) -> String {
    args.iter()
        .map(|arg| quote_arg(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    let mut quoted = String::from('"');
    let mut backslashes = 0;
    for ch in arg.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                quoted.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.extend(std::iter::repeat_n('\\', backslashes));
                quoted.push(ch);
                backslashes = 0;
            }
        }
    }
    quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
    quoted.push('"');
    quoted
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

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}
