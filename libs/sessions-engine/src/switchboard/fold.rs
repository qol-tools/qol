use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Serialize;

use super::adopt::{partial_of, TRANSCRIPT_SUFFIX};
use super::resume::AGENT_PREFIX;

const SUBAGENTS: &str = "subagents";
const BLOCK: usize = 1 << 16;

#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub(super) struct Folded {
    pub(super) shared: usize,
    pub(super) freed_bytes: u64,
}

struct Transcript {
    path: PathBuf,
    len: u64,
    modified: SystemTime,
    device: u64,
    index: u128,
    links: u64,
}

impl Transcript {
    fn read(path: PathBuf) -> Option<Self> {
        let metadata = fs::symlink_metadata(&path).ok()?;
        if !metadata.file_type().is_file() || metadata.len() == 0 {
            return None;
        }
        let identity = qol_fs::file_identity(&path, &metadata)?;
        Some(Self {
            len: metadata.len(),
            modified: metadata.modified().ok()?,
            device: identity.device,
            index: identity.index,
            links: identity.links,
            path,
        })
    }

    fn is_file_of(&self, other: &Self) -> bool {
        self.device == other.device && self.index == other.index
    }
}

fn is_transcript(name: &str) -> bool {
    name.starts_with(AGENT_PREFIX) && name.ends_with(TRANSCRIPT_SUFFIX)
}

fn copies(conversations: &Path) -> BTreeMap<String, Vec<Transcript>> {
    let mut found: BTreeMap<String, Vec<Transcript>> = BTreeMap::new();
    let Ok(entries) = fs::read_dir(conversations) else {
        return found;
    };
    for conversation in entries.flatten() {
        let Ok(transcripts) = fs::read_dir(conversation.path().join(SUBAGENTS)) else {
            continue;
        };
        for transcript in transcripts.flatten() {
            let Some(name) = transcript.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !is_transcript(&name) {
                continue;
            }
            if let Some(copy) = Transcript::read(transcript.path()) {
                found.entry(name).or_default().push(copy);
            }
        }
    }
    found
}

fn filled(file: &mut File, block: &mut [u8]) -> io::Result<usize> {
    let mut read = 0;
    while read < block.len() {
        match file.read(&mut block[read..])? {
            0 => break,
            count => read += count,
        }
    }
    Ok(read)
}

fn starts_with(longer: &Path, shorter: &Path, len: u64) -> io::Result<bool> {
    let mut longer = File::open(longer)?;
    let mut shorter = File::open(shorter)?;
    let mut left = vec![0u8; BLOCK];
    let mut right = vec![0u8; BLOCK];
    let mut compared = 0u64;
    while compared < len {
        let want = usize::try_from(len - compared).map_or(BLOCK, |rest| rest.min(BLOCK));
        let had = filled(&mut shorter, &mut right[..want])?;
        if had == 0 || filled(&mut longer, &mut left[..had])? != had || left[..had] != right[..had]
        {
            return Ok(false);
        }
        compared += had as u64;
    }
    Ok(true)
}

fn fold_fork(mut copies: Vec<Transcript>, folded: &mut Folded) {
    copies.sort_by(|left, right| {
        right
            .modified
            .cmp(&left.modified)
            .then_with(|| right.len.cmp(&left.len))
    });
    let mut kept: Vec<Transcript> = Vec::new();
    for copy in copies {
        if kept.iter().any(|newer| newer.is_file_of(&copy)) {
            continue;
        }
        let holder = kept.iter().position(|newer| {
            newer.len >= copy.len && starts_with(&newer.path, &copy.path, copy.len).unwrap_or(false)
        });
        let Some(holder) = holder else {
            kept.push(copy);
            continue;
        };
        let newer = &kept[holder].path;
        if crate::file_link::link(newer, &copy.path, &partial_of(&copy.path)).is_ok() {
            folded.shared += 1;
            if copy.links == 1 {
                folded.freed_bytes += copy.len;
            }
        }
    }
}

