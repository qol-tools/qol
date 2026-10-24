use std::fs::{self, Metadata};
use std::io;
use std::path::Path;
use std::time::{Duration, SystemTime};

use super::copy::same_link;
use super::{platform, Clash, CopyOnly, Resolve, Room, Stamp, Survey, Transfer, Verb};

const MTIME_SLACK: Duration = Duration::from_secs(2);

#[derive(Default)]
struct Tally {
    clashes: Vec<Clash>,
    same_inside: usize,
    bytes: u64,
}

pub fn survey(transfer: Transfer) -> io::Result<Survey> {
    if !fs::metadata(&transfer.into)?.is_dir() {
        return Err(invalid("the target is not a folder"));
    }
    let into = fs::canonicalize(&transfer.into)?;
    let mut tally = Tally::default();
    let mut copy_only = Vec::new();
    let mut crosses_volume = false;
    for source in &transfer.sources {
        let meta = fs::symlink_metadata(source)?;
        let name = source
            .file_name()
            .ok_or_else(|| invalid("that item has no name"))?;
        let parent = fs::canonicalize(source.parent().unwrap_or(source))?;
        if meta.is_dir() && into.starts_with(parent.join(name)) {
            return Err(invalid("a folder cannot go inside itself"));
        }
        crosses_volume |= crosses(source, &transfer.into);
        if transfer.verb == Verb::Move {
            if let Err(reason) = platform::removable(source) {
                copy_only.push(CopyOnly {
                    path: source.clone(),
                    reason,
                });
            }
        }
        if parent == into {
            if transfer.verb == Verb::Copy {
                tally.bytes += tree_bytes(source);
            }
            continue;
        }
        let existing = transfer.into.join(name);
        match fs::symlink_metadata(&existing) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                tally.bytes += tree_bytes(source);
            }
            Err(error) => return Err(error),
            Ok(here) if meta.is_dir() && here.is_dir() => merge(source, &existing, &mut tally)?,
            Ok(here) => top_level(source, &existing, &meta, &here, &mut tally),
        }
    }
    let need = if transfer.verb == Verb::Copy || crosses_volume {
        tally.bytes
    } else {
        0
    };
    let room = match platform::free_bytes(&transfer.into) {
        Ok(free) if need > free => Room::Short { need, free },
        _ => Room::Enough,
    };
    Ok(Survey {
        transfer,
        clashes: tally.clashes,
        same_inside: tally.same_inside,
        bytes: tally.bytes,
        room,
        copy_only,
        crosses_volume,
    })
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

pub fn crosses(source: &Path, into: &Path) -> bool {
    !same_volume(source.parent().unwrap_or(source), into)
}

fn same_volume(a: &Path, b: &Path) -> bool {
    let volume = |path: &Path| {
        let nearest = path.ancestors().find(|ancestor| ancestor.exists())?;
        platform::volume_of(nearest).ok()
    };
    match (volume(a), volume(b)) {
        (Some(left), Some(right)) => left == right,
        _ => a.components().next() == b.components().next(),
    }
}

fn top_level(source: &Path, existing: &Path, meta: &Metadata, here: &Metadata, tally: &mut Tally) {
    let incoming = stamp(source, meta);
    let stored = stamp(existing, here);
    let mismatched = incoming.folder != stored.folder;
    let same = is_same(&stored, &incoming) || same_link(source, existing);
    if !same {
        tally.bytes += incoming.bytes;
    }
    tally.clashes.push(Clash {
        source: source.to_path_buf(),
        existing: existing.to_path_buf(),
        here: stored,
        incoming,
        same,
        choice: if mismatched {
            Resolve::Keep
        } else {
            default_choice(&stored, &incoming, same)
        },
    });
}

fn merge(source: &Path, existing: &Path, tally: &mut Tally) -> io::Result<()> {
    let mut children = fs::read_dir(source)?.collect::<io::Result<Vec<_>>>()?;
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        let path = child.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        let target = existing.join(child.file_name());
        match fs::symlink_metadata(&target) {
            Err(_) => tally.bytes += tree_bytes(&path),
            Ok(here) if meta.is_dir() && here.is_dir() => merge(&path, &target, tally)?,
            Ok(here) => {
                let incoming = stamp(&path, &meta);
                let stored = stamp(&target, &here);
                if is_same(&stored, &incoming) || same_link(&path, &target) {
                    tally.same_inside += 1;
                    continue;
                }
                tally.bytes += incoming.bytes;
                tally.clashes.push(Clash {
                    source: path,
                    existing: target,
                    here: stored,
                    incoming,
                    same: false,
                    choice: if incoming.folder != stored.folder {
                        Resolve::Keep
                    } else {
                        default_choice(&stored, &incoming, false)
                    },
                });
            }
        }
    }
    Ok(())
}

fn stamp(path: &Path, meta: &Metadata) -> Stamp {
    let folder = meta.is_dir();
    Stamp {
        bytes: if folder { tree_bytes(path) } else { meta.len() },
        modified: meta.modified().ok(),
        folder,
    }
}

fn tree_bytes(path: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return 0;
    };
    if !meta.is_dir() {
        return meta.len();
    }
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| tree_bytes(&entry.path()))
        .sum()
}

fn is_same(here: &Stamp, incoming: &Stamp) -> bool {
    !here.folder
        && !incoming.folder
        && here.bytes == incoming.bytes
        && matches!(
            (here.modified, incoming.modified),
            (Some(a), Some(b)) if gap(a, b) <= MTIME_SLACK
        )
}

fn gap(a: SystemTime, b: SystemTime) -> Duration {
    a.duration_since(b)
        .or_else(|_| b.duration_since(a))
        .unwrap_or_default()
}

fn default_choice(here: &Stamp, incoming: &Stamp, same: bool) -> Resolve {
    let newer = matches!((here.modified, incoming.modified), (Some(h), Some(i)) if h > i);
    if same || newer {
        Resolve::Keep
    } else {
        Resolve::Replace
    }
}
