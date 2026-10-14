use std::ffi::OsStr;
use std::path::{Path, PathBuf};

mod platform;

const KITTEN: &str = "kitten";

pub fn locate_kitten() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    resolve(
        std::env::var_os("PATH").as_deref(),
        &platform::install_candidates(home.as_deref()),
        platform::is_executable,
    )
}

pub(super) fn kitten_program() -> PathBuf {
    locate_kitten().unwrap_or_else(|| PathBuf::from(KITTEN))
}

fn resolve(
    path_var: Option<&OsStr>,
    install_candidates: &[PathBuf],
    is_executable: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    path_var
        .into_iter()
        .flat_map(std::env::split_paths)
        .map(|directory| directory.join(KITTEN))
        .chain(install_candidates.iter().cloned())
        .find(|candidate| is_executable(candidate))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn path_var(directories: &[&str]) -> OsString {
        std::env::join_paths(directories).unwrap()
    }

    #[test]
    fn path_entry_wins_over_install_candidates() {
        let path = path_var(&["/usr/bin", "/opt/kitty/bin"]);
        let candidates = [PathBuf::from(
            "/Applications/kitty.app/Contents/MacOS/kitten",
        )];
        let found = resolve(Some(&path), &candidates, |candidate| {
            candidate == Path::new("/opt/kitty/bin/kitten") || candidate == candidates[0].as_path()
        });
        assert_eq!(found, Some(PathBuf::from("/opt/kitty/bin/kitten")));
    }

    #[test]
    fn install_candidate_is_used_when_path_has_no_kitten() {
        let path = path_var(&["/usr/bin", "/bin", "/usr/sbin", "/sbin"]);
        let candidates = [
            PathBuf::from("/Applications/kitty.app/Contents/MacOS/kitten"),
            PathBuf::from("/Users/me/Applications/kitty.app/Contents/MacOS/kitten"),
        ];
        let found = resolve(Some(&path), &candidates, |candidate| {
            candidate == candidates[1].as_path()
        });
        assert_eq!(found, Some(candidates[1].clone()));
    }

    #[test]
    fn missing_path_variable_still_checks_install_candidates() {
        let candidates = [PathBuf::from("/home/me/.local/kitty.app/bin/kitten")];
        let found = resolve(None, &candidates, |_| true);
        assert_eq!(found, Some(candidates[0].clone()));
    }

    #[test]
    fn nothing_executable_resolves_to_none() {
        let path = path_var(&["/usr/bin"]);
        let candidates = [PathBuf::from(
            "/Applications/kitty.app/Contents/MacOS/kitten",
        )];
        assert_eq!(resolve(Some(&path), &candidates, |_| false), None);
    }

    #[test]
    fn real_executable_is_found_on_a_path_directory() {
        let directory = tempfile::tempdir().unwrap();
        let kitten = directory.path().join(KITTEN);
        std::fs::write(&kitten, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&kitten, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = path_var(&[directory.path().to_str().unwrap()]);
        assert_eq!(
            resolve(Some(&path), &[], platform::is_executable),
            Some(kitten)
        );
    }
}
