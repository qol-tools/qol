use anyhow::{anyhow, Context, Result};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::path::Path;

const INSTALL_ID_BYTES: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkerFile {
    pub install_id: String,
    pub profile_id: String,
    pub schema_version: u32,
}

pub fn generate_install_id() -> String {
    let bytes: [u8; INSTALL_ID_BYTES] = rand::rng().random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn read_marker(path: &Path) -> Result<Option<MarkerFile>> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(e).with_context(|| format!("reading marker at {}", path.display()));
        }
    };
    let marker = serde_json::from_slice::<MarkerFile>(&bytes)
        .with_context(|| format!("parsing marker at {}", path.display()))?;
    Ok(Some(marker))
}

pub fn write_marker(path: &Path, marker: &MarkerFile) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating parent dir {}", parent.display()))?;
        }
    }
    let bytes = serde_json::to_vec_pretty(marker).context("serialising marker")?;
    let tmp = tmp_path(path);
    std::fs::write(&tmp, &bytes)
        .with_context(|| format!("writing marker tmp at {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| {
        format!(
            "renaming marker tmp {} into place at {}",
            tmp.display(),
            path.display()
        )
    })?;
    Ok(())
}

pub fn ensure_marker_or_create(
    path: &Path,
    expected_install_id: Option<&str>,
    profile_id: &str,
    schema_version: u32,
) -> Result<MarkerFile> {
    if let Some(existing) = read_marker(path)? {
        if let Some(expected) = expected_install_id {
            check_marker_compatible(&existing, expected, profile_id)?;
        } else {
            check_profile_compatible(&existing, profile_id)?;
        }
        return Ok(existing);
    }
    let install_id = match expected_install_id {
        Some(id) => id.to_string(),
        None => generate_install_id(),
    };
    let marker = MarkerFile {
        install_id,
        profile_id: profile_id.to_string(),
        schema_version,
    };
    write_marker(path, &marker)?;
    Ok(marker)
}

pub fn check_marker_compatible(
    found: &MarkerFile,
    expected_install_id: &str,
    expected_profile_id: &str,
) -> Result<()> {
    if found.install_id != expected_install_id {
        return Err(anyhow!(
            "marker install_id {} does not match expected {}; this remote is owned by a different qol-tray install",
            found.install_id,
            expected_install_id
        ));
    }
    check_profile_compatible(found, expected_profile_id)
}

fn check_profile_compatible(found: &MarkerFile, expected_profile_id: &str) -> Result<()> {
    if found.profile_id != expected_profile_id {
        return Err(anyhow!(
            "marker profile_id {} does not match expected {}; this remote is owned by a different qol-tray profile",
            found.profile_id,
            expected_profile_id
        ));
    }
    Ok(())
}

fn tmp_path(path: &Path) -> std::path::PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".tmp");
    s.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marker(install_id: &str, profile_id: &str, schema_version: u32) -> MarkerFile {
        MarkerFile {
            install_id: install_id.to_string(),
            profile_id: profile_id.to_string(),
            schema_version,
        }
    }

    #[test]
    fn generate_install_id_returns_32_char_lowercase_hex() {
        let id = generate_install_id();
        assert_eq!(id.len(), 32, "id: {id}");
        for c in id.chars() {
            assert!(
                matches!(c, '0'..='9' | 'a'..='f'),
                "non-hex-lowercase char {c:?} in {id}"
            );
        }
    }

    #[test]
    fn generate_install_id_is_unique_across_calls() {
        let a = generate_install_id();
        let b = generate_install_id();
        assert_ne!(a, b, "two generate_install_id calls collided: {a}");
    }

    #[test]
    fn write_then_read_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("marker.json");
        let original = marker("abc123", "default", 1);
        write_marker(&path, &original).unwrap();
        let loaded = read_marker(&path).unwrap();
        assert_eq!(loaded, Some(original));
    }

    #[test]
    fn read_marker_on_missing_file_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.json");
        assert_eq!(read_marker(&path).unwrap(), None);
    }

    #[test]
    fn read_marker_on_garbage_file_returns_err() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("marker.json");
        std::fs::write(&path, b"this is not json").unwrap();
        assert!(read_marker(&path).is_err());
    }

    #[test]
    fn write_marker_creates_missing_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a").join("b").join("c").join("marker.json");
        write_marker(&path, &marker("abc123", "default", 1)).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn ensure_marker_or_create_with_none_creates_then_reuses() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("marker.json");
        let first = ensure_marker_or_create(&path, None, "default", 1).unwrap();
        assert_eq!(first.install_id.len(), 32);
        assert_eq!(first.profile_id, "default");
        assert_eq!(first.schema_version, 1);
        let second = ensure_marker_or_create(&path, None, "default", 1).unwrap();
        assert_eq!(first, second, "second call should return the same marker");
    }

    #[test]
    fn ensure_marker_or_create_with_some_uses_provided_id_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("marker.json");
        let created = ensure_marker_or_create(&path, Some("abc"), "default", 1).unwrap();
        assert_eq!(created.install_id, "abc");
        assert_eq!(created.profile_id, "default");
    }

    #[test]
    fn ensure_marker_or_create_errors_on_install_id_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("marker.json");
        write_marker(&path, &marker("abc", "default", 1)).unwrap();
        let err = ensure_marker_or_create(&path, Some("xyz"), "default", 1).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("install_id"), "msg: {msg}");
        assert!(msg.contains("abc"), "msg: {msg}");
        assert!(msg.contains("xyz"), "msg: {msg}");
        assert!(
            msg.contains("different qol-tray install"),
            "msg should explain ownership: {msg}"
        );
    }

    #[test]
    fn check_marker_compatible_passes_when_matching() {
        let m = marker("abc", "default", 1);
        check_marker_compatible(&m, "abc", "default").unwrap();
    }

    #[test]
    fn check_marker_compatible_errors_on_install_id_mismatch() {
        let m = marker("abc", "default", 1);
        let err = check_marker_compatible(&m, "xyz", "default").unwrap_err();
        let msg = format!("{err}");
        let needles = ["install_id", "abc", "xyz", "different qol-tray install"];
        for needle in needles {
            assert!(msg.contains(needle), "missing {needle:?} in {msg}");
        }
    }

    #[test]
    fn check_marker_compatible_errors_on_profile_id_mismatch() {
        let m = marker("abc", "default", 1);
        let err = check_marker_compatible(&m, "abc", "work").unwrap_err();
        let msg = format!("{err}");
        let needles = [
            "profile_id",
            "default",
            "work",
            "different qol-tray profile",
        ];
        for needle in needles {
            assert!(msg.contains(needle), "missing {needle:?} in {msg}");
        }
    }
}
