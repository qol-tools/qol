use qol_theme::Mark;
use std::path::{Path, PathBuf};

pub const MARKS_DIR_NAME: &str = "launcher-icons";

pub fn marks_dir() -> Option<PathBuf> {
    qol_config::data_dir().map(|dir| dir.join(MARKS_DIR_NAME))
}

pub fn mark_path(dir: &Path, mark: Mark) -> PathBuf {
    dir.join(format!("{}.svg", mark.name()))
}

pub fn mark_at(dir: &Path, path: &Path) -> Option<Mark> {
    let mark = Mark::from_name(path.file_stem()?.to_str()?)?;
    (mark_path(dir, mark) == path).then_some(mark)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mark_file_reads_back_as_its_mark() {
        let dir = Path::new("/data/launcher-icons");
        for mark in Mark::ALL {
            assert_eq!(mark_at(dir, &mark_path(dir, mark)), Some(mark));
        }
    }

    #[test]
    fn other_files_are_not_marks() {
        let dir = Path::new("/data/launcher-icons");
        assert_eq!(mark_at(dir, Path::new("/icons/sound.svg")), None);
        assert_eq!(mark_at(dir, &dir.join("sound.png")), None);
        assert_eq!(mark_at(dir, &dir.join("no-such-mark.svg")), None);
    }
}
