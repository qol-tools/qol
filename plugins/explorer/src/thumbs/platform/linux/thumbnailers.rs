use std::ffi::OsString;
use std::fs::{self, DirBuilder, Permissions};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::Duration;

use qol_process::BoundedCommandOutput;

use super::super::super::{Flavor, ThumbError};
use super::cache::Stamp;
use super::sandbox;

const RUN_TIMEOUT: Duration = Duration::from_secs(15);
const OUTPUT_LIMIT: usize = 4096;
const ENTRY_HEADER: &str = "[Thumbnailer Entry]";
const WORK_PREFIX: &str = ".work-";
const WORK_OUTPUT: &str = "preview.png";
const WORK_MODE: u32 = 0o700;

pub(super) struct Thumbnailer {
    exec: Vec<String>,
    mimes: Vec<String>,
}

impl Thumbnailer {
    fn parse(text: &str) -> Option<Self> {
        let mut in_entry = false;
        let (mut exec, mut mimes, mut try_exec) = (None, None, None);
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') {
                in_entry = line == ENTRY_HEADER;
                continue;
            }
            let Some((key, value)) = line.split_once('=').filter(|_| in_entry) else {
                continue;
            };
            match key.trim() {
                "Exec" => exec = Some(value.trim().to_string()),
                "MimeType" => mimes = Some(value.trim().to_string()),
                "TryExec" => try_exec = Some(value.trim().to_string()),
                _ => {}
            }
        }
        if try_exec.is_some_and(|program| !sandbox::finds(&program)) {
            return None;
        }
        let exec: Vec<String> = exec?.split_whitespace().map(str::to_string).collect();
        if exec.first().is_none_or(|program| !sandbox::finds(program)) {
            return None;
        }
        let mimes = mimes?
            .split(';')
            .map(str::trim)
            .filter(|mime| !mime.is_empty())
            .map(str::to_string)
            .collect();
        Some(Self { exec, mimes })
    }

    fn words(&self, input: &Path, uri: &str, size: u32, output: &Path) -> Vec<OsString> {
        self.exec
            .iter()
            .map(|word| expand(word, input, uri, size, output))
            .collect()
    }

    pub(super) fn run(
        &self,
        input: &Path,
        stamp: &Stamp,
        flavor: Flavor,
        target: &Path,
    ) -> Result<(), ThumbError> {
        let failed = |reason: String| ThumbError::Unavailable(reason);
        let folder = target
            .parent()
            .ok_or_else(|| failed("the cache folder is not known".into()))?;
        fs::create_dir_all(folder).map_err(|error| failed(error.to_string()))?;
        let work = work_folder(folder).map_err(|error| failed(error.to_string()))?;
        let partial = work.join(WORK_OUTPUT);
        let words = self.words(input, &stamp.uri, flavor.px(), &partial);
        let result = self.make(&words, input, &work, &partial, target);
        clear(&work);
        result
    }

    fn make(
        &self,
        words: &[OsString],
        input: &Path,
        work: &Path,
        partial: &Path,
        target: &Path,
    ) -> Result<(), ThumbError> {
        let failed = |reason: String| ThumbError::Unavailable(reason);
        let command = sandbox::command(words, input, work).ok_or_else(|| {
            failed("previews need bubblewrap (bwrap) to run thumbnailers sandboxed".into())
        })?;
        let outcome =
            qol_process::run_owned_with_output_timeout(command, RUN_TIMEOUT, OUTPUT_LIMIT);
        let made = partial
            .symlink_metadata()
            .is_ok_and(|meta| meta.file_type().is_file() && meta.len() > 0);
        match outcome {
            Ok(BoundedCommandOutput::Completed(output)) if output.status.success() && made => {
                fs::rename(partial, target).map_err(|error| failed(error.to_string()))
            }
            Ok(BoundedCommandOutput::Completed(output)) => Err(failed(format!(
                "the thumbnailer {} exited with {} and made no preview",
                self.exec[0], output.status
            ))),
            Ok(BoundedCommandOutput::TimedOut { .. }) => Err(failed(format!(
                "the thumbnailer {} took longer than {} s",
                self.exec[0],
                RUN_TIMEOUT.as_secs()
            ))),
            Err(error) => Err(failed(format!("could not run {}: {error}", self.exec[0]))),
        }
    }
}

fn work_folder(folder: &Path) -> std::io::Result<PathBuf> {
    static RUNS: AtomicU64 = AtomicU64::new(0);
    static SWEPT: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
    {
        let mut swept = SWEPT.lock().unwrap_or_else(PoisonError::into_inner);
        if !swept.iter().any(|known| known == folder) {
            sweep(folder);
            swept.push(folder.to_path_buf());
        }
    }
    let run = RUNS.fetch_add(1, Ordering::Relaxed);
    let work = folder.join(format!("{WORK_PREFIX}{}-{run}", std::process::id()));
    if work.symlink_metadata().is_ok() {
        clear(&work);
    }
    DirBuilder::new().mode(WORK_MODE).create(&work)?;
    Ok(work)
}

