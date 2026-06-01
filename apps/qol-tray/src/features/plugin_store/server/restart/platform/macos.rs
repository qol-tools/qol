use std::os::unix::process::CommandExt;
use std::path::Path;

pub(super) fn exec_restart(binary: &Path) -> Result<(), String> {
    let binary = binary.to_path_buf();
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();

    extern "C" {
        static _dispatch_main_q: std::ffi::c_void;
        fn dispatch_async_f(
            queue: *const std::ffi::c_void,
            context: *mut std::ffi::c_void,
            work: extern "C" fn(*mut std::ffi::c_void),
        );
    }

    // Box the closure data so we can pass it through a raw pointer.
    type ExecData = (std::path::PathBuf, Vec<std::ffi::OsString>);
    let data = Box::into_raw(Box::new((binary, args))) as *mut std::ffi::c_void;

    extern "C" fn do_exec(ctx: *mut std::ffi::c_void) {
        let (binary, args) = unsafe { *Box::from_raw(ctx as *mut ExecData) };
        let error = std::process::Command::new(&binary).args(&args).exec();
        eprintln!("[qol-tray] exec restart failed: {}", error);
        std::process::exit(1);
    }

    unsafe {
        dispatch_async_f(&_dispatch_main_q, data, do_exec);
    }

    // Park this thread — the main thread will exec and replace the process.
    std::thread::sleep(std::time::Duration::from_secs(10));
    Err("exec did not happen within expected time".to_string())
}
