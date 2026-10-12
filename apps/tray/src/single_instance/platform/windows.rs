use windows_sys::core::{w, PCWSTR};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, SetLastError, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, HANDLE,
};
use windows_sys::Win32::System::Threading::CreateMutexW;

const INSTANCE_MUTEX: PCWSTR = w!("Local\\QolTools.QolTray.Instance");

pub struct InstanceLock(HANDLE);

impl Drop for InstanceLock {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CloseHandle(self.0) };
        }
    }
}

pub fn claim() -> Option<InstanceLock> {
    claim_named(INSTANCE_MUTEX)
}

fn claim_named(name: PCWSTR) -> Option<InstanceLock> {
    unsafe { SetLastError(0) };
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name) };
    let error = unsafe { GetLastError() };
    let lock = InstanceLock(handle);
    if error == ERROR_ALREADY_EXISTS || (handle.is_null() && error == ERROR_ACCESS_DENIED) {
        return None;
    }
    if handle.is_null() {
        log::warn!(
            "single instance mutex unavailable, starting without it: {}",
            std::io::Error::from_raw_os_error(error as i32)
        );
    }
    Some(lock)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_claim_in_the_session_fails_until_the_first_is_released() {
        let name = w!("Local\\QolTools.QolTray.Instance.Test");

        let first = claim_named(name).expect("the first claim owns the mutex");
        assert!(claim_named(name).is_none());

        drop(first);
        assert!(claim_named(name).is_some());
    }
}
