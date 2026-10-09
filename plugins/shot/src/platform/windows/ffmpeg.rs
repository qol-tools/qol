use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub(super) const INSTALL_HINT: &str = "Install ffmpeg with: winget install Gyan.FFmpeg";

const EXECUTABLE: &str = "ffmpeg.exe";
const WINGET_PACKAGE_PREFIX: &str = "Gyan.FFmpeg";

pub(super) fn resolve_ffmpeg() -> Option<PathBuf> {
    let local_app_data = env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let candidates = ffmpeg_candidates(
        env::var_os("PATH"),
        local_app_data.as_deref(),
        env::var_os("ProgramFiles").map(PathBuf::from).as_deref(),
        env::current_exe().ok().as_deref(),
    );
    candidates
        .into_iter()
        .chain(
            local_app_data
                .as_deref()
                .into_iter()
                .flat_map(winget_packages),
        )
        .find(|path| path.is_file())
}

fn ffmpeg_candidates(
    path: Option<OsString>,
    local_app_data: Option<&Path>,
    program_files: Option<&Path>,
    current_exe: Option<&Path>,
) -> Vec<PathBuf> {
    let mut dirs = path
        .map(|paths| env::split_paths(&paths).collect::<Vec<_>>())
        .unwrap_or_default();
    dirs.extend(local_app_data.map(|dir| dir.join("Microsoft").join("WinGet").join("Links")));
    dirs.extend(program_files.map(|dir| dir.join("WinGet").join("Links")));
    dirs.extend(current_exe.and_then(Path::parent).map(Path::to_path_buf));
    dirs.into_iter()
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.join(EXECUTABLE))
        .collect()
}

fn winget_packages(local_app_data: &Path) -> Vec<PathBuf> {
    let packages = local_app_data
        .join("Microsoft")
        .join("WinGet")
        .join("Packages");
    let Ok(entries) = std::fs::read_dir(packages) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(WINGET_PACKAGE_PREFIX)
        })
        .filter_map(|entry| std::fs::read_dir(entry.path()).ok())
        .flatten()
        .filter_map(Result::ok)
        .map(|build| build.path().join("bin").join(EXECUTABLE))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::ffmpeg_candidates;
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    #[test]
    fn candidates_search_path_then_winget_links_then_the_bundle() {
        let path = std::env::join_paths([r"C:\tools", r"C:\bin"]).unwrap();
        let cases: [(Option<OsString>, Vec<&str>); 2] = [
            (
                Some(path),
                vec![
                    r"C:\tools\ffmpeg.exe",
                    r"C:\bin\ffmpeg.exe",
                    r"C:\Users\me\AppData\Local\Microsoft\WinGet\Links\ffmpeg.exe",
                    r"C:\Program Files\WinGet\Links\ffmpeg.exe",
                    r"C:\qol\plugins\shot\ffmpeg.exe",
                ],
            ),
            (
                None,
                vec![
                    r"C:\Users\me\AppData\Local\Microsoft\WinGet\Links\ffmpeg.exe",
                    r"C:\Program Files\WinGet\Links\ffmpeg.exe",
                    r"C:\qol\plugins\shot\ffmpeg.exe",
                ],
            ),
        ];
        for (path, expected) in cases {
            let candidates = ffmpeg_candidates(
                path,
                Some(Path::new(r"C:\Users\me\AppData\Local")),
                Some(Path::new(r"C:\Program Files")),
                Some(Path::new(r"C:\qol\plugins\shot\qol-shot.exe")),
            );
            let expected = expected.into_iter().map(PathBuf::from).collect::<Vec<_>>();
            assert_eq!(candidates, expected);
        }
    }
}
