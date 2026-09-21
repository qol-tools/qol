use std::fs;
use std::path::PathBuf;

use qol_terminal_sessions::SessionBinding;

pub(crate) struct LastSendStore {
    dir: PathBuf,
}

impl LastSendStore {
    pub(crate) fn system() -> Option<Self> {
        qol_config::data_subdir("sessions").map(|dir| Self {
            dir: dir.join("last-send"),
        })
    }

    #[cfg(test)]
    pub(crate) fn with_dir(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub(crate) fn record(&self, binding: &SessionBinding, text: &str) {
        if fs::create_dir_all(&self.dir).is_err() {
            return;
        }
        let file = self.file_for(binding);
        let temporary = file.with_extension("tmp");
        let Ok(encoded) = serde_json::to_string(text) else {
            return;
        };
        if fs::write(&temporary, encoded).is_ok() {
            let _ = fs::rename(&temporary, &file);
        }
    }

    pub(crate) fn last_sent(&self, binding: &SessionBinding) -> Option<String> {
        let file = self.file_for(binding);
        let encoded = fs::read_to_string(file).ok()?;
        serde_json::from_str(&encoded).ok()
    }

    fn file_for(&self, binding: &SessionBinding) -> PathBuf {
        self.dir.join(binding.token().replace(':', "_"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn binding() -> SessionBinding {
        SessionBinding::from_str("v1:kitty:7:1").expect("fake binding")
    }

    #[test]
    fn record_and_read_roundtrip_the_last_sent_text() {
        let root = tempfile::TempDir::new().unwrap();
        let store = LastSendStore::with_dir(root.path().to_path_buf());
        assert_eq!(store.last_sent(&binding()), None);

        store.record(&binding(), "sleep 4; echo relay-slow-ok");
        assert_eq!(
            store.last_sent(&binding()).as_deref(),
            Some("sleep 4; echo relay-slow-ok")
        );

        store.record(&binding(), "echo next");
        assert_eq!(store.last_sent(&binding()).as_deref(), Some("echo next"));
    }

    #[test]
    fn sessions_do_not_share_last_sent_state() {
        let root = tempfile::TempDir::new().unwrap();
        let store = LastSendStore::with_dir(root.path().to_path_buf());
        store.record(&binding(), "only this session");

        let other = SessionBinding::from_str("v1:kitty:8:2").expect("fake binding");
        assert_eq!(store.last_sent(&other), None);
    }

    #[test]
    fn multiline_text_survives_the_roundtrip() {
        let root = tempfile::TempDir::new().unwrap();
        let store = LastSendStore::with_dir(root.path().to_path_buf());
        store.record(&binding(), "line one\nline two\n");
        assert_eq!(
            store.last_sent(&binding()).as_deref(),
            Some("line one\nline two\n")
        );
    }
}
