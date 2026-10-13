use std::io;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, AsRawSocket, FromRawHandle, OwnedHandle};
use std::path::Path;

use qol_platform::native::security::TokenSid;
use windows_sys::Win32::Foundation::ERROR_INVALID_PARAMETER;
use windows_sys::Win32::Networking::WinSock::{WSAGetLastError, WSAIoctl, SIO_AF_UNIX_GETPEERPID};
use windows_sys::Win32::Storage::FileSystem::{
    FileAttributeTagInfo, GetFileInformationByHandleEx, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_ATTRIBUTE_TAG_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
};
use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

pub type LocalListener = uds_windows::UnixListener;
pub type LocalStream = uds_windows::UnixStream;

/// `sun_path` is 108 bytes on Windows as on Linux, minus the NUL.
pub(super) const MAX_SOCKET_PATH_BYTES: usize = 107;
const IO_REPARSE_TAG_AF_UNIX: u32 = 0x8000_0023;

pub(super) fn bind_listener(path: &Path) -> io::Result<LocalListener> {
    LocalListener::bind(path)
}

pub fn is_socket_file(path: &Path) -> bool {
    let Ok(file) = std::fs::OpenOptions::new()
        .access_mode(0)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
    else {
        return false;
    };
    let mut information = FILE_ATTRIBUTE_TAG_INFO {
        FileAttributes: 0,
        ReparseTag: 0,
    };
    let read = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileAttributeTagInfo,
            std::ptr::from_mut(&mut information).cast(),
            std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    };
    read != 0
        && information.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        && information.ReparseTag == IO_REPARSE_TAG_AF_UNIX
}

pub(super) fn authorize_peer(stream: &LocalStream) -> io::Result<()> {
    let pid = peer_pid(stream)?;
    if same_user(pid)? {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "local IPC peer belongs to a different user",
        ))
    }
}

pub fn peer_pid(stream: &LocalStream) -> io::Result<u32> {
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
        return Ok(pid);
    }
    let code = unsafe { WSAGetLastError() };
    Err(io::Error::from_raw_os_error(code))
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
    let process = unsafe { OwnedHandle::from_raw_handle(process) };
    let peer = TokenSid::user_of(process.as_raw_handle()).map_err(denied)?;
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

    #[test]
    fn the_peer_pid_is_the_process_across_the_socket() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("peer.sock");
        let listener = bind_listener(&path).unwrap();
        let client = LocalStream::connect(&path).unwrap();
        let _accepted = listener.accept().unwrap();
        assert_eq!(peer_pid(&client).unwrap(), std::process::id());
    }
}
