pub(in crate::surface::memory) fn flush_on_termination(flush: fn()) {
    if let Err(error) = qol_process::on_stop_request(flush) {
        log::warn!("[window-state] termination flush unavailable: {error}");
    }
}
