use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct Picks {
    picked: HashSet<PathBuf>,
    folders: HashMap<PathBuf, usize>,
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

    pub fn count_in(&self, folder: &Path) -> usize {
        self.folders.get(folder).copied().unwrap_or(0)
    }

    pub fn folder_count(&self) -> usize {
        self.folders.len()
    }

    pub fn toggle(&mut self, path: &Path) {
        self.range = None;
        if !self.remove(path) {
            self.insert(path.to_path_buf());
        }
    }

    pub fn add(&mut self, path: &Path) {
        self.range = None;
        self.insert(path.to_path_buf());
    }

    pub fn all<T: AsRef<Path>>(&mut self, rows: &[T]) {
        self.range = None;
        for row in rows {
            self.insert(row.as_ref().to_path_buf());
        }
    }

    pub fn set(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        self.clear();
        for path in paths {
            self.insert(path);
        }
    }

    pub fn clear(&mut self) {
        self.range = None;
        self.picked.clear();
        self.folders.clear();
    }

    pub fn retain(&mut self, keep: impl Fn(&Path) -> bool) {
        self.picked.retain(|path| keep(path));
        self.recount();
        if let Some(range) = self.range.as_mut() {
            range.base.retain(|path| keep(path));
        }
    }

    pub fn rename(&mut self, from: &Path, to: PathBuf) {
        if self.remove(from) {
            self.insert(to.clone());
        }
        let Some(range) = self.range.as_mut() else {
            return;
        };
        if range.base.remove(from) {
            range.base.insert(to.clone());
        }
        if range.anchor == from {
            range.anchor = to;
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
        self.recount();
    }

    pub fn end_range(&mut self) {
        self.range = None;
    }

    fn insert(&mut self, path: PathBuf) {
        if self.picked.contains(&path) {
            return;
        }
        if let Some(folder) = path.parent() {
            tally(&mut self.folders, folder);
        }
        self.picked.insert(path);
    }

    fn remove(&mut self, path: &Path) -> bool {
        if !self.picked.remove(path) {
            return false;
        }
        let Some(folder) = path.parent() else {
            return true;
        };
        if let Some(count) = self.folders.get_mut(folder) {
            *count -= 1;
            if *count == 0 {
                self.folders.remove(folder);
            }
        }
        true
    }

    fn recount(&mut self) {
        self.folders.clear();
        for folder in self.picked.iter().filter_map(|path| path.parent()) {
            tally(&mut self.folders, folder);
        }
    }
}

fn tally(folders: &mut HashMap<PathBuf, usize>, folder: &Path) {
    match folders.get_mut(folder) {
        Some(count) => *count += 1,
        None => {
            folders.insert(folder.to_path_buf(), 1);
        }
    }
}
