use super::files::{collect_files, read_entries, FileStat};
use super::Pacer;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const INCREMENTAL_DIR: &str = "incremental";

pub(super) struct CrateCache {
    pub(super) path: PathBuf,
    pub(super) sessions: Vec<Session>,
    pub(super) other_files: Vec<FileStat>,
}

pub(super) struct Session {
    pub(super) path: PathBuf,
    pub(super) lock: PathBuf,
    pub(super) name: SessionName,
    pub(super) files: Vec<FileStat>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SessionName {
    pub(super) started: SystemTime,
    pub(super) lock_file: String,
    pub(super) finalized: bool,
}

impl SessionName {
    pub(super) fn parse(name: &str) -> Option<Self> {
        let mut parts = name.strip_prefix("s-")?.splitn(3, '-');
        let timestamp = parts.next()?;
        let random = parts.next()?;
        let suffix = parts.next()?;
        let micros = u64::from_str_radix(timestamp, 36).ok()?;
        Some(Self {
            started: SystemTime::UNIX_EPOCH + Duration::from_micros(micros),
            lock_file: format!("s-{timestamp}-{random}.lock"),
            finalized: suffix != "working",
        })
    }
}

impl CrateCache {
    pub(super) fn files(&self) -> impl Iterator<Item = &FileStat> {
        self.sessions
            .iter()
            .flat_map(|session| &session.files)
            .chain(&self.other_files)
    }

    pub(super) fn superseded(&self) -> impl Iterator<Item = &Session> {
        let newest = newest_finalized(self.sessions.iter().map(|session| &session.name));
        self.sessions.iter().filter(move |session| {
            session.name.finalized && newest.is_some_and(|newest| session.name.started < newest)
        })
    }

    pub(super) fn idle_since(&self, cutoff: SystemTime, live_files: &HashSet<(u64, u64)>) -> bool {
        if !idle_since(self.sessions.iter().map(|session| &session.name), cutoff) {
            return false;
        }
        let newest = newest_finalized(self.sessions.iter().map(|session| &session.name));
        !self
            .sessions
            .iter()
            .filter(|session| Some(session.name.started) == newest)
            .flat_map(|session| &session.files)
            .any(|file| {
                file.identity
                    .is_some_and(|identity| live_files.contains(&identity))
            })
    }
}

pub(super) fn scan(profile: &Path, pacer: &mut Pacer) -> Vec<CrateCache> {
    read_entries(&profile.join(INCREMENTAL_DIR))
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| scan_crate(&entry.path(), pacer))
        .collect()
}

fn scan_crate(path: &Path, pacer: &mut Pacer) -> CrateCache {
    let mut cache = CrateCache {
        path: path.to_path_buf(),
        sessions: Vec::new(),
        other_files: Vec::new(),
    };
    for entry in read_entries(path) {
        let entry_path = entry.path();
        let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
        match SessionName::parse(&entry.file_name().to_string_lossy()).filter(|_| is_dir) {
            Some(name) => {
                let mut files = Vec::new();
                collect_files(&entry_path, &mut files, pacer);
                cache.sessions.push(Session {
                    path: entry_path,
                    lock: path.join(&name.lock_file),
                    name,
                    files,
                });
            }
            None => collect_files(&entry_path, &mut cache.other_files, pacer),
        }
    }
    cache
}

pub(super) fn session_names(crate_dir: &Path) -> Vec<SessionName> {
    read_entries(crate_dir)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| SessionName::parse(&entry.file_name().to_string_lossy()))
        .collect()
}

pub(super) fn newest_finalized<'a>(
    names: impl IntoIterator<Item = &'a SessionName>,
) -> Option<SystemTime> {
    names
        .into_iter()
        .filter(|name| name.finalized)
        .map(|name| name.started)
        .max()
}

pub(super) fn idle_since<'a>(
    names: impl IntoIterator<Item = &'a SessionName>,
    cutoff: SystemTime,
) -> bool {
    names
        .into_iter()
        .all(|name| name.finalized && name.started < cutoff)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_names_decode_rustc_base36_microsecond_timestamps() {
        let finalized = SessionName::parse("s-hn2bw05svb-1prkhum-dubvaty506tpjp7km0a5134ca")
            .expect("finalized session");
        assert!(finalized.finalized);
        assert_eq!(finalized.lock_file, "s-hn2bw05svb-1prkhum.lock");
        assert_eq!(
            finalized
                .started
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("after epoch")
                .as_micros(),
            u128::from(u64::from_str_radix("hn2bw05svb", 36).expect("base36")),
        );

        let working = SessionName::parse("s-hn2or1gzkw-1ebppzh-working").expect("working");
        assert!(!working.finalized);

        for name in ["s-hn2bw05svb-1prkhum.lock", "query-cache.bin", "s-??-x-y"] {
            assert_eq!(SessionName::parse(name), None, "{name}");
        }
    }

    #[test]
    fn idle_needs_every_session_finalized_and_older_than_the_cutoff() {
        let cutoff = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let session = |secs: u64, finalized: bool| SessionName {
            started: SystemTime::UNIX_EPOCH + Duration::from_secs(secs),
            lock_file: String::new(),
            finalized,
        };
        assert!(idle_since(&[session(10, true), session(50, true)], cutoff));
        assert!(!idle_since(
            &[session(10, true), session(150, true)],
            cutoff
        ));
        assert!(!idle_since(
            &[session(10, true), session(20, false)],
            cutoff
        ));
        assert!(
            idle_since(&[], cutoff),
            "a crate dir with no sessions holds nothing"
        );
    }
}
