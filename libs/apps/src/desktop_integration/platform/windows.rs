use std::ffi::{OsStr, OsString};
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::RevealPlan;

const EXPLORER: &str = "explorer.exe";
const WINDOWS_DIRECTORY_CAPACITY: usize = 32_768;

#[link(name = "kernel32")]
extern "system" {
    fn GetWindowsDirectoryW(buffer: *mut u16, size: u32) -> u32;
}

pub(super) use self::open as open_checked;

pub(super) fn open(target: &OsStr) -> io::Result<()> {
    crate::shell_execute::shell_execute(target, &[])
}

pub(super) fn reveal_plan(path: &Path) -> RevealPlan {
    reveal_with(windows_directory(), path)
}

fn reveal_with(windows: Option<PathBuf>, path: &Path) -> RevealPlan {
    let Some(windows) = windows else {
        let folder = if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(path)
        };
        return folder.to_path_buf().into();
    };
    let mut command = Command::new(windows.join(EXPLORER));
    command.raw_arg(select_argument(path));
    command.into()
}

fn select_argument(path: &Path) -> OsString {
    let mut argument = OsString::from("/select,\"");
    argument.push(path.as_os_str());
    argument.push("\"");
    argument
}

fn windows_directory() -> Option<PathBuf> {
    let mut buffer = vec![0u16; WINDOWS_DIRECTORY_CAPACITY];
    let length = unsafe { GetWindowsDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    (length > 0 && length < buffer.len())
        .then(|| PathBuf::from(OsString::from_wide(&buffer[..length])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_selects_the_path_with_explorer_from_the_windows_directory() {
        let cases = [
            (r"C:\capture.png", r#"/select,"C:\capture.png""#),
            (
                r"C:\Users\me\My Shots\a,b %USERNAME%.png",
                r#"/select,"C:\Users\me\My Shots\a,b %USERNAME%.png""#,
            ),
        ];
        for (path, expected) in cases {
            let RevealPlan::Command(command) =
                reveal_with(Some(PathBuf::from(r"C:\Windows")), Path::new(path))
            else {
                panic!("Windows reveal must use Explorer");
            };
            assert_eq!(
                command.get_program(),
                OsStr::new(r"C:\Windows\explorer.exe")
            );
            assert_eq!(
                command.get_args().collect::<Vec<_>>(),
                [OsStr::new(expected)]
            );
        }
    }

    #[test]
    fn reveal_opens_the_folder_when_the_windows_directory_is_unknown() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("capture.png");
        std::fs::write(&file, []).unwrap();
        let RevealPlan::Open(folder) = reveal_with(None, &file) else {
            panic!("an unknown Windows directory must not run a bare Explorer");
        };
        assert_eq!(folder, temp.path());
    }

    #[test]
    fn the_windows_directory_holds_explorer() {
        let windows = windows_directory().expect("Windows directory");
        assert!(windows.join(EXPLORER).is_file());
    }

    #[test]
    fn opening_a_missing_file_fails_without_expanding_its_name() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("%USERNAME%-missing.txt");
        assert_eq!(
            open(missing.as_os_str()).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert_eq!(
            open_checked(missing.as_os_str()).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }
}
