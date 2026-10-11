use super::files::{collect_files, read_entries, FileStat};
use super::Pacer;
use crate::platform::{BuildPlatform, Platform};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub(super) const FINGERPRINT_DIR: &str = ".fingerprint";
const ARTIFACT_DIRS: [&str; 3] = ["deps", "build", "examples"];

#[derive(Default)]
pub(super) struct Unit {
    pub(super) fingerprint: Option<PathBuf>,
    pub(super) paths: Vec<PathBuf>,
    pub(super) files: Vec<FileStat>,
    pub(super) last_use: Option<SystemTime>,
    pub(super) dependencies: Vec<String>,
}

#[derive(Default)]
pub(super) struct UnitScan {
    pub(super) units: HashMap<String, Unit>,
    pub(super) by_fingerprint: HashMap<String, String>,
}

pub(super) fn scan(profile: &Path, access_times: bool, pacer: &mut Pacer) -> UnitScan {
    let mut scan = UnitScan::default();
    for entry in read_entries(&profile.join(FINGERPRINT_DIR)) {
        let Some(hash) = unit_hash(&entry.file_name().to_string_lossy()).map(str::to_string) else {
            continue;
        };
        let path = entry.path();
        let unit = scan.units.entry(hash.clone()).or_default();
        unit.last_use = Some(last_use(&path, access_times));
        for file in read_entries(&path) {
            pacer.tick();
            let file_path = file.path();
            let Ok(metadata) = fs::symlink_metadata(&file_path) else {
                continue;
            };
            if metadata.is_dir() {
                collect_files(&file_path, &mut unit.files, pacer);
                continue;
            }
            unit.files.push(FileStat::of(&metadata));
            let name = file.file_name().to_string_lossy().into_owned();
            if name.ends_with(".json") {
                if let Ok(bytes) = fs::read(&file_path) {
                    unit.dependencies.extend(dependency_fingerprints(&bytes));
                }
            } else if is_hash_file(&name) {
                if let Some(value) = Platform.read_preserving_access_time(&file_path) {
                    scan.by_fingerprint
                        .insert(value.trim().to_string(), hash.clone());
                }
            }
        }
        unit.paths.push(path.clone());
        unit.fingerprint = Some(path);
    }
    for dir in ARTIFACT_DIRS {
        for entry in read_entries(&profile.join(dir)) {
            let Some(hash) = unit_hash(&entry.file_name().to_string_lossy()).map(str::to_string)
            else {
                continue;
            };
            let unit = scan.units.entry(hash).or_default();
            collect_files(&entry.path(), &mut unit.files, pacer);
            unit.paths.push(entry.path());
        }
    }
    scan
}

pub(super) fn last_use(fingerprint: &Path, access_times: bool) -> SystemTime {
    let mut newest = SystemTime::UNIX_EPOCH;
    for entry in read_entries(fingerprint) {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        newest = newest.max(metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH));
        if access_times && is_hash_file(&entry.file_name().to_string_lossy()) {
            newest = newest.max(metadata.accessed().unwrap_or(SystemTime::UNIX_EPOCH));
        }
    }
    newest
}

pub(super) fn live_units(
    scan: &UnitScan,
    roots: &HashSet<&str>,
    cutoff: Option<SystemTime>,
) -> HashSet<String> {
    let mut live = HashSet::new();
    let mut pending: Vec<&str> = scan
        .units
        .iter()
        .filter(|(hash, unit)| {
            unit.fingerprint.is_some()
                && (roots.contains(hash.as_str())
                    || cutoff.is_none_or(|cutoff| unit.last_use.is_some_and(|used| used >= cutoff)))
        })
        .map(|(hash, _)| hash.as_str())
        .collect();
    while let Some(hash) = pending.pop() {
        if !live.insert(hash.to_string()) {
            continue;
        }
        let Some(unit) = scan.units.get(hash) else {
            continue;
        };
        for dependency in &unit.dependencies {
            if let Some(target) = scan.by_fingerprint.get(dependency) {
                if !live.contains(target) {
                    pending.push(target);
                }
            }
        }
    }
    live
}

pub(super) fn fingerprinted_hashes(profile: &Path) -> HashSet<String> {
    read_entries(&profile.join(FINGERPRINT_DIR))
        .filter_map(|entry| unit_hash(&entry.file_name().to_string_lossy()).map(str::to_string))
        .collect()
}

fn unit_hash(name: &str) -> Option<&str> {
    let stem = name.split('.').next()?;
    let (_, hash) = stem.rsplit_once('-')?;
    (hash.len() == 16
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
    .then_some(hash)
}

fn is_hash_file(name: &str) -> bool {
    !name.ends_with(".json")
        && !name.starts_with("dep-")
        && !name.starts_with("output-")
        && name != "invoked.timestamp"
}

fn dependency_fingerprints(json: &[u8]) -> Vec<String> {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(json) else {
        return Vec::new();
    };
    value
        .get("deps")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|dependency| dependency.get(3)?.as_u64())
        .map(little_endian_hex)
        .collect()
}

fn little_endian_hex(value: u64) -> String {
    value
        .to_le_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_hash_reads_the_suffix_of_every_cargo_artifact_name() {
        let cases = [
            ("libserde-0123456789abcdef.rlib", Some("0123456789abcdef")),
            ("serde-0123456789abcdef.d", Some("0123456789abcdef")),
            ("qol_tray-0123456789abcdef", Some("0123456789abcdef")),
            ("proc-macro2-0123456789abcdef", Some("0123456789abcdef")),
            (
                "qol_tray-0123456789abcdef.qol_tray.1a2b-cgu.0.rcgu.o",
                Some("0123456789abcdef"),
            ),
            ("rmetaAbC123", None),
            ("serde-0123456789ABCDEF.d", None),
            ("serde-0123456789abcde.d", None),
        ];
        for (name, expected) in cases {
            assert_eq!(unit_hash(name), expected, "{name}");
        }
    }

    #[test]
    fn dependency_fingerprints_match_the_hash_file_encoding() {
        let json =
            br#"{"rustc":1,"deps":[[11,"serde",false,1311768467294899695],[12,"log",false,0]]}"#;
        assert_eq!(
            dependency_fingerprints(json),
            ["efcdab9078563412", "0000000000000000"],
            "1311768467294899695 is 0x1234567890abcdef, written little-endian"
        );
        assert!(dependency_fingerprints(b"not json").is_empty());
    }

    #[test]
    fn only_the_fingerprint_hash_counts_as_a_read_signal() {
        for name in [
            "lib-serde",
            "build-script-build-script-build",
            "test-lib-qol_tray",
        ] {
            assert!(is_hash_file(name), "{name}");
        }
        for name in [
            "lib-serde.json",
            "dep-lib-serde",
            "output-lib-serde",
            "invoked.timestamp",
        ] {
            assert!(!is_hash_file(name), "{name}");
        }
    }
}
