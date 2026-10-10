use std::io;
use std::os::windows::io::AsRawSocket;
use std::path::Path;

use qol_platform::native::security::TokenSid;
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, HANDLE};
use windows_sys::Win32::Networking::WinSock::{
    WSAGetLastError, WSAIoctl, SIO_AF_UNIX_GETPEERPID, WSAEINVAL, WSAEOPNOTSUPP,
};
use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

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
        let error = denied(io::Error::last_os_error());
        if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "local IPC peer exited before authorization",
            ));
        }
        return Err(error);
    }
    let process = Handle(process);
    let peer = TokenSid::user_of(process.0).map_err(denied)?;
    Ok(peer.same_as(&TokenSid::current_user().map_err(denied)?))
}

fn denied(error: io::Error) -> io::Error {
    if error.kind() == io::ErrorKind::PermissionDenied {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "local IPC peer belongs to a different user",
        )
    } else {
        error
    }
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