fn sweep(folder: &Path) {
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(owner) = name
            .to_str()
            .and_then(|name| name.strip_prefix(WORK_PREFIX))
            .and_then(|rest| rest.split_once('-'))
            .and_then(|(pid, _)| pid.parse::<u32>().ok())
        else {
            continue;
        };
        if owner != std::process::id() && !Path::new("/proc").join(owner.to_string()).exists() {
            clear(&entry.path());
        }
    }
}

fn clear(work: &Path) {
    if fs::remove_dir_all(work).is_ok() {
        return;
    }
    unlock(work);
    if let Err(error) = fs::remove_dir_all(work) {
        log::debug!("could not remove {}: {error}", work.display());
    }
}

fn unlock(dir: &Path) {
    let _ = fs::set_permissions(dir, Permissions::from_mode(WORK_MODE));
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            unlock(&entry.path());
        }
    }
}

pub(super) fn for_mime(mime: &str) -> Option<&'static Thumbnailer> {
    static ALL: OnceLock<Vec<Thumbnailer>> = OnceLock::new();
    ALL.get_or_init(discover)
        .iter()
        .find(|thumbnailer| thumbnailer.mimes.iter().any(|known| known == mime))
}

fn discover() -> Vec<Thumbnailer> {
    let mut seen: Vec<OsString> = Vec::new();
    let mut found = Vec::new();
    for dir in super::data_dirs() {
        let Ok(entries) = fs::read_dir(dir.join("thumbnailers")) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "thumbnailer"))
            .collect();
        files.sort();
        for file in files {
            let Some(name) = file.file_name().map(OsString::from) else {
                continue;
            };
            if seen.contains(&name) {
                continue;
            }
            seen.push(name);
            if let Some(thumbnailer) = fs::read_to_string(&file)
                .ok()
                .as_deref()
                .and_then(Thumbnailer::parse)
            {
                found.push(thumbnailer);
            }
        }
    }
    found
}

fn expand(word: &str, input: &Path, uri: &str, size: u32, output: &Path) -> OsString {
    let mut expanded = OsString::new();
    let mut chars = word.chars();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            expanded.push(ch.to_string());
            continue;
        }
        match chars.next() {
            Some('i') => expanded.push(input),
            Some('u') => expanded.push(uri),
            Some('o') => expanded.push(output),
            Some('s') => expanded.push(size.to_string()),
            Some('%') => expanded.push("%"),
            _ => {}
        }
    }
    expanded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_codes_expand_to_the_file_the_size_and_the_output() {
        let thumbnailer = Thumbnailer {
            exec: [
                "sh", "-i", "%i", "-u", "%u", "-o", "%o", "-s", "%s", "100%%", "%k",
            ]
            .map(str::to_string)
            .to_vec(),
            mimes: vec![],
        };
        let words = thumbnailer.words(
            Path::new("/v/a b.webm"),
            "file:///v/a%20b.webm",
            256,
            Path::new("/c/o.png"),
        );
        assert_eq!(
            words,
            [
                "sh",
                "-i",
                "/v/a b.webm",
                "-u",
                "file:///v/a%20b.webm",
                "-o",
                "/c/o.png",
                "-s",
                "256",
                "100%",
                ""
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn every_run_gets_its_own_private_empty_folder() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let first = work_folder(temp.path()).unwrap();
        let second = work_folder(temp.path()).unwrap();
        assert_ne!(first, second);
        for work in [&first, &second] {
            let meta = work.symlink_metadata().unwrap();
            assert!(meta.is_dir(), "{work:?}");
            assert_eq!(meta.permissions().mode() & 0o777, WORK_MODE, "{work:?}");
            assert_eq!(fs::read_dir(work).unwrap().count(), 0, "{work:?}");
        }
    }

    #[test]
    fn locked_and_orphaned_work_folders_are_removed() {
        let temp = tempfile::tempdir().unwrap();
        let orphan = temp.path().join(format!("{WORK_PREFIX}{}-0", u32::MAX));
        let locked = orphan.join("locked");
        fs::create_dir_all(locked.join("inner")).unwrap();
        fs::set_permissions(&locked, Permissions::from_mode(0)).unwrap();
        let kept = temp.path().join("kept.png");
        fs::write(&kept, b"png").unwrap();
        sweep(temp.path());
        assert!(orphan.symlink_metadata().is_err());
        assert!(kept.is_file());
    }

    #[test]
    fn only_entries_whose_program_is_installed_count() {
        let cases = [
            (
                "[Thumbnailer Entry]\nExec=sh -c %i\nMimeType=video/webm;video/mp4;\n",
                Some(vec!["video/webm", "video/mp4"]),
            ),
            (
                "[Thumbnailer Entry]\nTryExec=qol-no-such-tool\nExec=sh %i\nMimeType=video/webm;\n",
                None,
            ),
            (
                "[Thumbnailer Entry]\nExec=qol-no-such-tool %i\nMimeType=video/webm;\n",
                None,
            ),
            ("[Other]\nExec=sh %i\nMimeType=video/webm;\n", None),
            ("[Thumbnailer Entry]\nExec=sh %i\n", None),
        ];
        for (text, expected) in cases {
            let parsed = Thumbnailer::parse(text).map(|thumbnailer| thumbnailer.mimes);
            let expected =
                expected.map(|mimes| mimes.into_iter().map(str::to_string).collect::<Vec<_>>());
            assert_eq!(parsed, expected, "{text}");
        }
    }
}
