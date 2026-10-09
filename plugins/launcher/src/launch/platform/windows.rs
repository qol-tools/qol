use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::{null, null_mut};

use windows_sys::w;
use windows_sys::Win32::System::Com::{
    CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

const SHELL_EXECUTE_FIRST_SUCCESS: usize = 33;
const SE_ERR_FNF: usize = 2;
const SE_ERR_PNF: usize = 3;
const SE_ERR_ACCESSDENIED: usize = 5;

pub(crate) fn daemon_action_args(_path: &Path, exec: &[String]) -> Option<(String, String)> {
    super::daemon_exec_args(exec).map(|(target, action)| (target.to_string(), action.to_string()))
}

pub(crate) fn launch_app(_path: &Path, exec: &[String]) -> io::Result<()> {
    let Some(target) = exec.first() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "application has no executable",
        ));
    };
    let file = wide(OsStr::new(target));
    let directory = qol_platform::launch_working_dir().map(|dir| wide(dir.as_os_str()));
    std::thread::Builder::new()
        .name("shell-execute".into())
        .spawn(move || {
            if let Err(error) = shell_execute(&file, directory.as_deref()) {
                log::warn!("launch failed: {error}");
            }
        })
        .map(|_| ())
}

fn shell_execute(file: &[u16], directory: Option<&[u16]>) -> io::Result<()> {
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
            null(),
            directory.map_or(null(), <[u16]>::as_ptr),
            SW_SHOWNORMAL,
        )
    } as usize;
    if com >= 0 {
        unsafe { CoUninitialize() };
    }
    shell_execute_result(result)
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
