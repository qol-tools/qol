pub(crate) fn run(_args: Vec<String>) -> i32 {
    eprintln!(
        "keyremap: only macOS is supported (requires CGEventTap and Accessibility APIs); host is {}",
        std::env::consts::OS
    );
    1
}
