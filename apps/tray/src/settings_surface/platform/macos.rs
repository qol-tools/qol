#[path = "gpui_host.rs"]
mod gpui_host;
#[path = "native_tools/mod.rs"]
mod native_tools;

pub(in crate::settings_surface) use gpui_host::{
    apply_theme, plugins_changed, prewarm, request, run, show_toast, stop, wait_until_ready,
};

pub(in crate::settings_surface) fn native_available() -> bool {
    true
}

fn process_elapsed_ms() -> Option<u64> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>();
    let read = unsafe {
        libc::proc_pidinfo(
            libc::getpid(),
            libc::PROC_PIDTBSDINFO,
            0,
            std::ptr::from_mut(&mut info).cast(),
            i32::try_from(size).ok()?,
        )
    };
    if read != i32::try_from(size).ok()? {
        return None;
    }
    let started = std::time::UNIX_EPOCH.checked_add(std::time::Duration::new(
        info.pbi_start_tvsec,
        u32::try_from(info.pbi_start_tvusec * 1000).ok()?,
    ))?;
    std::time::SystemTime::now()
        .duration_since(started)
        .ok()
        .map(|elapsed| elapsed.as_millis() as u64)
}
