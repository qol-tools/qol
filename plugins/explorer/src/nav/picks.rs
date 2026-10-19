use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct Picks {
    picked: HashSet<PathBuf>,
    range: Option<Range>,
}

#[derive(Debug, Clone)]
struct Range {
    anchor: PathBuf,
    base: HashSet<PathBuf>,
}

impl Picks {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_picked(&self, path: &Path) -> bool {
        self.picked.contains(path)
    }

    pub fn len(&self) -> usize {
        self.picked.len()
    }

    pub fn is_empty(&self) -> bool {
        self.picked.is_empty()
    }

    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.picked.iter().map(PathBuf::as_path)
    }

    pub fn toggle(&mut self, path: &Path) {
        self.range = None;
        if !self.picked.remove(path) {
            self.picked.insert(path.to_path_buf());
        }
    }

    pub fn add(&mut self, path: &Path) {
        self.range = None;
        self.picked.insert(path.to_path_buf());
    }

    pub fn all<T: AsRef<Path>>(&mut self, rows: &[T]) {
        self.range = None;
        self.picked
            .extend(rows.iter().map(|row| row.as_ref().to_path_buf()));
    }

    pub fn clear(&mut self) {
        self.range = None;
        self.picked.clear();
    }

    pub fn retain(&mut self, keep: impl Fn(&Path) -> bool) {
        self.picked.retain(|path| keep(path));
        if let Some(range) = self.range.as_mut() {
            range.base.retain(|path| keep(path));
        }
    }

    pub fn extend_range<T: AsRef<Path>>(&mut self, rows: &[T], from: usize, to: usize) {
        let Some(start) = rows.get(from) else {
            return;
        };
        let range = self.range.get_or_insert_with(|| Range {
            anchor: start.as_ref().to_path_buf(),
            base: self.picked.clone(),
        });
        let anchor = rows
            .iter()
            .position(|row| row.as_ref() == range.anchor)
            .unwrap_or(from);
        let low = anchor.min(to);
        let high = anchor.max(to);
        let mut picked = range.base.clone();
        picked.extend(
            rows.iter()
                .take(high + 1)
                .skip(low)
                .map(|row| row.as_ref().to_path_buf()),
        );
        self.picked = picked;
    }

    pub fn end_range(&mut self) {
        self.range = None;
    }
}
