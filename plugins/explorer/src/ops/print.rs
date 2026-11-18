use std::collections::hash_map::RandomState;
use std::ffi::OsStr;
use std::fs::{self, Metadata};
use std::hash::BuildHasher;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::platform;

const LITTER: [&str; 3] = [".DS_Store", "Thumbs.db", "desktop.ini"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Identity {
    pub device: u64,
    pub index: [u64; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    File,
    Folder,
    Link,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Print {
    pub id: Option<Identity>,
    pub kind: Kind,
    pub len: u64,
    pub modified: Option<SystemTime>,
    pub born: Option<SystemTime>,
}

impl Print {
    pub fn read(path: &Path) -> io::Result<Self> {
        let meta = fs::symlink_metadata(path)?;
        Ok(Self::of(path, &meta))
    }

    pub fn of(path: &Path, meta: &Metadata) -> Self {
        Self {
            id: identity(path, meta),
            kind: kind_of(meta),
            len: meta.len(),
            modified: meta.modified().ok(),
            born: meta.created().ok(),
        }
    }

    pub fn same_object(&self, now: &Print) -> bool {
        self.kind == now.kind
            && matches!((self.id, now.id), (Some(then), Some(now)) if then == now)
            && kept(self.born, now.born)
    }

    pub fn unchanged(&self, now: &Print) -> bool {
        self.same_object(now)
            && (self.kind == Kind::Folder || self.len == now.len)
            && kept(self.modified, now.modified)
    }

    pub fn device(&self) -> Option<u64> {
        self.id.map(|id| id.device)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreePrint {
    pub root: Print,
    pub entries: u64,
    pub bytes: u64,
    pub shape: u64,
    pub objects: u64,
}

impl TreePrint {
    pub fn read(root: &Path, key: &RandomState) -> io::Result<Self> {
        let meta = fs::symlink_metadata(root)?;
        let top = Print::of(root, &meta);
        let mut tree = Self {
            root: top,
            entries: 0,
            bytes: 0,
            shape: 0,
            objects: 0,
        };
        tree.add_shape(key, Path::new(""), root, &meta)?;
        if top.kind != Kind::Folder {
            return Ok(tree);
        }
        let mut folders = vec![PathBuf::new()];
        while let Some(relative) = folders.pop() {
            for entry in fs::read_dir(root.join(&relative))? {
                let entry = entry?;
                let name = entry.file_name();
                if is_litter(&name) {
                    continue;
                }
                let inner = relative.join(&name);
                let path = root.join(&inner);
                let meta = fs::symlink_metadata(&path)?;
                tree.entries += 1;
                tree.add_shape(key, &inner, &path, &meta)?;
                let changed = if meta.is_dir() {
                    None
                } else {
                    platform::changed_at(&meta)
                };
                let object = (&inner, identity(&path, &meta), changed);
                tree.objects = tree.objects.wrapping_add(key.hash_one(object));
                if meta.is_dir() {
                    folders.push(inner);
                }
            }
        }
        Ok(tree)
    }

    fn add_shape(
        &mut self,
        key: &RandomState,
        relative: &Path,
        path: &Path,
        meta: &Metadata,
    ) -> io::Result<()> {
        let kind = kind_of(meta);
        let (len, modified, target) = match kind {
            Kind::File => (meta.len(), meta.modified().ok(), None),
            Kind::Link => (0, None, Some(fs::read_link(path)?)),
            Kind::Folder | Kind::Other => (0, None, None),
        };
        self.bytes += len;
        self.shape = self
            .shape
            .wrapping_add(key.hash_one((relative, kind, len, modified, target)));
        Ok(())
    }

    pub fn unchanged(&self, now: &TreePrint) -> bool {
        let root = if self.root.kind == Kind::Folder {
            self.root.same_object(&now.root)
        } else {
            self.root.unchanged(&now.root)
        };
        root && self.matches(now) && self.objects == now.objects
    }

    pub fn matches(&self, other: &TreePrint) -> bool {
        self.entries == other.entries && self.bytes == other.bytes && self.shape == other.shape
    }
}

fn identity(path: &Path, meta: &Metadata) -> Option<Identity> {
    qol_fs::file_identity(path, meta).map(|found| Identity {
        device: found.device,
        index: [found.index as u64, (found.index >> 64) as u64],
    })
}

fn kind_of(meta: &Metadata) -> Kind {
    let kind = meta.file_type();
    if kind.is_symlink() {
        Kind::Link
    } else if kind.is_dir() {
        Kind::Folder
    } else if kind.is_file() {
        Kind::File
    } else {
        Kind::Other
    }
}

fn kept(then: Option<SystemTime>, now: Option<SystemTime>) -> bool {
    match (then, now) {
        (Some(then), Some(now)) => then == now,
        (None, None) => true,
        _ => false,
    }
}

fn is_litter(name: &OsStr) -> bool {
    LITTER.iter().any(|litter| name == *litter)
}

#[cfg(test)]
mod tests {
    use std::collections::hash_map::RandomState;
    use std::fs::{self, File};
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime};

    use tempfile::TempDir;

    use super::{Kind, Print, TreePrint};

    fn then() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000)
    }

    fn set_modified(path: &Path, when: SystemTime) {
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(when)
            .unwrap();
    }

    fn write(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
        set_modified(path, then());
    }

    fn edit(path: &Path, text: &str) {
        let mut file = File::options()
            .write(true)
            .truncate(true)
            .open(path)
            .unwrap();
        file.write_all(text.as_bytes()).unwrap();
        file.set_modified(then() + Duration::from_secs(60)).unwrap();
    }

    fn replace(path: &Path, text: &str) {
        let swap = path.with_file_name(".swap");
        write(&swap, text);
        fs::rename(&swap, path).unwrap();
    }

    fn read(path: &Path) -> Print {
        Print::read(path).unwrap()
    }

    fn sample() -> (TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("tree");
        write(&root.join("a/x.txt"), "one");
        write(&root.join("b.txt"), "two");
        (dir, root)
    }

    fn tree(root: &Path, key: &RandomState) -> TreePrint {
        TreePrint::read(root, key).unwrap()
    }

    #[test]
    fn the_same_file_read_twice_is_the_same_object_and_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        write(&path, "text");
        let first = read(&path);
        let second = read(&path);
        assert!(first.same_object(&second));
        assert!(first.unchanged(&second));
        assert_eq!(first.kind, Kind::File);
        assert_eq!(first.len, 4);
        assert_eq!(first.device(), second.device());
    }

    #[test]
    fn a_file_rewritten_in_place_is_the_same_object_but_not_unchanged() {
        let rewrites = ["", "TEXT", "a much longer text"];
        for rewrite in rewrites {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("a.txt");
            write(&path, "text");
            let before = read(&path);
            edit(&path, rewrite);
            let after = read(&path);
            assert!(before.same_object(&after), "{rewrite:?}");
            assert!(!before.unchanged(&after), "{rewrite:?}");
        }
    }

    #[test]
    fn a_file_touched_without_a_new_length_is_not_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        write(&path, "text");
        let before = read(&path);
        set_modified(&path, then() + Duration::from_secs(60));
        let after = read(&path);
        assert!(before.same_object(&after));
        assert!(!before.unchanged(&after));
    }

    #[test]
    fn a_new_file_under_the_old_name_is_not_the_same_object() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        write(&path, "text");
        let before = read(&path);
        replace(&path, "text");
        let after = read(&path);
        assert!(!before.same_object(&after));
        assert!(!before.unchanged(&after));
    }

    #[test]
    fn a_file_and_a_folder_are_never_the_same_object() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a");
        write(&path, "text");
        let file = read(&path);
        let folder = read(dir.path());
        assert_eq!(folder.kind, Kind::Folder);
        assert!(!file.same_object(&folder));
        assert!(!folder.same_object(&file));
    }

    #[test]
    fn a_folder_stays_the_same_object_when_something_is_added() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("folder");
        fs::create_dir(&folder).unwrap();
        let before = read(&folder);
        write(&folder.join("added.txt"), "new");
        let after = read(&folder);
        assert!(before.same_object(&after));
    }

    #[cfg(unix)]
    #[test]
    fn a_link_is_printed_as_the_link_even_when_its_target_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.txt");
        let link = dir.path().join("link");
        write(&target, "content");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let printed = read(&link);
        assert_eq!(printed.kind, Kind::Link);
        assert!(!printed.same_object(&read(&target)));
        fs::remove_file(&target).unwrap();
        assert!(printed.unchanged(&read(&link)));
    }

    #[cfg(unix)]
    #[test]
    fn a_retargeted_inner_link_changes_the_tree() {
        let (_dir, root) = sample();
        let link = root.join("link");
        std::os::unix::fs::symlink("b.txt", &link).unwrap();
        let key = RandomState::new();
        let before = tree(&root, &key);
        assert!(before.unchanged(&tree(&root, &key)));
        fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink("a/x.txt", &link).unwrap();
        assert!(!before.unchanged(&tree(&root, &key)));
        assert!(!before.matches(&tree(&root, &key)));
    }

    #[test]
    fn two_reads_of_an_untouched_folder_are_unchanged() {
        let (_dir, root) = sample();
        let key = RandomState::new();
        let first = tree(&root, &key);
        let second = tree(&root, &key);
        assert!(first.unchanged(&second));
        assert!(first.matches(&second));
        assert_eq!(first.entries, 3);
        assert_eq!(first.bytes, 6);
    }

    #[test]
    fn a_folder_tree_notices_every_kind_of_inner_change() {
        type Mutation = fn(&Path);
        let cases: [(&str, Mutation); 7] = [
            ("added file", |root: &Path| {
                write(&root.join("added.txt"), "new")
            }),
            ("added empty folder", |root: &Path| {
                fs::create_dir(root.join("empty")).unwrap()
            }),
            ("removed file", |root: &Path| {
                fs::remove_file(root.join("b.txt")).unwrap()
            }),
            ("removed folder", |root: &Path| {
                fs::remove_dir_all(root.join("a")).unwrap()
            }),
            ("edited file", |root: &Path| {
                edit(&root.join("a/x.txt"), "one and more")
            }),
            ("touched file", |root: &Path| {
                set_modified(&root.join("a/x.txt"), then() + Duration::from_secs(60))
            }),
            ("renamed inner file", |root: &Path| {
                fs::rename(root.join("a/x.txt"), root.join("a/y.txt")).unwrap()
            }),
        ];
        for (name, mutate) in cases {
            let (_dir, root) = sample();
            let key = RandomState::new();
            let before = tree(&root, &key);
            assert!(before.unchanged(&tree(&root, &key)), "{name}");
            mutate(&root);
            assert!(!before.unchanged(&tree(&root, &key)), "{name}");
        }
    }

    #[test]
    fn a_renamed_inner_file_changes_the_shape_not_the_counts() {
        let (_dir, root) = sample();
        let key = RandomState::new();
        let before = tree(&root, &key);
        fs::rename(root.join("a/x.txt"), root.join("a/y.txt")).unwrap();
        let after = tree(&root, &key);
        assert_eq!(before.entries, after.entries);
        assert_eq!(before.bytes, after.bytes);
        assert!(!before.matches(&after));
    }

    #[test]
    fn an_inner_file_replaced_by_an_identical_one_is_not_unchanged() {
        let (_dir, root) = sample();
        let key = RandomState::new();
        let before = tree(&root, &key);
        replace(&root.join("a/x.txt"), "one");
        let after = tree(&root, &key);
        assert!(before.matches(&after));
        assert!(!before.unchanged(&after));
    }

    #[test]
    fn a_copy_of_a_tree_matches_but_is_not_the_same_tree() {
        let (_dir, root) = sample();
        let copy = root.with_file_name("copy");
        write(&copy.join("a/x.txt"), "one");
        write(&copy.join("b.txt"), "two");
        let key = RandomState::new();
        let original = tree(&root, &key);
        let copied = tree(&copy, &key);
        assert!(original.matches(&copied));
        assert!(!original.unchanged(&copied));
    }

    #[test]
    fn the_tree_of_a_file_follows_its_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        write(&path, "text");
        let key = RandomState::new();
        let before = tree(&path, &key);
        assert!(before.unchanged(&tree(&path, &key)));
        assert_eq!(before.entries, 0);
        assert_eq!(before.bytes, 4);
        edit(&path, "other text");
        assert!(!before.unchanged(&tree(&path, &key)));
    }

    #[test]
    fn litter_files_at_the_top_of_a_tree_are_not_part_of_it() {
        let (_dir, root) = sample();
        let key = RandomState::new();
        let before = tree(&root, &key);
        for litter in LITTER {
            write(&root.join(litter), "litter");
        }
        assert!(before.unchanged(&tree(&root, &key)));
    }

    #[test]
    fn litter_files_inside_a_subfolder_are_not_part_of_the_tree() {
        let (_dir, root) = sample();
        let key = RandomState::new();
        let before = tree(&root, &key);
        for litter in LITTER {
            write(&root.join("a").join(litter), "litter");
        }
        assert!(before.unchanged(&tree(&root, &key)));
    }
}
