use std::ffi::{c_char, c_void, CStr};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

const ACTIVATE_SETTINGS: &str =
    "/System/Library/PrivateFrameworks/SystemAdministration.framework/Resources/activateSettings";
const PREFERENCE_PANES: &CStr =
    c"/System/Library/PrivateFrameworks/PreferencePanesSupport.framework/PreferencePanesSupport";
const SET_SCROLL_DIRECTION: &CStr = c"setSwipeScrollDirection";
const TRACKPAD_DRIVER: &CStr = c"AppleMultitouchTrackpadHIDEventDriver";
const MAIN_PORT: u32 = 0;

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOServiceMatching(name: *const c_char) -> *mut c_void;
    fn IOServiceGetMatchingService(main_port: u32, matching: *mut c_void) -> u32;
    fn IOObjectRelease(object: u32) -> i32;
}

pub(super) fn settings() -> Result<()> {
    let output = Command::new(ACTIVATE_SETTINGS)
        .arg("-u")
        .stdin(Stdio::null())
        .output()
        .context("the setting is saved, but macOS could not apply it until you log in again")?;
    if !output.status.success() {
        bail!(
            "the setting is saved, but macOS could not apply it until you log in again: activateSettings {}",
            output.status
        );
    }
    Ok(())
}

pub(super) fn scroll_direction(natural: bool) -> Result<()> {
    let set: extern "C" fn(u8) = unsafe {
        let library = libc::dlopen(PREFERENCE_PANES.as_ptr(), libc::RTLD_LAZY);
        if library.is_null() {
            bail!("natural scrolling is saved, but macOS applies it only after you log in again");
        }
        let symbol = libc::dlsym(library, SET_SCROLL_DIRECTION.as_ptr());
        if symbol.is_null() {
            bail!("natural scrolling is saved, but macOS applies it only after you log in again");
        }
        std::mem::transmute::<*mut c_void, extern "C" fn(u8)>(symbol)
    };
    set(u8::from(natural));
    Ok(())
}

pub(super) fn trackpad_present() -> bool {
    let service = unsafe {
        let matching = IOServiceMatching(TRACKPAD_DRIVER.as_ptr());
        if matching.is_null() {
            return false;
        }
        IOServiceGetMatchingService(MAIN_PORT, matching)
    };
    if service == 0 {
        return false;
    }
    unsafe { IOObjectRelease(service) };
    true
}
