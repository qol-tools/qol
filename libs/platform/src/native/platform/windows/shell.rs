use std::ffi::OsStr;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr::null;
use std::time::Duration;

use super::wide::wide_nul;
use windows_sys::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, GetProcessId, WaitForSingleObject, INFINITE,
};
use windows_sys::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS,
    SHELLEXECUTEINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{SW_HIDE, SW_SHOWNORMAL};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShellSpawn {
    pub elevate: bool,
    pub visible: bool,
}

#[derive(Debug)]
pub struct ShellProcess(OwnedHandle);

impl ShellProcess {
    pub fn id(&self) -> u32 {
        unsafe { GetProcessId(self.0.as_raw_handle()) }
    }

    pub fn wait_timeout(&self, timeout: Duration) -> io::Result<Option<u32>> {
        let millis = u32::try_from(timeout.as_millis())
            .map_or(INFINITE - 1, |millis| millis.min(INFINITE - 1));
        match unsafe { WaitForSingleObject(self.0.as_raw_handle(), millis) } {
            WAIT_OBJECT_0 => {}
            WAIT_TIMEOUT => return Ok(None),
            _ => return Err(io::Error::last_os_error()),
        }
        let mut code = 0u32;
        if unsafe { GetExitCodeProcess(self.0.as_raw_handle(), &mut code) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Some(code))
    }
}

pub fn shell_spawn(
    program: impl AsRef<OsStr>,
    arguments: &str,
    options: ShellSpawn,
) -> io::Result<ShellProcess> {
    let verb = wide_nul("runas");
    let file = wide_nul(program);
    let parameters = wide_nul(arguments);
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI;
    info.lpVerb = if options.elevate {
        verb.as_ptr()
    } else {
        null()
    };
    info.lpFile = file.as_ptr();
    info.lpParameters = parameters.as_ptr();
    info.nShow = if options.visible {
        SW_SHOWNORMAL
    } else {
        SW_HIDE
    };
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if info.hProcess.is_null() {
        return Err(io::Error::other("no process was started"));
    }
    Ok(ShellProcess(unsafe {
        OwnedHandle::from_raw_handle(info.hProcess)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd() -> std::path::PathBuf {
        let system_root = std::env::var_os("SystemRoot").unwrap();
        std::path::Path::new(&system_root).join(r"System32\cmd.exe")
    }

    #[test]
    fn a_shell_started_program_reports_its_exit_code() {
        let hidden = ShellSpawn {
            elevate: false,
            visible: false,
        };
        let process = shell_spawn(cmd(), "/d /c exit 7", hidden).unwrap();
        assert_ne!(process.id(), 0);
        assert_eq!(
            process.wait_timeout(Duration::from_secs(30)).unwrap(),
            Some(7)
        );
    }

    #[test]
    fn a_running_program_times_out_without_an_exit_code() {
        let hidden = ShellSpawn {
            elevate: false,
            visible: false,
        };
        let process = shell_spawn(cmd(), "/d /c ping -n 3 127.0.0.1 >nul", hidden).unwrap();
        assert_eq!(process.wait_timeout(Duration::ZERO).unwrap(), None);
        assert_eq!(
            process.wait_timeout(Duration::from_secs(30)).unwrap(),
            Some(0)
        );
    }

    #[test]
    fn a_missing_program_fails_without_a_dialog() {
        let hidden = ShellSpawn {
            elevate: false,
            visible: false,
        };
        let error = shell_spawn(r"C:\qol-missing\nothing-here.exe", "", hidden).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound, "{error}");
    }
}
