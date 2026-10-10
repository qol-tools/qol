pub(in crate::surface::memory) fn flush_on_termination(flush: fn()) {
    let spawned = std::thread::Builder::new()
        .name("qol-window-state-flush".into())
        .spawn(move || match qol_process::wait_for_stop_request() {
            Ok(()) => {
                flush();
                std::process::exit(0);
            }
            Err(error) => log::warn!("[window-state] termination flush unavailable: {error}"),
        });
    if let Err(error) = spawned {
        log::warn!("[window-state] termination flush thread failed: {error}");
    }
}
