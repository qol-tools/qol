#[cfg(debug_assertions)]
const LOG_FILE: &str = "/tmp/qol-altmon.log";

#[cfg(debug_assertions)]
const MAX_LOG_BYTES: u64 = 16 * 1024 * 1024;

pub fn probe(tag: &str, msg: &str) {
    #[cfg(debug_assertions)]
    {
        use std::io::Write;

        rotate_if_needed();

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let line = format!("{ts} pid={} {tag} {msg}\n", std::process::id());
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(LOG_FILE)
        {
            let _ = file.write_all(line.as_bytes());
        }
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = (tag, msg);
    }
}

#[cfg(debug_assertions)]
fn rotate_if_needed() {
    let Ok(metadata) = std::fs::metadata(LOG_FILE) else {
        return;
    };
    if metadata.len() < MAX_LOG_BYTES {
        return;
    }

    let rotated = format!("{LOG_FILE}.1");
    let _ = std::fs::remove_file(&rotated);
    let _ = std::fs::rename(LOG_FILE, rotated);
}

#[macro_export]
macro_rules! probe {
    ($tag:expr, $($arg:tt)+) => {{
        #[cfg(debug_assertions)]
        {
            $crate::probe::probe($tag, &::std::format!($($arg)+));
        }
    }};
}
