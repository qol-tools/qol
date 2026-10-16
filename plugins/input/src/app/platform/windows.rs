pub(crate) fn on_stop_request(stop: impl FnOnce() + Send + 'static) {
    let listener = std::thread::Builder::new()
        .name("input-stop".into())
        .spawn(move || match qol_process::wait_for_stop_request() {
            Ok(()) => stop(),
            Err(error) => log::warn!("stop request listener unavailable: {error}"),
        });
    if let Err(error) = listener {
        log::warn!("stop request listener thread failed: {error}");
    }
}
