use anyhow::{Context, Result};
use qol_plugin_api::launcher_marks;
use qol_theme::Mark;
use std::path::{Path, PathBuf};

pub struct MarkFiles {
    dir: PathBuf,
    line: u32,
}

impl MarkFiles {
    pub fn locate() -> Option<Self> {
        launcher_marks::marks_dir().map(|dir| Self::in_dir(dir, theme_line()))
    }

    pub fn in_dir(dir: PathBuf, line: u32) -> Self {
        Self { dir, line }
    }

    pub fn svg(&self, mark: Mark) -> String {
        mark.svg(self.line)
    }

    pub fn write(&self) -> Result<()> {
        std::fs::create_dir_all(&self.dir).with_context(|| {
            format!("Failed to create launcher icon dir {}", self.dir.display())
        })?;
        for mark in Mark::ALL {
            write_if_changed(&self.path(mark), &self.svg(mark))?;
        }
        self.remove_stale()
    }

    pub fn path(&self, mark: Mark) -> PathBuf {
        launcher_marks::mark_path(&self.dir, mark)
    }

    fn remove_stale(&self) -> Result<()> {
        for entry in std::fs::read_dir(&self.dir)?.flatten() {
            let path = entry.path();
            if launcher_marks::mark_at(&self.dir, &path).is_none() {
                std::fs::remove_file(&path)
                    .with_context(|| format!("Failed to remove {}", path.display()))?;
            }
        }
        Ok(())
    }
}

fn theme_line() -> u32 {
    let native = crate::features::theme::current_native_theme_key();
    let accent = crate::features::theme::native_accent_wire();
    let theme = qol_theme::theme_for_native_key(Some(&native), Some(&accent));
    Mark::line_on(&qol_theme::Grounds::from_theme(theme.mode, theme.system).pane)
}

fn write_if_changed(path: &Path, content: &str) -> Result<()> {
    if std::fs::read_to_string(path).is_ok_and(|current| current == content) {
        return Ok(());
    }
    std::fs::write(path, content).with_context(|| format!("Failed to write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_every_mark_and_removes_strays() {
        let dir = tempfile::tempdir().unwrap();
        let stray = dir.path().join("retired-mark.svg");
        std::fs::write(&stray, "<svg/>").unwrap();

        let files = MarkFiles::in_dir(dir.path().to_path_buf(), 0x8a93f7);
        files.write().unwrap();

        for mark in Mark::ALL {
            assert_eq!(
                std::fs::read_to_string(files.path(mark)).unwrap(),
                mark.svg(0x8a93f7)
            );
        }
        assert!(!stray.exists());
    }

    #[test]
    fn leaves_unchanged_marks_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let files = MarkFiles::in_dir(dir.path().to_path_buf(), 0x8a93f7);
        files.write().unwrap();
        let path = files.path(Mark::Qol);
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();

        files.write().unwrap();

        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), old);
    }
}
