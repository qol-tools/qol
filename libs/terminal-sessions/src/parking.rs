use std::fs;
use std::io::{self, ErrorKind};
use std::path::PathBuf;

pub const PARKING_ENV: &str = "QOL_SESSIONS_PARK";

const ON: &str = "on";
const OFF: &str = "off";

pub fn parking_dir() -> Option<PathBuf> {
    qol_config::data_subdir("sessions").map(|path| path.join("parking"))
}

pub fn launch_env(on: bool) -> (String, String) {
    (
        PARKING_ENV.to_owned(),
        if on { "1" } else { "0" }.to_owned(),
    )
}

pub fn launched_with_parking() -> bool {
    std::env::var_os(PARKING_ENV).is_some_and(|value| value == "1")
}

#[derive(Clone, Debug, Default)]
pub struct ParkingStore {
    dir: Option<PathBuf>,
}

impl ParkingStore {
    pub fn system() -> Self {
        Self { dir: parking_dir() }
    }

    pub fn with_dir(dir: PathBuf) -> Self {
        Self { dir: Some(dir) }
    }

    pub fn explicit(&self, external_id: &str) -> Option<bool> {
        match fs::read_to_string(self.path(external_id)?).ok()?.trim() {
            ON => Some(true),
            OFF => Some(false),
            _ => None,
        }
    }

    pub fn is_on(&self, external_id: &str, launch_default: bool) -> bool {
        self.explicit(external_id).unwrap_or(launch_default)
    }

    pub fn set(&self, external_id: &str, on: bool) -> io::Result<()> {
        let path = self.path(external_id).ok_or_else(|| {
            io::Error::new(
                ErrorKind::InvalidInput,
                format!("`{external_id}` cannot hold a parking choice"),
            )
        })?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let staged = path.with_extension("tmp");
        fs::write(&staged, if on { ON } else { OFF })?;
        fs::rename(staged, path)
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
    fn an_explicit_choice_beats_the_launch_default_until_it_is_changed() {
        let root = tempfile::TempDir::new().unwrap();
        let parking = ParkingStore::with_dir(root.path().join("parking"));
        let id = "80612153-0dc9-4218-803e-0221f8bb479b";
        assert_eq!(parking.explicit(id), None);
        assert!(!parking.is_on(id, false));
        assert!(parking.is_on(id, true));
        parking.set(id, false).unwrap();
        assert!(!parking.is_on(id, true));
        parking.set(id, true).unwrap();
        assert!(parking.is_on(id, false));
        assert_eq!(parking.explicit(id), Some(true));
    }

    #[test]
    fn an_unsafe_id_holds_no_choice() {
        let root = tempfile::TempDir::new().unwrap();
        let parking = ParkingStore::with_dir(root.path().to_path_buf());
        for id in ["", "../escape", "a/b"] {
            assert!(parking.set(id, true).is_err(), "{id}");
            assert_eq!(parking.explicit(id), None, "{id}");
        }
    }
}
