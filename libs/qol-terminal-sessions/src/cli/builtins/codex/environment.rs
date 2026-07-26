use std::path::PathBuf;
use std::process::Command;

pub(super) trait CodexEnvironment: Send + Sync {
    fn open_rollout(&self, pid: i32) -> Option<PathBuf>;
    fn session_index_path(&self) -> Option<PathBuf>;
}

pub(super) struct SystemCodexEnvironment;

impl CodexEnvironment for SystemCodexEnvironment {
    fn open_rollout(&self, pid: i32) -> Option<PathBuf> {
        let output = Command::new("lsof")
            .args(["-p", &pid.to_string(), "-Fn"])
            .output()
            .ok()?;
        String::from_utf8(output.stdout)
            .ok()?
            .lines()
            .filter_map(|line| line.strip_prefix('n'))
            .find(|path| {
                path.contains("/sessions/")
                    && path.contains("/rollout-")
                    && path.ends_with(".jsonl")
            })
            .map(PathBuf::from)
    }

    fn session_index_path(&self) -> Option<PathBuf> {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join(".codex").join("session_index.jsonl"))
    }
}
