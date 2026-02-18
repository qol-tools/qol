use std::fs;
use std::path::{Path, PathBuf};

const EXEC_FIELD_CODES: &[&str] = &[
    "%u", "%U", "%f", "%F", "%i", "%c", "%k",
    "%d", "%D", "%n", "%N", "%v", "%m",
];

#[derive(Debug, Clone)]
pub struct DesktopEntry {
    pub name: String,
    pub exec: Vec<String>,
    pub path: PathBuf,
}


pub fn scan(dirs: &[PathBuf]) -> Vec<DesktopEntry> {
    let mut entries: Vec<DesktopEntry> = dirs
        .iter()
        .filter_map(|dir| fs::read_dir(dir).ok())
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "desktop"))
        .filter_map(|p| parse(&p))
        .collect();

    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    entries.dedup_by(|a, b| a.name.to_lowercase() == b.name.to_lowercase());
    entries
}

fn parse(path: &Path) -> Option<DesktopEntry> {
    let content = fs::read_to_string(path).ok()?;

    let field = |prefix: &str| {
        content
            .lines()
            .find(|l| l.starts_with(prefix))
            .map(|l| l[prefix.len()..].to_string())
    };

    if content.lines().any(|l| l == "NoDisplay=true" || l == "Hidden=true") {
        return None;
    }

    let exec_raw = field("Exec=")?;
    let exec = shell_words::split(&exec_raw)
        .ok()?
        .into_iter()
        .filter(|token| !EXEC_FIELD_CODES.contains(&token.as_str()))
        .collect();

    Some(DesktopEntry {
        name: field("Name=")?,
        exec,
        path: path.to_path_buf(),
    })
}
