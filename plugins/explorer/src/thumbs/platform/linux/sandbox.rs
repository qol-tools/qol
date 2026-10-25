use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const ROOT_LINKS: [&str; 5] = ["/bin", "/lib", "/lib64", "/lib32", "/sbin"];
const READ_ONLY: [&str; 3] = ["/etc/ld.so.cache", "/etc/alternatives", "/etc/fonts"];
const SANDBOX_PATH: &str = "/usr/bin:/bin";

pub(super) fn command(words: &[OsString], input: &Path, folder: &Path) -> Option<Command> {
    let bwrap = super::on_path("bwrap")?;
    Some(sandboxed(&bwrap, words, input, folder))
}

pub(super) fn finds(program: &str) -> bool {
    let path = Path::new(program);
    let candidates: Vec<PathBuf> = if path.is_absolute() {
        vec![path.to_path_buf()]
    } else {
        std::env::split_paths(SANDBOX_PATH)
            .map(|dir| dir.join(program))
            .collect()
    };
    candidates
        .iter()
        .filter_map(|candidate| fs::canonicalize(candidate).ok())
        .any(|real| real.is_file() && visible(&real))
}

fn visible(path: &Path) -> bool {
    path.starts_with("/usr") || ROOT_LINKS.iter().any(|link| path.starts_with(link))
}

fn sandboxed(bwrap: &Path, words: &[OsString], input: &Path, folder: &Path) -> Command {
    let mut command = Command::new(bwrap);
    command.env_clear().env("PATH", SANDBOX_PATH);
    command.args(["--ro-bind", "/usr", "/usr"]);
    for link in ROOT_LINKS {
        match fs::read_link(link) {
            Ok(target) => {
                command.arg("--symlink").arg(target).arg(link);
            }
            Err(_) if Path::new(link).is_dir() => {
                command.args(["--ro-bind", link, link]);
            }
            Err(_) => {}
        }
    }
    for path in READ_ONLY {
        command.args(["--ro-bind-try", path, path]);
    }
    command.args(["--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp"]);
    command.arg("--ro-bind").arg(input).arg(input);
    command.arg("--bind").arg(folder).arg(folder);
    command.args([
        "--chdir",
        "/",
        "--unshare-all",
        "--die-with-parent",
        "--new-session",
        "--",
    ]);
    command.args(words);
    command
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    #[test]
    fn a_thumbnailer_sees_only_the_system_its_file_and_its_output_folder() {
        let words = ["ffmpegthumbnailer", "-i", "/v/a b.webm"].map(OsString::from);
        let command = sandboxed(
            Path::new("/usr/bin/bwrap"),
            &words,
            Path::new("/v/a b.webm"),
            Path::new("/c/large"),
        );
        assert_eq!(command.get_program(), OsStr::new("/usr/bin/bwrap"));
        let args: Vec<&OsStr> = command.get_args().collect();
        let has = |run: &[&str]| {
            args.windows(run.len()).any(|window| {
                window
                    .iter()
                    .zip(run)
                    .all(|(arg, want)| *arg == OsStr::new(want))
            })
        };
        assert!(has(&["--ro-bind", "/usr", "/usr"]));
        assert!(has(&["--ro-bind", "/v/a b.webm", "/v/a b.webm"]));
        assert!(has(&["--bind", "/c/large", "/c/large"]));
        assert!(has(&["--unshare-all", "--die-with-parent"]));
        assert!(args.ends_with(&["--", "ffmpegthumbnailer", "-i", "/v/a b.webm"].map(OsStr::new)));
        let envs: Vec<_> = command.get_envs().collect();
        assert_eq!(envs, [(OsStr::new("PATH"), Some(OsStr::new(SANDBOX_PATH)))]);
    }

    #[test]
    fn only_programs_the_sandbox_can_see_count() {
        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("thumbnailer");
        fs::write(&outside, "#!/bin/sh\n").unwrap();
        let outside = outside.to_str().unwrap();
        let cases = [
            ("sh", true),
            ("/usr/bin/sh", true),
            ("qol-no-such-tool", false),
            (outside, false),
        ];
        for (program, expected) in cases {
            assert_eq!(finds(program), expected, "{program}");
        }
    }
}
