//! libproc-based fd resolver for macOS.
//!
//! Walks a `claude` process's open file descriptors via
//! `proc_pidinfo(PROC_PIDLISTFDS)` + `proc_pidfdinfo(PROC_PIDFDVNODEPATHINFO)`
//! and returns the first path matching the Claude session jsonl pattern:
//!
//! ```text
//! <HOME>/.claude/projects/<encoded-cwd>/<uuid>.jsonl
//! ```
//!
//! The libproc dylib is already loaded by qol-tray; we declare its
//! extern entry points here so the plugin doesn't need a build script
//! or a feature flag.

use std::ffi::CStr;
use std::os::raw::c_void;
use std::path::{Path, PathBuf};

use crate::resolver::ResolveError;

// Constants from <sys/proc_info.h>. Hand-mirrored because libc's macOS
// proc_info coverage is incomplete; the layout here matches what
// proc_pidinfo / proc_pidfdinfo expect.
const PROC_PIDLISTFDS: i32 = 1;
const PROC_PIDFDVNODEPATHINFO: i32 = 2;
const PROX_FDTYPE_VNODE: i32 = 1;
const MAXPATHLEN: usize = 1024;

#[repr(C)]
#[derive(Copy, Clone)]
struct ProcFdInfo {
    proc_fd: i32,
    proc_fdtype: i32,
}

#[repr(C)]
struct VInfoStat {
    // vinfo_stat is opaque to us; we never read it. Sized to match the
    // real layout so the surrounding struct alignment is right.
    _bytes: [u8; 144],
}

#[repr(C)]
struct VnodeInfoPath {
    vip_vi: VInfoStat,
    vip_path: [u8; MAXPATHLEN],
}

#[repr(C)]
struct VnodeFdInfoWithPath {
    // proc_fileinfo precedes the vnode_info_path block; opaque to us.
    pfi: [u8; 80],
    pvip: VnodeInfoPath,
}

#[link(name = "proc", kind = "dylib")]
extern "C" {
    fn proc_pidinfo(pid: i32, flavor: i32, arg: u64, buffer: *mut c_void, buffersize: i32) -> i32;
    fn proc_pidfdinfo(pid: i32, fd: i32, flavor: i32, buffer: *mut c_void, buffersize: i32) -> i32;
}

pub fn resolve_session_jsonl(pid: u32) -> Result<PathBuf, ResolveError> {
    let pid_i32 = pid as i32;
    let fds = list_fds(pid_i32)?;
    let home = home_dir().ok_or_else(|| ResolveError::OsError("$HOME not set".to_string()))?;
    let projects_root = home.join(".claude").join("projects");

    for fd in fds {
        if fd.proc_fdtype != PROX_FDTYPE_VNODE {
            continue;
        }
        let Some(path) = fd_path(pid_i32, fd.proc_fd) else {
            continue;
        };
        if matches_session_jsonl(&path, &projects_root) {
            return Ok(path);
        }
    }

    Err(ResolveError::NoSessionJsonl(pid))
}

/// Wrap `proc_pidinfo(PROC_PIDLISTFDS)` in two passes: ask for the
/// required size, then read into a sized Vec.
fn list_fds(pid: i32) -> Result<Vec<ProcFdInfo>, ResolveError> {
    let needed = unsafe { proc_pidinfo(pid, PROC_PIDLISTFDS, 0, std::ptr::null_mut(), 0) };
    if needed <= 0 {
        return Err(map_pid_error(pid));
    }

    let entry_size = std::mem::size_of::<ProcFdInfo>() as i32;
    let count = (needed / entry_size) as usize;
    let mut buf: Vec<ProcFdInfo> = Vec::with_capacity(count);
    let got = unsafe {
        proc_pidinfo(
            pid,
            PROC_PIDLISTFDS,
            0,
            buf.as_mut_ptr() as *mut c_void,
            (count * std::mem::size_of::<ProcFdInfo>()) as i32,
        )
    };
    if got <= 0 {
        return Err(map_pid_error(pid));
    }
    let returned = (got / entry_size) as usize;
    // SAFETY: kernel filled `returned` entries; capacity was reserved
    // above; layout matches our repr(C) declaration.
    unsafe { buf.set_len(returned) };
    Ok(buf)
}

/// Wrap `proc_pidfdinfo(PROC_PIDFDVNODEPATHINFO)`. Returns `None` on
/// any kernel error (the fd may have closed between listing and probe,
/// which is normal and not a hard error).
fn fd_path(pid: i32, fd: i32) -> Option<PathBuf> {
    let mut info: VnodeFdInfoWithPath = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<VnodeFdInfoWithPath>() as i32;
    let r = unsafe {
        proc_pidfdinfo(
            pid,
            fd,
            PROC_PIDFDVNODEPATHINFO,
            &mut info as *mut _ as *mut c_void,
            size,
        )
    };
    if r <= 0 {
        return None;
    }
    let cstr = CStr::from_bytes_until_nul(&info.pvip.vip_path).ok()?;
    let s = cstr.to_str().ok()?;
    Some(PathBuf::from(s))
}

/// Translate a libproc syscall failure for a non-dead PID into the
/// public error variants. macOS returns ESRCH (no such process) for
/// dead pids; everything else is bucketed as OsError.
fn map_pid_error(pid: i32) -> ResolveError {
    let err = std::io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::ESRCH) {
        ResolveError::PidDead(pid as u32)
    } else {
        ResolveError::OsError(err.to_string())
    }
}

/// Read `$HOME` for the running user. We deliberately do not call into
/// `dirs::home_dir()` here because we want the same env var the user
/// shell would see; libproc paths are evaluated by the kernel, not
/// re-resolved against passwd records.
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Path matches the Claude session jsonl pattern:
///
/// 1. Lives under `<HOME>/.claude/projects/`.
/// 2. Has a `.jsonl` extension.
/// 3. The basename (sans extension) is a syntactically valid uuid.
/// 4. The grandparent dir (the encoded-cwd segment) starts with `-`
///    and uses only the regex charset.
///
/// The strict uuid + encoded-cwd checks happen in `build_claim`; this
/// function is the fast filter that decides which path to hand off.
fn matches_session_jsonl(path: &Path, projects_root: &Path) -> bool {
    if !path.starts_with(projects_root) {
        return false;
    }
    if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
        return false;
    }
    // .../projects/<encoded-cwd>/<uuid>.jsonl
    // Need exactly one intermediate dir between projects_root and the
    // jsonl file.
    let Ok(rel) = path.strip_prefix(projects_root) else {
        return false;
    };
    let components: Vec<_> = rel.components().collect();
    components.len() == 2
}
