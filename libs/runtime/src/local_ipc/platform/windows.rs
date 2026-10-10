use std::io;
use std::os::windows::io::AsRawSocket;
use std::path::Path;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::Networking::WinSock::{
    WSAGetLastError, WSAIoctl, SIO_AF_UNIX_GETPEERPID, WSAEINVAL, WSAEOPNOTSUPP,
};
use windows_sys::Win32::Security::{
    EqualSid, GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};

pub type LocalListener = uds_windows::UnixListener;
pub type LocalStream = uds_windows::UnixStream;

/// `sun_path` is 108 bytes on Windows as on Linux, minus the NUL.
pub(super) const MAX_SOCKET_PATH_BYTES: usize = 107;

pub(super) fn bind_listener(path: &Path) -> io::Result<LocalListener> {
    LocalListener::bind(path)
}

pub(super) fn authorize_peer(stream: &LocalStream) -> io::Result<()> {
    let Some(pid) = peer_pid(stream)? else {
        return Ok(());
    };
    if same_user(pid)? {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "local IPC peer belongs to a different user",
        ))
    }
}

fn peer_pid(stream: &LocalStream) -> io::Result<Option<u32>> {
    let mut pid = 0u32;
    let mut returned = 0u32;
    let status = unsafe {
        WSAIoctl(
            stream.as_raw_socket() as usize,
            SIO_AF_UNIX_GETPEERPID,
            std::ptr::null(),
            0,
            (&mut pid as *mut u32).cast(),
            std::mem::size_of::<u32>() as u32,
            &mut returned,
            std::ptr::null_mut(),
            None,
        )
    };
    if status == 0 {
        return Ok(Some(pid));
    }
    let code = unsafe { WSAGetLastError() };
    if code == WSAEOPNOTSUPP || code == WSAEINVAL {
        return Ok(None);
    }
    Err(io::Error::from_raw_os_error(code))
}

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

fn same_user(pid: u32) -> io::Result<bool> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(denied_or_last_error());
    }
    let process = Handle(process);
    let peer = token_user(&process)?;
    let current = token_user(&Handle(unsafe { GetCurrentProcess() }))?;
    Ok(unsafe { EqualSid(sid_of(&peer), sid_of(&current)) } != 0)
}

fn denied_or_last_error() -> io::Error {
    let error = io::Error::last_os_error();
    if error.kind() == io::ErrorKind::PermissionDenied {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "local IPC peer belongs to a different user",
        )
    } else {
        error
    }
}

fn sid_of(buffer: &[u64]) -> *mut core::ffi::c_void {
    unsafe { (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid }
}

fn token_user(process: &Handle) -> io::Result<Vec<u64>> {
    let mut token = std::ptr::null_mut();
    if unsafe { OpenProcessToken(process.0, TOKEN_QUERY, &mut token) } == 0 {
        return Err(denied_or_last_error());
    }
    let token = Handle(token);
    let mut needed = 0u32;
    unsafe { GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut needed) };
    let mut buffer = vec![0u64; (needed as usize).div_ceil(8).max(1)];
    let ok = unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            (buffer.len() * 8) as u32,
            &mut needed,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_user_peer_is_authorized() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("peer.sock");
        let listener = bind_listener(&path).unwrap();
        let _client = LocalStream::connect(&path).unwrap();
        let (accepted, _) = listener.accept().unwrap();
        assert!(authorize_peer(&accepted).is_ok());
    }
}
