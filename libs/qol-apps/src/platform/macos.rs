use std::path::PathBuf;
use std::process::Command;

pub(crate) fn spotlight_app_paths(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut command = Command::new("/usr/bin/mdfind");
    for root in roots {
        command.arg("-onlyin").arg(root);
    }
    command.arg("kMDItemContentType == 'com.apple.application-bundle'");
    let Ok(output) = command.output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect()
}
