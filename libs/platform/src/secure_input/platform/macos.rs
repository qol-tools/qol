use core_foundation::base::{CFType, TCFType};
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use objc2::rc::autoreleasepool;
use objc2_app_kit::NSRunningApplication;

use super::{SecureInputApi, SecureInputHolder};

const UNKNOWN_APP: &str = "An app";

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn IsSecureEventInputEnabled() -> u8;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGSessionCopyCurrentDictionary() -> CFDictionaryRef;
}

pub(crate) struct SecureInput;

impl SecureInputApi for SecureInput {
    fn enabled(&self) -> bool {
        unsafe { IsSecureEventInputEnabled() != 0 }
    }

    fn holder(&self) -> Option<SecureInputHolder> {
        if !self.enabled() {
            return None;
        }
        Some(holder_of(holder_pid(), app_name))
    }
}

fn holder_of(pid: Option<i32>, name_of: impl FnOnce(i32) -> Option<String>) -> SecureInputHolder {
    SecureInputHolder {
        pid: pid.unwrap_or(0),
        app: pid
            .and_then(name_of)
            .unwrap_or_else(|| UNKNOWN_APP.to_string()),
    }
}

fn holder_pid() -> Option<i32> {
    let dictionary = unsafe { CGSessionCopyCurrentDictionary() };
    if dictionary.is_null() {
        return None;
    }
    let dictionary: CFDictionary<CFString, CFType> =
        unsafe { CFDictionary::wrap_under_create_rule(dictionary) };
    let key = CFString::from_static_string("kCGSSessionSecureInputPID");
    dictionary.find(&key)?.downcast::<CFNumber>()?.to_i32()
}

fn app_name(pid: i32) -> Option<String> {
    autoreleasepool(|_| {
        NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
            .and_then(|app| app.localizedName())
            .map(|name| name.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_holder_carries_the_pid_and_the_app_name() {
        let cases = [
            ("named", Some(4242), Some("kitty"), 4242, "kitty"),
            (
                "pid without a running app",
                Some(4242),
                None,
                4242,
                UNKNOWN_APP,
            ),
            ("no pid", None, Some("kitty"), 0, UNKNOWN_APP),
            ("neither", None, None, 0, UNKNOWN_APP),
        ];
        for (label, pid, name, expected_pid, expected_app) in cases {
            let holder = holder_of(pid, |_| name.map(str::to_string));
            assert_eq!(holder.pid, expected_pid, "case: {label}");
            assert_eq!(holder.app, expected_app, "case: {label}");
        }
    }
}
