use super::{PolicyError, ResidentPolicy};
use anyhow::Result;
#[cfg(target_os = "linux")]
use std::os::fd::OwnedFd;
use std::time::{Duration, Instant};

const LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(25);
const LOCK_RETRY_WINDOW: Duration = Duration::from_secs(10);

pub(crate) struct PolicyLockGuard {
    #[cfg(target_os = "linux")]
    _socket: OwnedFd,
}

impl std::fmt::Debug for PolicyLockGuard {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PolicyLockGuard")
    }
}

pub(crate) fn acquire(policy: &ResidentPolicy) -> Result<PolicyLockGuard> {
    let deadline = Instant::now() + LOCK_RETRY_WINDOW;
    loop {
        match try_acquire(policy) {
            Ok(guard) => return Ok(guard),
            Err(error) if is_busy(&error) && Instant::now() < deadline => {
                std::thread::sleep(LOCK_RETRY_INTERVAL);
            }
            Err(error) => return Err(error),
        }
    }
}

pub(crate) fn try_acquire(policy: &ResidentPolicy) -> Result<PolicyLockGuard> {
    #[cfg(target_os = "linux")]
    {
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
        Ok(PolicyLockGuard { _socket: socket })
    }
    #[cfg(not(target_os = "linux"))]
    {
        lock_name(policy)?;
        Err(PolicyError::PlatformUnsupported {
            policy: policy.id().to_string(),
        }
        .into())
    }
}

pub(crate) fn lock_name(policy: &ResidentPolicy) -> Result<String> {
    let base = format!("qol-resident-policy:{}", policy.id());
    #[cfg(any(test, feature = "sandbox"))]
    if let Ok(namespace) = std::env::var("QOL_POLICY_LOCK_NAMESPACE") {
        if !namespace.is_empty()
            && namespace.len() <= 64
            && namespace
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Ok(format!("{base}:{namespace}"));
        }
    }
    Ok(base)
}

fn is_busy(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<PolicyError>(),
        Some(PolicyError::Busy { .. })
    )
}
