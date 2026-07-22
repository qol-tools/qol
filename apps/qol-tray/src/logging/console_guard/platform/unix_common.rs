pub(super) fn spawn_console_guard(watch: impl FnOnce() + Send + 'static) {
    let result = std::thread::Builder::new()
        .name("qol-console-guard".into())
        .spawn(watch);
    if let Err(error) = result {
        log::warn!("console pipe guard failed to start: {error}");
    }
}

pub(super) fn redirect_to_null(fd: libc::c_int) {
    let devnull =
        unsafe { libc::open(c"/dev/null".as_ptr() as *const libc::c_char, libc::O_WRONLY) };
    if devnull < 0 {
        return;
    }
    unsafe {
        libc::dup2(devnull, fd);
        libc::close(devnull);
    }
    log::info!("console fd {fd} lost its reader; redirected to /dev/null");
}
