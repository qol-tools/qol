use super::super::{lock_name, PolicyError, ResidentPolicy};
use anyhow::Result;
use std::os::fd::OwnedFd;

pub(crate) struct HeldLock {
    _socket: OwnedFd,
}

pub(crate) fn try_acquire(policy: &ResidentPolicy) -> Result<HeldLock> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let name = lock_name(policy)?;
    if name.len() > 100 {
        return Err(PolicyError::LockFailure {
            policy: policy.id().to_string(),
            detail: "lock name too long".to_string(),
        }
        .into());
    }
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(PolicyError::LockFailure {
            policy: policy.id().to_string(),
            detail: format!(
                "failed to create the policy lock socket: {}",
                std::io::Error::last_os_error()
            ),
        }
        .into());
    }
    let socket = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    let sun_path_offset = std::mem::offset_of!(libc::sockaddr_un, sun_path);
    let name_bytes = name.as_bytes();
    let address_length = (sun_path_offset + 1 + name_bytes.len()) as libc::socklen_t;
    unsafe {
        std::ptr::copy_nonoverlapping(
            name_bytes.as_ptr() as *const libc::c_char,
            address.sun_path.as_mut_ptr().add(1),
            name_bytes.len(),
        );
    }
    let bind_result = unsafe {
        libc::bind(
            socket.as_raw_fd(),
            &address as *const libc::sockaddr_un as *const libc::sockaddr,
            address_length,
        )
    };
    if bind_result != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EADDRINUSE) {
            return Err(PolicyError::Busy {
                policy: policy.id().to_string(),
                detail: "another process holds the residency policy".to_string(),
            }
            .into());
        }
        return Err(PolicyError::LockFailure {
            policy: policy.id().to_string(),
            detail: format!("failed to bind the policy lock: {error}"),
        }
        .into());
    }
    Ok(HeldLock { _socket: socket })
}
