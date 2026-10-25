use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use qol_process::BoundedCommandOutput;

use super::super::super::{Flavor, ThumbError};
use super::cache::Stamp;
use super::sandbox;

const RUN_TIMEOUT: Duration = Duration::from_secs(15);
const OUTPUT_LIMIT: usize = 4096;
const ENTRY_HEADER: &str = "[Thumbnailer Entry]";

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
        if try_exec.is_some_and(|program| !installed(&program)) {
            return None;
        }
        let exec: Vec<String> = exec?.split_whitespace().map(str::to_string).collect();
        if exec.first().is_none_or(|program| !installed(program)) {
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
        let partial = folder.join(format!(
            ".{}.partial.png",
            target
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("thumbnail")
        ));
        let words = self.words(input, &stamp.uri, flavor.px(), &partial);
        let command = sandbox::command(&words, input, folder).ok_or_else(|| {
            failed("previews need bubblewrap (bwrap) to run thumbnailers sandboxed".into())
        })?;
        let outcome =
            qol_process::run_owned_with_output_timeout(command, RUN_TIMEOUT, OUTPUT_LIMIT);
        let made = partial.metadata().is_ok_and(|meta| meta.len() > 0);
        let result = match outcome {
            Ok(BoundedCommandOutput::Completed(output)) if output.status.success() && made => {
                fs::rename(&partial, target).map_err(|error| failed(error.to_string()))
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
        };
        if result.is_err() {
            let _ = fs::remove_file(&partial);
        }
        result
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

fn installed(program: &str) -> bool {
    super::on_path(program).is_some()
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
