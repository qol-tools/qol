use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub(crate) struct LogWriter {
    dir: PathBuf,
    file: Mutex<Option<(String, File)>>,
}

impl LogWriter {
    pub(crate) fn new(dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&dir);
        Self {
            dir,
            file: Mutex::new(None),
        }
    }

    pub(crate) fn write(&self, entry: &str) {
        let today = log_file_name();
        let mut guard = match self.file.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let file = match guard.as_mut() {
            Some((name, f)) if *name == today => f,
            _ => {
                let path = self.dir.join(&today);
                let f = OpenOptions::new().create(true).append(true).open(&path);
                let Ok(f) = f else { return };
                *guard = Some((today, f));
                &mut guard.as_mut().unwrap().1
            }
        };
        let _ = file.write_all(entry.as_bytes());
    }
}

fn log_file_name() -> String {
    let now = chrono::Local::now();
    format!("qol-tray-{}.log", now.format("%Y-%m-%d"))
}

pub(crate) fn rotate_old_logs(dir: &Path, keep: usize) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut logs: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name();
            let s = name.to_string_lossy();
            s.starts_with("qol-tray-") && s.ends_with(".log")
        })
        .collect();

    if logs.len() <= keep {
        return;
    }

    logs.sort_by_key(|e| e.file_name());
    for entry in &logs[..logs.len() - keep] {
        let _ = fs::remove_file(entry.path());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn log_file_name_contains_date() {
        let name = log_file_name();
        assert!(name.starts_with("qol-tray-"), "name: {}", name);
        assert!(name.ends_with(".log"), "name: {}", name);
        assert!(name.len() > 20, "name should contain date: {}", name);
    }

    #[test]
    fn write_entry_creates_file_and_appends() {
        let tmp = TempDir::new().unwrap();
        let writer = LogWriter::new(tmp.path().to_path_buf());

        writer.write("first line\n");
        writer.write("second line\n");

        let files: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(files.len(), 1, "should have one log file");

        let content = std::fs::read_to_string(files[0].path()).unwrap();
        assert!(content.contains("first line"), "content: {}", content);
        assert!(content.contains("second line"), "content: {}", content);
    }

    #[test]
    fn rotate_removes_old_files() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();

        for day in 1..=10 {
            let name = format!("qol-tray-2026-03-{:02}.log", day);
            std::fs::write(dir.join(&name), "old").unwrap();
        }

        rotate_old_logs(dir, 7);

        let remaining: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".log"))
            .collect();
        assert_eq!(remaining.len(), 7, "should keep 7 most recent");
    }

    #[test]
    fn rotate_ignores_non_log_files() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("other.txt"), "keep").unwrap();
        std::fs::write(tmp.path().join("qol-tray-2020-01-01.log"), "old").unwrap();

        rotate_old_logs(tmp.path(), 7);

        assert!(tmp.path().join("other.txt").exists());
    }
}
