use std::ffi::OsStr;
use std::io;
use std::marker::PhantomData;
use std::os::windows::ffi::OsStrExt;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{
    CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, TRUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Console::{
    AttachConsole, FreeConsole, GetStdHandle, SetConsoleCtrlHandler, SetStdHandle,
    STD_ERROR_HANDLE, STD_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};

const STD_HANDLES: [STD_HANDLE; 3] = [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE];

pub(super) struct Detached {
    saved: [HANDLE; 3],
}

impl Detached {
    pub(super) fn begin() -> Detached {
        let saved = STD_HANDLES.map(|kind| unsafe { GetStdHandle(kind) });
        unsafe { SetConsoleCtrlHandler(None, TRUE) };
        unsafe { FreeConsole() };
        Detached { saved }
    }

    pub(super) fn attach(&self, pid: u32) -> io::Result<Attached<'_>> {
        unsafe { FreeConsole() };
        if unsafe { AttachConsole(pid) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Attached(PhantomData))
    }
}

impl Drop for Detached {
    fn drop(&mut self) {
        unsafe { FreeConsole() };
        for (kind, handle) in STD_HANDLES.into_iter().zip(self.saved) {
            unsafe { SetStdHandle(kind, handle) };
        }
    }
}

pub(super) struct Attached<'scope>(PhantomData<&'scope Detached>);

impl Drop for Attached<'_> {
    fn drop(&mut self) {
        unsafe { FreeConsole() };
    }
}

pub(super) struct ConsoleFile(pub(super) HANDLE);

impl Drop for ConsoleFile {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

impl ConsoleFile {
    pub(super) fn open(name: &str) -> io::Result<ConsoleFile> {
        let name: Vec<u16> = OsStr::new(name).encode_wide().chain(Some(0)).collect();
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null(),
                OPEN_EXISTING,
                0,
                null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE || handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(ConsoleFile(handle))
    }
}
