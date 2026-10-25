use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use qol_process::BoundedCommandOutput;

use super::super::{prepare_external_command, OpenHandler};
use crate::desktop::{desktop_field, linux_app_roots, scan_desktop_root};

const QUERY_TIMEOUT: Duration = Duration::from_secs(5);
const QUERY_LIMIT: usize = 4096;

struct Candidate {
    id: String,
    name: String,
    mime_types: Vec<String>,
    exec: String,
}

pub(in super::super) fn open_with_supported() -> bool {
    true
}

pub(in super::super) fn file_type(path: &Path) -> io::Result<String> {
    let mut command = Command::new("xdg-mime");
    command.arg("query").arg("filetype").arg(path);
    let output = query(command)?;
    if output.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "the file type is unknown",
        ));
    }
    Ok(output)
}

pub(in super::super) fn open_handlers(path: &Path) -> io::Result<Vec<OpenHandler>> {
    let mime = file_type(path)?;
    let mut command = Command::new("xdg-mime");
    command.arg("query").arg("default").arg(&mime);
    let default = query(command).ok().filter(|id| !id.is_empty());
    Ok(handlers_for(&candidates(), &mime, default.as_deref()))
}

pub(in super::super) fn open_with_handler(handler: &str, paths: &[PathBuf]) -> io::Result<()> {
    let candidate = candidates()
        .into_iter()
        .find(|candidate| candidate.id == handler)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "the app is no longer installed"))?;
    for mut command in launch_commands(&candidate.exec, paths)? {
        prepare_external_command(&mut command);
        qol_process::spawn_detached(&mut command)?;
    }
    Ok(())
}

fn query(command: Command) -> io::Result<String> {
    match qol_process::run_owned_with_output_timeout(command, QUERY_TIMEOUT, QUERY_LIMIT)? {
        BoundedCommandOutput::Completed(output) if output.status.success() => {
            Ok(String::from_utf8_lossy(output.stdout.as_bytes())
                .trim()
                .to_string())
        }
        BoundedCommandOutput::Completed(_) => Err(io::Error::other("the file type query failed")),
        BoundedCommandOutput::TimedOut { .. } => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "the file type query timed out",
        )),
    }
}

fn candidates() -> Vec<Candidate> {
    let mut found: Vec<Candidate> = Vec::new();
    for root in linux_app_roots() {
        for entry in scan_desktop_root(&root) {
            let Some(id) = entry.path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if found.iter().any(|candidate| candidate.id == id) {
                continue;
            }
            let Ok(content) = fs::read_to_string(&entry.path) else {
                continue;
            };
            if desktop_field(&content, "Terminal=").as_deref() == Some("true") {
                continue;
            }
            let (Some(exec), Some(mimes)) = (
                desktop_field(&content, "Exec="),
                desktop_field(&content, "MimeType="),
            ) else {
                continue;
            };
            found.push(Candidate {
                id: id.to_string(),
                name: entry.name,
                mime_types: mimes
                    .split(';')
                    .filter(|mime| !mime.is_empty())
                    .map(str::to_string)
                    .collect(),
                exec,
            });
        }
    }
    found
}

fn handlers_for(candidates: &[Candidate], mime: &str, default: Option<&str>) -> Vec<OpenHandler> {
    let mut handlers: Vec<OpenHandler> = candidates
        .iter()
        .filter(|candidate| candidate.mime_types.iter().any(|known| known == mime))
        .map(|candidate| OpenHandler {
            id: candidate.id.clone(),
            name: candidate.name.clone(),
            is_default: default == Some(candidate.id.as_str()),
        })
        .collect();
    handlers.sort_by(|a, b| {
        b.is_default
            .cmp(&a.is_default)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    handlers
}

fn launch_commands(exec: &str, paths: &[PathBuf]) -> io::Result<Vec<Command>> {
    let tokens = shell_words::split(exec).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "the app's launcher is malformed",
        )
    })?;
    let takes_many = tokens.iter().any(|token| token == "%F" || token == "%U");
    let takes_one = tokens.iter().any(|token| {
        let codes = token.replace("%%", "");
        codes.contains("%f") || codes.contains("%u")
    });
    let batches: Vec<&[PathBuf]> = if takes_many || !takes_one {
        vec![paths]
    } else {
        paths.chunks(1).collect()
    };
    batches
        .into_iter()
        .map(|batch| expand(&tokens, batch))
        .collect()
}

fn expand(tokens: &[String], files: &[PathBuf]) -> io::Result<Command> {
    let mut words: Vec<OsString> = Vec::new();
    for token in tokens {
        match token.as_str() {
            "%f" | "%F" => words.extend(files.iter().map(OsString::from)),
            "%u" | "%U" => words.extend(files.iter().map(|file| file_uri(file))),
            "%i" | "%c" | "%k" => {}
            _ => words.push(substitute(token, files.first())),
        }
    }
    let mut words = words.into_iter();
    let program = words
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "the app's launcher is empty"))?;
    let mut command = Command::new(program);
    command.args(words);
    Ok(command)
}

