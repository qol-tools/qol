use std::ffi::c_void;

use core_foundation::base::TCFType;
use core_foundation::string::{CFString, CFStringRef};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ConsoleUser {
    pub(super) uid: u32,
    pub(super) gid: u32,
}

#[link(name = "SystemConfiguration", kind = "framework")]
extern "C" {
    fn SCDynamicStoreCopyConsoleUser(
        store: *const c_void,
        uid: *mut u32,
        gid: *mut u32,
    ) -> CFStringRef;
}

pub(super) fn console_user() -> Option<ConsoleUser> {
    let (mut uid, mut gid) = (0, 0);
    let name = unsafe { SCDynamicStoreCopyConsoleUser(std::ptr::null(), &mut uid, &mut gid) };
    if name.is_null() {
        return None;
    }
    let name = unsafe { CFString::wrap_under_create_rule(name) }.to_string();
    console_owner(&name, uid, gid)
}

fn console_owner(name: &str, uid: u32, gid: u32) -> Option<ConsoleUser> {
    (name != "loginwindow" && uid != 0).then_some(ConsoleUser { uid, gid })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_login_window_and_root_are_not_console_users() {
        assert_eq!(console_owner("loginwindow", 501, 20), None);
        assert_eq!(console_owner("root", 0, 0), None);
        assert_eq!(
            console_owner("kaho", 501, 20),
            Some(ConsoleUser { uid: 501, gid: 20 })
        );
    }
}
