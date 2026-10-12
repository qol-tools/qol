use std::net::TcpListener;
use std::path::{Path, PathBuf};

const DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";

pub(in crate::daemon) fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
}

pub(in crate::daemon) fn executable_candidates(directory: &Path, program: &str) -> Vec<PathBuf> {
    let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| DEFAULT_PATHEXT.to_string());
    program_file_names(program, &pathext)
        .into_iter()
        .map(|name| directory.join(name))
        .collect()
}

pub(in crate::daemon) fn launch_path(path: &Path) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(path).map(qol_platform::windows_path::strip_verbatim)
}

pub(in crate::daemon) fn inherited_listener() -> std::io::Result<Option<TcpListener>> {
    Ok(None)
}

pub(in crate::daemon) fn system_temp_dir() -> PathBuf {
    std::env::temp_dir()
}

fn program_file_names(program: &str, pathext: &str) -> Vec<String> {
    if Path::new(program).extension().is_some() {
        return vec![program.to_string()];
    }
    pathext
        .split(';')
        .map(str::trim)
        .filter(|extension| extension.len() > 1 && extension.starts_with('.'))
        .map(|extension| format!("{program}{}", extension.to_ascii_lowercase()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_file_names_follow_pathext() {
        let cases: [(&str, &str, &[&str]); 4] = [
            (
                "code",
                ".COM;.EXE;.CMD",
                &["code.com", "code.exe", "code.cmd"],
            ),
            ("code.cmd", ".EXE", &["code.cmd"]),
            ("git", " .EXE ; ;bad", &["git.exe"]),
            ("idea64", "", &[]),
        ];
        for (program, pathext, expected) in cases {
            assert_eq!(
                program_file_names(program, pathext),
                expected
                    .iter()
                    .map(|name| name.to_string())
                    .collect::<Vec<_>>(),
                "program={program:?} pathext={pathext:?}"
            );
        }
    }
}
