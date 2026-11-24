use std::ffi::{c_void, OsString};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::ReplacePlatformOps;

const MAIN_THREAD_WAIT: Duration = Duration::from_secs(10);

pub(super) struct Platform;

type Successor = (PathBuf, Vec<OsString>);

extern "C" {
    static _dispatch_main_q: c_void;
    fn dispatch_async_f(
        queue: *const c_void,
        context: *mut c_void,
        work: extern "C" fn(*mut c_void),
    );
}

extern "C" fn start_successor(context: *mut c_void) {
    let (binary, args) = unsafe { *Box::from_raw(context as *mut Successor) };
    let error = super::super::spawn_successor_and_exit(&binary, &args);
    log::error!("restart into {} failed: {error}", binary.display());
    std::process::exit(1);
}

impl ReplacePlatformOps for Platform {
    fn replace_process(binary: &Path, args: &[OsString]) -> String {
        let successor: Successor = (binary.to_path_buf(), args.to_vec());
        let context = Box::into_raw(Box::new(successor)) as *mut c_void;
        unsafe {
            dispatch_async_f(&_dispatch_main_q, context, start_successor);
        }
        std::thread::sleep(MAIN_THREAD_WAIT);
        "the restart did not happen within the expected time".to_string()
    }
}
