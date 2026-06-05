pub fn probe(tag: &str, msg: &str) {
    #[cfg(debug_assertions)]
    {
        use std::io::Write;
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let line = format!("{ts} pid={} {tag} {msg}\n", std::process::id());
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/qol-altmon.log")
        {
            let _ = file.write_all(line.as_bytes());
        }
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = (tag, msg);
    }
}
