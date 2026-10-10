use std::fs;
use std::io::{self, ErrorKind};
use std::path::PathBuf;

pub const PINNED_REASON: &str = "it is pinned";

pub fn pins_dir() -> Option<PathBuf> {
    qol_config::data_subdir("sessions").map(|path| path.join("pins"))
}

#[derive(Clone, Debug, Default)]
pub struct PinStore {
    dir: Option<PathBuf>,
}

impl PinStore {
    pub fn system() -> Self {
        Self { dir: pins_dir() }
    }

    pub fn with_dir(dir: PathBuf) -> Self {
        Self { dir: Some(dir) }
    }

    pub fn is_pinned(&self, external_id: &str) -> bool {
        self.path(external_id).is_some_and(|path| path.is_file())
    }

    pub fn set(&self, external_id: &str, pinned: bool) -> io::Result<()> {
        let path = self.path(external_id).ok_or_else(|| {
            io::Error::new(
                ErrorKind::InvalidInput,
                format!("`{external_id}` cannot be pinned"),
            )
        })?;
        if pinned {
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir)?;
            }
            return fs::write(path, b"");
        }
        match fs::remove_file(path) {
            Err(error) if error.kind() != ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }

    pub fn toggle(&self, external_id: &str) -> io::Result<bool> {
        let pinned = !self.is_pinned(external_id);
        self.set(external_id, pinned)?;
        Ok(pinned)
    }

    fn path(&self, external_id: &str) -> Option<PathBuf> {
        let safe = !external_id.is_empty()
            && external_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        self.dir
            .as_ref()
            .filter(|_| safe)
            .map(|dir| dir.join(external_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_flips_a_pin_and_set_is_idempotent() {
        let root = tempfile::TempDir::new().unwrap();
        let pins = PinStore::with_dir(root.path().join("pins"));
        let id = "80612153-0dc9-4218-803e-0221f8bb479b";
        assert!(!pins.is_pinned(id));
        assert!(pins.toggle(id).unwrap());
        assert!(pins.is_pinned(id));
        pins.set(id, true).unwrap();
        assert!(pins.is_pinned(id));
        assert!(!pins.toggle(id).unwrap());
        assert!(!pins.is_pinned(id));
        pins.set(id, false).unwrap();
        assert!(!pins.is_pinned(id));
    }

    #[test]
    fn an_unsafe_id_or_a_store_without_a_dir_is_never_pinned() {
        let root = tempfile::TempDir::new().unwrap();
        let pins = PinStore::with_dir(root.path().to_path_buf());
        for id in ["", "../escape", "a/b", "."] {
            assert!(pins.set(id, true).is_err(), "{id}");
            assert!(!pins.is_pinned(id), "{id}");
        }
        let detached = PinStore::default();
        assert!(detached.set("abc", true).is_err());
        assert!(!detached.is_pinned("abc"));
    }
}
