use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use super::*;
use crate::capture::{Capture, Content};

const MINUTE: u64 = 60_000;
const DAY: u64 = 24 * 60 * MINUTE;
const NOW: u64 = 1_800_000_000_000;

pub(crate) struct TempStore(pub(crate) PathBuf);

impl TempStore {
    pub(crate) fn new(tag: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "qol-clipboard-{tag}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&path);
        Self(path)
    }

    pub(crate) fn history(&self) -> History {
        History::new(Store::at(&self.0))
    }
}

impl Drop for TempStore {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(crate) fn retention() -> Retention {
    Retention {
        max_unpinned: 200,
        max_age: Duration::from_millis(7 * DAY),
        secret_ttl: Duration::from_secs(30),
    }
}

const KEEP: SecretPolicy = SecretPolicy { keep: true };

pub(crate) fn text(value: &str, app: &str) -> Capture {
    Capture {
        content: Content::Text(value.to_string()),
        app: Some(app.to_string()),
        sensitive: false,
    }
}

fn secret(value: &str) -> Capture {
    Capture {
        sensitive: true,
        ..text(value, "KeePassXC")
    }
}

fn added(recorded: Recorded) -> Clip {
    match recorded {
        Recorded::Added(clip) => clip,
        other => panic!("expected Added, got {other:?}"),
    }
}

fn clip(id: &str, copied_at_ms: u64, pinned: bool) -> Clip {
    Clip {
        id: id.to_string(),
        kind: ClipKind::Text,
        text: Some(id.to_string()),
        image: None,
        bytes: id.len() as u64,
        app: None,
        copied_at_ms,
        pinned,
    }
}

fn ids(clips: &[Clip]) -> Vec<&str> {
    clips.iter().map(|clip| clip.id.as_str()).collect()
}

#[test]
fn store_round_trips_and_tolerates_missing_and_broken_lines() {
    let temp = TempStore::new("store");
    let store = Store::at(&temp.0);
    assert!(store.load().unwrap().is_empty());

    let clips = vec![clip("a", NOW, false), clip("b", NOW - MINUTE, true)];
    store.save(&clips).unwrap();
    assert_eq!(store.load().unwrap(), clips);

    let mut raw = std::fs::read_to_string(store.history_path()).unwrap();
    raw.push_str("{ not json\n\n");
    std::fs::write(store.history_path(), raw).unwrap();
    assert_eq!(store.load().unwrap(), clips);
}

#[test]
fn recording_the_same_text_again_refreshes_one_clip() {
    let temp = TempStore::new("refresh");
    let history = temp.history();
    let first = added(
        history
            .record(text("git status", "kitty"), NOW, retention(), KEEP)
            .unwrap(),
    );
    assert_eq!(first.kind, ClipKind::Command);
    assert!(first.id.starts_with("c_"));

    let again = history
        .record(text("git status", "Code"), NOW + MINUTE, retention(), KEEP)
        .unwrap();
    let refreshed = match again {
        Recorded::Refreshed(clip) => clip,
        other => panic!("expected Refreshed, got {other:?}"),
    };
    assert_eq!(refreshed.id, first.id);
    assert_eq!(refreshed.copied_at_ms, NOW + MINUTE);
    assert_eq!(refreshed.app.as_deref(), Some("Code"));
    assert_eq!(history.live(NOW + MINUTE, retention()).unwrap().len(), 1);
}

#[test]
fn empty_and_unreadable_captures_are_dropped() {
    let temp = TempStore::new("dropped");
    let history = temp.history();
    let cases = [
        text("   \n", "kitty"),
        Capture {
            content: Content::Png(b"not a png".to_vec()),
            app: None,
            sensitive: false,
        },
        Capture {
            content: Content::Files(Vec::new()),
            app: None,
            sensitive: false,
        },
    ];
    for capture in cases {
        let recorded = history.record(capture, NOW, retention(), KEEP).unwrap();
        assert!(matches!(recorded, Recorded::Dropped(_)), "{recorded:?}");
    }
    assert!(history.live(NOW, retention()).unwrap().is_empty());
}

#[test]
fn images_live_in_blobs_until_forgotten() {
    let temp = TempStore::new("image");
    let history = temp.history();
    let png = kind::png_header(856, 626);
    let capture = Capture {
        content: Content::Png(png.clone()),
        app: Some("Firefox".to_string()),
        sensitive: false,
    };
    let clip = added(history.record(capture, NOW, retention(), KEEP).unwrap());
    let image = clip.image.clone().expect("image metadata");
    assert_eq!((image.width, image.height), (856, 626));
    let blob = history.store().blob_path(&image.blob);
    assert_eq!(std::fs::read(&blob).unwrap(), png);

    history.forget(&clip.id).unwrap();
    assert!(!blob.exists());
    assert!(history.forget(&clip.id).is_err());
}

#[test]
fn retention_keeps_the_newest_unpinned_and_every_pinned_clip() {
    let limits = Retention {
        max_unpinned: 2,
        ..retention()
    };
    let clips = vec![
        clip("old-pinned", NOW - 30 * DAY, true),
        clip("u1", NOW - MINUTE, false),
        clip("u2", NOW - 2 * MINUTE, false),
        clip("u3", NOW - 3 * MINUTE, false),
        clip("u4", NOW, false),
    ];
    let (kept, dropped) = retain(clips, NOW, limits);
    assert_eq!(ids(&kept), vec!["u4", "u1", "old-pinned"]);
    assert_eq!(ids(&dropped), vec!["u2", "u3"]);
}

#[test]
fn retention_drops_unpinned_clips_past_the_age_limit() {
    let cases = [
        (NOW - 7 * DAY, true),
        (NOW - 7 * DAY - 1, false),
        (NOW - 8 * DAY, false),
    ];
    for (copied_at, kept) in cases {
        let (live, _) = retain(vec![clip("a", copied_at, false)], NOW, retention());
        assert_eq!(live.len() == 1, kept, "copied_at: {copied_at}");
        let (pinned, _) = retain(vec![clip("a", copied_at, true)], NOW, retention());
        assert_eq!(pinned.len(), 1, "pinned clips never age out");
    }
}

#[test]
fn retention_on_disk_prunes_blobs_too() {
    let temp = TempStore::new("prune");
    let history = temp.history();
    let capture = Capture {
        content: Content::Png(kind::png_header(1, 1)),
        app: None,
        sensitive: false,
    };
    let image = added(
        history
            .record(capture, NOW - 8 * DAY, retention(), KEEP)
            .unwrap(),
    );
    let blob = history
        .store()
        .blob_path(&image.image.as_ref().unwrap().blob);
    assert!(blob.exists());

    assert_eq!(history.prune(NOW, retention()).unwrap(), 1);
    assert!(!blob.exists());
    assert!(history.live(NOW, retention()).unwrap().is_empty());
}

#[test]
fn secrets_are_kept_masked_then_expire() {
    let temp = TempStore::new("secret");
    let history = temp.history();
    let stored = added(
        history
            .record(secret("hunter2"), NOW, retention(), KEEP)
            .unwrap(),
    );
    assert_eq!(stored.kind, ClipKind::Secret);
    assert!(stored.image.is_none());

    let cases = [(NOW + 29_999, 1), (NOW + 30_000, 0), (NOW + 60_000, 0)];
    for (now, count) in cases {
        assert_eq!(
            history.live(now, retention()).unwrap().len(),
            count,
            "now: {now}"
        );
    }
    assert!(history.store().load().unwrap().is_empty());
}

#[test]
fn secrets_are_dropped_at_capture_when_not_kept() {
    let temp = TempStore::new("secret-drop");
    let history = temp.history();
    let recorded = history
        .record(
            secret("hunter2"),
            NOW,
            retention(),
            SecretPolicy { keep: false },
        )
        .unwrap();
    assert!(matches!(recorded, Recorded::Dropped(_)));
    assert!(!history.store().history_path().exists());

    let image = Capture {
        content: Content::Png(kind::png_header(4, 4)),
        app: None,
        sensitive: true,
    };
    let recorded = history.record(image, NOW, retention(), KEEP).unwrap();
    assert!(matches!(recorded, Recorded::Dropped(_)));
}

#[test]
fn secrets_cannot_be_pinned_and_never_match_on_content() {
    let temp = TempStore::new("secret-search");
    let history = temp.history();
    let stored = added(
        history
            .record(secret("hunter2"), NOW, retention(), KEEP)
            .unwrap(),
    );
    assert!(history.toggle_pin(&stored.id, NOW, retention()).is_err());
    assert!(history
        .search("hunter2", 10, NOW, retention())
        .unwrap()
        .is_empty());
    assert_eq!(
        history
            .search("keepass", 10, NOW, retention())
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn search_matches_every_term_newest_first() {
    let temp = TempStore::new("search");
    let history = temp.history();
    for (offset, (value, app)) in [
        ("cargo test -p qol-clipboard", "kitty"),
        ("https://github.com/qol-tools", "Firefox"),
        ("cargo build --release", "kitty"),
    ]
    .into_iter()
    .enumerate()
    {
        history
            .record(text(value, app), NOW + offset as u64, retention(), KEEP)
            .unwrap();
    }
    let titles = |query: &str| -> Vec<String> {
        history
            .search(query, 10, NOW + 10, retention())
            .unwrap()
            .into_iter()
            .filter_map(|clip| clip.text)
            .collect()
    };
    assert_eq!(
        titles("cargo"),
        vec!["cargo build --release", "cargo test -p qol-clipboard"]
    );
    assert_eq!(titles("CARGO test"), vec!["cargo test -p qol-clipboard"]);
    assert_eq!(titles("firefox"), vec!["https://github.com/qol-tools"]);
    assert_eq!(titles("").len(), 3);
    assert_eq!(
        history.search("", 2, NOW + 10, retention()).unwrap().len(),
        2
    );
}

#[test]
fn pin_toggles_and_clear_spares_pinned_unless_asked() {
    let temp = TempStore::new("pin");
    let history = temp.history();
    let keep = added(
        history
            .record(text("keep me", "a"), NOW, retention(), KEEP)
            .unwrap(),
    );
    history
        .record(text("drop me", "a"), NOW + 1, retention(), KEEP)
        .unwrap();

    assert!(history.toggle_pin(&keep.id, NOW + 2, retention()).unwrap());
    assert!(!history.toggle_pin(&keep.id, NOW + 2, retention()).unwrap());
    assert!(history.toggle_pin(&keep.id, NOW + 2, retention()).unwrap());
    assert!(history
        .toggle_pin("c_missing", NOW + 2, retention())
        .is_err());

    assert_eq!(history.clear(false).unwrap(), 1);
    assert_eq!(
        ids(&history.live(NOW + 3, retention()).unwrap()),
        vec![keep.id.as_str()]
    );
    assert_eq!(history.clear(true).unwrap(), 1);
    assert!(history.live(NOW + 3, retention()).unwrap().is_empty());
}

#[test]
fn two_handles_writing_at_once_lose_no_clip() {
    let temp = TempStore::new("race");
    let writers: Vec<_> = ["daemon", "cli"]
        .into_iter()
        .map(|app| {
            let history = temp.history();
            std::thread::spawn(move || {
                for at in 0..40u64 {
                    history
                        .record(
                            text(&format!("{app} {at}"), app),
                            NOW + at,
                            retention(),
                            KEEP,
                        )
                        .unwrap();
                }
            })
        })
        .collect();
    for writer in writers {
        writer.join().unwrap();
    }
    assert_eq!(temp.history().store().load().unwrap().len(), 80);
}

#[test]
fn lines_this_version_cannot_read_survive_the_next_write() {
    let temp = TempStore::new("unread");
    let history = temp.history();
    let store = history.store();
    let foreign = r#"{"id":"c_future","kind":"hologram","copied_at_ms":1,"pinned":true}"#;
    store.save(&[clip("kept", NOW, true)]).unwrap();
    let path = store.history_path();
    let mut file = std::fs::read_to_string(&path).unwrap();
    file.push_str(foreign);
    file.push('\n');
    std::fs::write(&path, file).unwrap();

    history
        .record(text("new", "Firefox"), NOW + 1, retention(), KEEP)
        .unwrap();

    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.lines().any(|line| line == foreign), "{written}");
    let mut stored: Vec<_> = store.load().unwrap();
    stored.sort_by_key(|clip| clip.copied_at_ms);
    assert_eq!(stored.len(), 2);
    assert_eq!(stored[0].id, "kept");
}
