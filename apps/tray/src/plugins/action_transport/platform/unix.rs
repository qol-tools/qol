use qol_runtime::local_ipc::LocalStream;
use std::path::Path;
use std::time::Duration;

pub(super) const DEFAULT_IO_TIMEOUT: Duration = Duration::from_secs(10);
type DispatchResult<T> = Result<T, ()>;

pub(super) fn connect(endpoint: &Path, timeout: Duration) -> DispatchResult<LocalStream> {
    use std::os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::ffi::OsStrExt,
    };
    let bytes = endpoint.as_os_str().as_bytes();
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.is_empty() || bytes.contains(&0) || bytes.len() >= address.sun_path.len() {
        return Err(());
    }
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (slot, byte) in address.sun_path.iter_mut().zip(bytes) {
        *slot = *byte as libc::c_char;
    }
    let length = std::mem::offset_of!(libc::sockaddr_un, sun_path) + bytes.len() + 1;
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    {
        address.sun_len = length as u8;
    }
    let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if raw < 0 {
        return Err(());
    }
    let owned = unsafe { OwnedFd::from_raw_fd(raw) };
    if unsafe { libc::fcntl(raw, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
        return Err(());
    }
    let stream = LocalStream::from(owned);
    stream.set_nonblocking(true).map_err(|_| ())?;
    let result = unsafe {
        libc::connect(
            stream.as_raw_fd(),
            (&address as *const libc::sockaddr_un).cast(),
            length as libc::socklen_t,
        )
    };
    if result != 0 {
        if std::io::Error::last_os_error().raw_os_error() != Some(libc::EINPROGRESS) {
            return Err(());
        }
        let mut poll = libc::pollfd {
            fd: stream.as_raw_fd(),
            events: libc::POLLOUT,
            revents: 0,
        };
        let millis = timeout.as_millis().clamp(1, i32::MAX as u128) as i32;
        if unsafe { libc::poll(&mut poll, 1, millis) } <= 0
            || stream.take_error().map_err(|_| ())?.is_some()
        {
            return Err(());
        }
    }
    stream.set_nonblocking(false).map_err(|_| ())?;
    Ok(stream)
}
