pub(crate) mod warning;

use std::sync::Arc;
use std::time::Duration;

use core_foundation::base::{CFType, TCFType};
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use objc2::rc::autoreleasepool;
use objc2_app_kit::NSRunningApplication;
use qol_runtime::protocol::NotificationLevel;

use super::input::InputState;
use crate::platform::SecureInputHolder;
use warning::SecureInputWatch;

const POLL_INTERVAL: Duration = Duration::from_secs(1);
const UNKNOWN_APP: &str = "An app";

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn IsSecureEventInputEnabled() -> u8;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGSessionCopyCurrentDictionary() -> CFDictionaryRef;
}

pub(crate) fn enabled() -> bool {
    unsafe { IsSecureEventInputEnabled() != 0 }
}

pub(crate) fn holder() -> Option<SecureInputHolder> {
    if !enabled() {
        return None;
    }
    let pid = holder_pid();
    Some(SecureInputHolder {
        pid: pid.unwrap_or(0),
        app: pid
            .and_then(app_name)
            .unwrap_or_else(|| UNKNOWN_APP.to_string()),
    })
}

pub(crate) fn watch(input: Arc<InputState>) {
    std::thread::Builder::new()
        .name("keyremap-secure-input".into())
        .spawn(move || {
            let mut watch = SecureInputWatch::default();
            let client = qol_runtime::PlatformStateClient::from_env();
            loop {
                if let Some(toast) = watch.observe(holder().as_ref(), input.strategy.get()) {
                    log::warn!("{}", toast.title);
                    if !client.send_notification(&toast.title, &toast.body, NotificationLevel::Warn)
                    {
                        log::warn!("could not show the Secure Input warning in qol-tray");
                    }
                }
                std::thread::sleep(POLL_INTERVAL);
            }
        })
        .expect("failed to spawn the Secure Input watch thread");
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
