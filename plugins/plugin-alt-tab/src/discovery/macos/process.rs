use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub(super) struct ProcessIdentity {
    pub pid: i32,
    pub start_time_us: u64,
}

#[repr(C)]
struct ProcBsdInfo {
    pbi_flags: u32,
    pbi_status: u32,
    pbi_xstatus: u32,
    pbi_pid: u32,
    pbi_ppid: u32,
    pbi_uid: u32,
    pbi_ruid: u32,
    pbi_gid: u32,
    pbi_rgid: u32,
    pbi_svuid: u32,
    pbi_svgid: u32,
    rfu_1: u32,
    pbi_comm: [u8; 16],
    pbi_name: [u8; 32],
    pbi_nfiles: u32,
    pbi_pgid: u32,
    pbi_pjobc: u32,
    e_tdev: u32,
    e_tpgid: u32,
    pbi_nice: i32,
    pbi_start_tvsec: u64,
    pbi_start_tvusec: u64,
}

#[link(name = "proc")]
extern "C" {
    fn proc_pidinfo(pid: i32, flavor: i32, arg: u64, buffer: *mut c_void, buffersize: i32) -> i32;
}

const PROC_PIDTBSDINFO: i32 = 3;

static KNOWN_WINDOW_IDS_BY_IDENTITY: OnceLock<Mutex<HashMap<ProcessIdentity, HashSet<u32>>>> =
    OnceLock::new();

pub(super) fn known_window_ids_by_identity(
) -> &'static Mutex<HashMap<ProcessIdentity, HashSet<u32>>> {
    KNOWN_WINDOW_IDS_BY_IDENTITY.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn process_identity(pid: i32) -> Option<ProcessIdentity> {
    if pid <= 0 {
        return None;
    }

    let mut info: ProcBsdInfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<ProcBsdInfo>() as i32;
    let read = unsafe {
        proc_pidinfo(
            pid,
            PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut ProcBsdInfo).cast::<c_void>(),
            size,
        )
    };
    if read != size {
        return None;
    }

    let start_time_us = info
        .pbi_start_tvsec
        .saturating_mul(1_000_000)
        .saturating_add(info.pbi_start_tvusec);

    Some(ProcessIdentity { pid, start_time_us })
}

pub(super) fn cached_process_identity(
    pid: i32,
    cache: &mut HashMap<i32, Option<ProcessIdentity>>,
) -> Option<ProcessIdentity> {
    if let Some(identity) = cache.get(&pid) {
        return *identity;
    }
    let identity = process_identity(pid);
    cache.insert(pid, identity);
    identity
}

/// Check if a PID belongs to a regular app (appears in Dock / Cmd+Tab).
/// Returns false for menu bar apps, background agents, and system services.
pub(super) fn is_regular_app(pid: i32) -> bool {
    use objc2_app_kit::{NSApplicationActivationPolicy, NSRunningApplication};

    objc2::rc::autoreleasepool(|_pool| {
        let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) else {
            return false;
        };
        app.activationPolicy() == NSApplicationActivationPolicy::Regular
    })
}

#[cfg(debug_assertions)]
pub(super) fn app_policy_debug(pid: i32) -> String {
    use objc2_app_kit::NSRunningApplication;

    objc2::rc::autoreleasepool(|_pool| {
        match NSRunningApplication::runningApplicationWithProcessIdentifier(pid) {
            None => "no-app".to_string(),
            Some(app) => format!("{:?}", app.activationPolicy()),
        }
    })
}

pub(super) fn is_app_hidden(pid: i32) -> bool {
    use objc2_app_kit::NSRunningApplication;

    objc2::rc::autoreleasepool(|_pool| {
        let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) else {
            return false;
        };
        app.isHidden()
    })
}