pub(super) fn fold(conversations: &Path) -> Folded {
    let mut folded = Folded::default();
    for fork in copies(conversations).into_values() {
        if fork.len() > 1 {
            fold_fork(fork, &mut folded);
        }
    }
    folded
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::file_link::is_same_file;

    fn write(conversations: &Path, conversation: &str, id: &str, body: &str, age: u64) -> PathBuf {
        let subagents = conversations.join(conversation).join(SUBAGENTS);
        fs::create_dir_all(&subagents).unwrap();
        let path = subagents.join(format!("{AGENT_PREFIX}{id}{TRANSCRIPT_SUFFIX}"));
        fs::write(&path, body).unwrap();
        let modified = SystemTime::now() - Duration::from_secs(age);
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        path
    }

    #[test]
    fn older_copies_a_newer_one_starts_with_become_that_one_file() {
        let root = tempfile::tempdir().unwrap();
        let first = write(root.path(), "c1", "a1", "one\n", 300);
        let second = write(root.path(), "c2", "a1", "one\ntwo\n", 200);
        let newest = write(root.path(), "c3", "a1", "one\ntwo\nthree\n", 100);
        let other = write(root.path(), "c1", "b2", "other\n", 300);

        let folded = fold(root.path());

        assert_eq!(
            folded,
            Folded {
                shared: 2,
                freed_bytes: 4 + 8
            }
        );
        for older in [&first, &second] {
            assert!(is_same_file(older, &newest));
            assert_eq!(fs::read_to_string(older).unwrap(), "one\ntwo\nthree\n");
        }
        assert_eq!(fs::read_to_string(&other).unwrap(), "other\n");
        assert_eq!(
            fold(root.path()),
            Folded::default(),
            "a second pass has nothing to do"
        );
        let leftovers = fs::read_dir(root.path().join("c1").join(SUBAGENTS))
            .unwrap()
            .count();
        assert_eq!(leftovers, 2, "no partial file is left behind");
    }

    #[test]
    fn a_copy_that_differs_or_is_newer_keeps_its_own_file() {
        let root = tempfile::tempdir().unwrap();
        let cleared = write(root.path(), "c1", "a1", "before the clear\n", 300);
        let newest = write(root.path(), "c2", "a1", "after the clear\nmore\n", 100);
        let longer_but_older = write(root.path(), "c1", "b2", "one\ntwo\nthree\n", 300);
        let shorter_but_newer = write(root.path(), "c2", "b2", "one\ntwo\n", 100);

        assert_eq!(fold(root.path()), Folded::default());

        assert_eq!(fs::read_to_string(&cleared).unwrap(), "before the clear\n");
        assert_eq!(
            fs::read_to_string(&newest).unwrap(),
            "after the clear\nmore\n"
        );
        assert_eq!(
            fs::read_to_string(&longer_but_older).unwrap(),
            "one\ntwo\nthree\n"
        );
        assert_eq!(
            fs::read_to_string(&shorter_but_newer).unwrap(),
            "one\ntwo\n"
        );
    }

    #[test]
    fn only_fork_transcripts_in_a_conversation_folder_are_folded() {
        let root = tempfile::tempdir().unwrap();
        let newest = write(root.path(), "c2", "a1", "one\ntwo\n", 100);
        let subagents = root.path().join("c1").join(SUBAGENTS);
        fs::create_dir_all(&subagents).unwrap();
        let meta = subagents.join("agent-a1.meta.json");
        let partial = subagents.join("agent-a1.jsonl.a0.partial");
        let empty = subagents.join("agent-a1.jsonl");
        fs::write(&meta, "one\n").unwrap();
        fs::write(&partial, "one\n").unwrap();
        fs::write(&empty, "").unwrap();
        fs::write(root.path().join("c1.jsonl"), "one\n").unwrap();

        assert_eq!(fold(root.path()), Folded::default());
        assert_eq!(fold(&root.path().join("missing")), Folded::default());

        for kept in [&meta, &partial] {
            assert!(!is_same_file(kept, &newest));
        }
        assert_eq!(fs::read_to_string(&empty).unwrap(), "");
    }

    #[test]
    fn a_copy_shared_with_a_record_archive_frees_nothing_yet() {
        let root = tempfile::tempdir().unwrap();
        let older = write(root.path(), "c1", "a1", "one\n", 300);
        let newest = write(root.path(), "c2", "a1", "one\ntwo\n", 100);
        fs::hard_link(&older, root.path().join("part-1.jsonl")).unwrap();

        let folded = fold(root.path());

        assert_eq!(
            folded,
            Folded {
                shared: 1,
                freed_bytes: 0
            }
        );
        assert!(is_same_file(&older, &newest));
        assert_eq!(
            fs::read_to_string(root.path().join("part-1.jsonl")).unwrap(),
            "one\n"
        );
    }

    #[test]
    fn a_long_copy_is_compared_past_its_first_block() {
        let root = tempfile::tempdir().unwrap();
        let body = "row\n".repeat(BLOCK);
        let older = write(root.path(), "c1", "a1", &body, 300);
        let changed = format!("{}X{}", &body[..BLOCK + 1], &body[BLOCK + 2..]);
        let differs = write(root.path(), "c2", "a1", &format!("{changed}more\n"), 200);
        let newest = write(root.path(), "c3", "a1", &format!("{body}more\n"), 100);

        let folded = fold(root.path());

        assert_eq!(folded.shared, 1);
        assert!(is_same_file(&older, &newest));
        assert!(!is_same_file(&differs, &newest));
    }
}
