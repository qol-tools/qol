use std::ffi::c_void;
use std::ptr;

use core_foundation::base::TCFType;
use core_foundation::string::{CFString, CFStringRef};

#[link(name = "SystemConfiguration", kind = "framework")]
extern "C" {
    fn SCDynamicStoreCopyComputerName(store: *const c_void, encoding: *mut u32) -> CFStringRef;
}

pub(in super::super) fn device_name() -> Option<String> {
    let name = unsafe { SCDynamicStoreCopyComputerName(ptr::null(), ptr::null_mut()) };
    if name.is_null() {
        return None;
    }
    Some(unsafe { CFString::wrap_under_create_rule(name) }.to_string())
}
