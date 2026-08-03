use std::path::PathBuf;

pub(super) trait PiEnvironment: Send + Sync {
    fn session_file(&self, cwd: &str) -> Option<PathBuf>;
}

pub(super) struct SystemPiEnvironment;

impl PiEnvironment for SystemPiEnvironment {
    fn session_file(&self, cwd: &str) -> Option<PathBuf> {
        let directory = agent_dir()?.join("sessions").join(session_dir_name(cwd));
        newest_session_file(&directory)
    }
}

fn agent_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("PI_CODING_AGENT_DIR") {
        let dir = PathBuf::from(dir);
        return expand_tilde(dir);
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".pi").join("agent"))
}

fn expand_tilde(path: PathBuf) -> Option<PathBuf> {
    let text = path.to_str()?;
    if text == "~" || text.starts_with("~/") {
        let home = std::env::var_os("HOME").map(PathBuf::from)?;
        Some(home.join(text.strip_prefix('~').unwrap_or_default()))
    } else {
        Some(path)
    }
}

fn session_dir_name(cwd: &str) -> String {
    let trimmed = cwd.trim_start_matches(['/', '\\']);
    let encoded: String = trimmed
        .chars()
        .map(|character| match character {
            '/' | '\\' | ':' => '-',
            other => other,
        })
        .collect();
    format!("--{encoded}--")
}

fn newest_session_file(directory: &PathBuf) -> Option<PathBuf> {
    std::fs::read_dir(directory)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        .filter(|path| path.is_file())
        .max_by_key(|path| {
            path.metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
        })
}

#[cfg(test)]
mod tests {
    use super::session_dir_name;

    #[test]
    fn session_dir_name_matches_pis_encoding() {
        assert_eq!(
            session_dir_name("/media/kmrh47/WD_SN850X/Git/qol-monorepo"),
            "--media-kmrh47-WD_SN850X-Git-qol-monorepo--"
        );
        assert_eq!(session_dir_name("/"), "----");
    }
}
