pub fn open_settings() {
    let _ = std::process::Command::new("xdg-open")
        .arg(super::settings_url())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}