fn substitute(token: &str, file: Option<&PathBuf>) -> OsString {
    let mut word = OsString::new();
    let mut chars = token.chars();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            word.push(ch.to_string());
            continue;
        }
        match (chars.next(), file) {
            (Some('%'), _) => word.push("%"),
            (Some('f'), Some(file)) => word.push(file),
            (Some('u'), Some(file)) => word.push(file_uri(file)),
            _ => {}
        }
    }
    word
}

fn file_uri(path: &Path) -> OsString {
    let mut uri = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            uri.push(char::from(byte));
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri.into()
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    fn candidate(id: &str, name: &str, mimes: &[&str]) -> Candidate {
        Candidate {
            id: id.into(),
            name: name.into(),
            mime_types: mimes.iter().map(|mime| mime.to_string()).collect(),
            exec: String::new(),
        }
    }

    fn names(handlers: &[OpenHandler]) -> Vec<(&str, bool)> {
        handlers
            .iter()
            .map(|handler| (handler.name.as_str(), handler.is_default))
            .collect()
    }

    #[test]
    fn the_default_comes_first_then_the_rest_by_name() {
        let all = [
            candidate("z.desktop", "Zed", &["text/plain"]),
            candidate("a.desktop", "alpha", &["text/plain"]),
            candidate("m.desktop", "Mid", &["text/plain", "image/png"]),
            candidate("p.desktop", "Paint", &["image/png"]),
        ];
        let cases = [
            (
                "text/plain",
                Some("m.desktop"),
                vec![("Mid", true), ("alpha", false), ("Zed", false)],
            ),
            (
                "text/plain",
                None,
                vec![("alpha", false), ("Mid", false), ("Zed", false)],
            ),
            (
                "image/png",
                Some("gone.desktop"),
                vec![("Mid", false), ("Paint", false)],
            ),
            ("video/mp4", Some("m.desktop"), vec![]),
        ];
        for (mime, default, expected) in cases {
            assert_eq!(
                names(&handlers_for(&all, mime, default)),
                expected,
                "{mime}"
            );
        }
    }

    fn words(command: &Command) -> Vec<&OsStr> {
        std::iter::once(command.get_program())
            .chain(command.get_args())
            .collect()
    }

    #[test]
    fn exec_codes_expand_to_the_files() {
        let files = [PathBuf::from("/a b.txt"), PathBuf::from("/c.txt")];
        let many = launch_commands("editor --new %F", &files).unwrap();
        assert_eq!(many.len(), 1);
        assert_eq!(
            words(&many[0]),
            ["editor", "--new", "/a b.txt", "/c.txt"].map(OsStr::new)
        );
        let one = launch_commands("viewer %f", &files).unwrap();
        assert_eq!(one.len(), 2);
        assert_eq!(words(&one[1]), ["viewer", "/c.txt"].map(OsStr::new));
        let bare = launch_commands("tool --icon %i --100%%", &files).unwrap();
        assert_eq!(
            words(&bare[0]),
            ["tool", "--icon", "--100%"].map(OsStr::new)
        );
    }

    #[test]
    fn codes_inside_a_token_expand_and_uri_codes_get_file_uris() {
        let files = [PathBuf::from("/a b.txt"), PathBuf::from("/c.txt")];
        let cases: [(&str, Vec<Vec<&str>>); 4] = [
            (
                "viewer --file=%f",
                vec![
                    vec!["viewer", "--file=/a b.txt"],
                    vec!["viewer", "--file=/c.txt"],
                ],
            ),
            (
                "browser %U",
                vec![vec!["browser", "file:///a%20b.txt", "file:///c.txt"]],
            ),
            (
                "sh -c \"viewer %u\"",
                vec![
                    vec!["sh", "-c", "viewer file:///a%20b.txt"],
                    vec!["sh", "-c", "viewer file:///c.txt"],
                ],
            ),
            ("tool %c --50%% %k", vec![vec!["tool", "--50%"]]),
        ];
        for (exec, expected) in cases {
            let commands = launch_commands(exec, &files).unwrap();
            let got: Vec<Vec<&OsStr>> = commands.iter().map(words).collect();
            let expected: Vec<Vec<&OsStr>> = expected
                .iter()
                .map(|command| command.iter().map(OsStr::new).collect())
                .collect();
            assert_eq!(got, expected, "{exec}");
        }
    }

    #[test]
    fn an_empty_or_broken_launcher_is_an_error() {
        assert!(launch_commands("", &[PathBuf::from("/a")]).is_err());
        assert!(launch_commands("\"unterminated %f", &[PathBuf::from("/a")]).is_err());
    }
}
