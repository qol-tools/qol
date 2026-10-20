use std::ffi::{c_char, c_void, CStr};

use anyhow::{bail, Result};

use crate::lock::Availability;

extern "C" {
    fn dlopen(path: *const c_char, mode: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

const RTLD_NOW: i32 = 2;
const LOGIN_FRAMEWORK: &CStr =
    c"/System/Library/PrivateFrameworks/login.framework/Versions/Current/login";
const LOCK_SYMBOL: &CStr = c"SACLockScreenImmediate";

type LockFn = unsafe extern "C" fn() -> i32;

fn resolve_lock() -> Option<LockFn> {
    let handle = unsafe { dlopen(LOGIN_FRAMEWORK.as_ptr(), RTLD_NOW) };
    if handle.is_null() {
        return None;
    }
    let symbol = unsafe { dlsym(handle, LOCK_SYMBOL.as_ptr()) };
    if symbol.is_null() {
        return None;
    }
    Some(unsafe { std::mem::transmute::<*mut c_void, LockFn>(symbol) })
}

pub fn lock_screen() -> Result<()> {
    let Some(lock) = resolve_lock() else {
        bail!("the macOS login framework does not export {LOCK_SYMBOL:?}");
    };
    unsafe { lock() };
    Ok(())
}

pub fn availability() -> Availability {
    if resolve_lock().is_some() {
        return Availability::Ready("The macOS login framework will lock the screen.".into());
    }
    Availability::Missing {
        detail: format!("The macOS login framework does not export {LOCK_SYMBOL:?}."),
        fix: "Lock with Control+Command+Q until a supported lock path is added.".into(),
    }
}
