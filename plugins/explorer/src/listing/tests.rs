use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use super::read::{run, CountClaim};
use super::*;

fn touch(folder: &Path, name: &str) {
    fs::write(folder.join(name), name).unwrap();
}

fn make_folder(folder: &Path, name: &str) {
    fs::create_dir(folder.join(name)).unwrap();
}

fn collect(folder: &Path, batch_size: usize, cancel: &CancelToken) -> Vec<ListingEvent> {
    let mut events = Vec::new();
    run(folder, Pass::WithCounts, batch_size, cancel, |event| {
        events.push(event);
        true
    });
    events
}

fn listing_of(folder: &Path) -> Listing {
    let mut listing = Listing::new(folder.to_path_buf());
    for event in collect(folder, 2, &CancelToken::new()) {
        listing.apply(event);
    }
    listing
}

fn items(count: u64) -> Option<ItemCount> {
    Some(ItemCount::Items {
        all: count,
        shown: count,
    })
}

fn names(listing: &Listing) -> Vec<&str> {
    listing
        .entries()
        .iter()
        .map(|entry| entry.display.as_str())
        .collect()
}

#[test]
fn sort_keys_put_folders_first_and_compare_digit_runs_by_value() {
    let mut names = vec![
        ("file10", false),
        ("File1", false),
        ("file2", false),
        ("file02", false),
        ("zeta", true),
        ("Alpha", true),
        ("alpha", false),
        ("file2a", false),
        ("file", false),
        ("10", false),
        ("9", false),
    ];
    names.sort_by_key(|(name, folder)| SortKey::new(OsStr::new(name), *folder));
    let sorted: Vec<&str> = names.iter().map(|(name, _)| *name).collect();
    assert_eq!(
        sorted,
        [
            "Alpha", "zeta", "9", "10", "alpha", "file", "File1", "file02", "file2", "file2a",
            "file10"
        ]
    );
}

#[test]
fn sort_keys_ignore_case_and_break_ties_by_the_raw_name() {
    let upper = SortKey::new(OsStr::new("README"), false);
    let lower = SortKey::new(OsStr::new("readme"), false);
    let other = SortKey::new(OsStr::new("Readme2"), false);
    assert!(upper < lower);
    assert!(lower < other);
    assert_ne!(upper, lower);
}

#[test]
fn sort_keys_fold_accents_into_their_plain_letters() {
    let mut names = ["zebra", "Éclair", "eagle", "école", "apple"];
    names.sort_by_key(|name| SortKey::new(OsStr::new(name), false));
    assert_eq!(names, ["apple", "eagle", "Éclair", "école", "zebra"]);
}

#[test]
fn a_read_folder_lists_folders_first_in_natural_order() {
    let temp = tempfile::tempdir().unwrap();
    for name in ["file10", "file2", "Zeta", "alpha.txt"] {
        touch(temp.path(), name);
    }
    for name in ["b_dir", "A_dir"] {
        make_folder(temp.path(), name);
    }

    let listing = listing_of(temp.path());

    assert_eq!(listing.state(), &ListingState::Done);
    assert_eq!(
        names(&listing),
        ["A_dir", "b_dir", "alpha.txt", "file2", "file10", "Zeta"]
    );
    let file = &listing.entries()[3];
    assert_eq!(file.kind, EntryKind::File);
    assert_eq!(file.size, Size::Bytes("file2".len() as u64));
    assert_eq!(file.path, temp.path().join("file2"));
    assert_eq!(listing.entries()[0].kind, EntryKind::Folder);
}

#[test]
fn folder_item_counts_arrive_after_the_listing_is_done() {
    let temp = tempfile::tempdir().unwrap();
    make_folder(temp.path(), "three");
    make_folder(temp.path(), "empty");
    for name in ["a", "b", "c"] {
        touch(&temp.path().join("three"), name);
    }

    let events = collect(temp.path(), 8, &CancelToken::new());
    let done = events
        .iter()
        .position(|event| *event == ListingEvent::Done)
        .expect("done");
    let first_count = events
        .iter()
        .position(|event| matches!(event, ListingEvent::Counts(_)))
        .expect("counts");
    assert!(done < first_count);

    let mut listing = Listing::new(temp.path().to_path_buf());
    for event in events {
        listing.apply(event);
    }
    assert_eq!(listing.item_count(&temp.path().join("three")), items(3));
    assert_eq!(listing.item_count(&temp.path().join("empty")), items(0));
}

#[test]
fn folder_counts_say_how_many_items_are_hidden() {
    let temp = tempfile::tempdir().unwrap();
    make_folder(temp.path(), "mixed");
    for name in [".dot", "plain", "other"] {
        touch(&temp.path().join("mixed"), name);
    }

    let listing = listing_of(temp.path());

    assert_eq!(
        listing.item_count(&temp.path().join("mixed")),
        Some(ItemCount::Items { all: 3, shown: 2 })
    );
}

#[cfg(unix)]
#[test]
fn a_link_to_a_folder_is_counted_like_the_folder() {
    let temp = tempfile::tempdir().unwrap();
    make_folder(temp.path(), "real");
    touch(&temp.path().join("real"), "one");
    touch(&temp.path().join("real"), "two");
    std::os::unix::fs::symlink(temp.path().join("real"), temp.path().join("link")).unwrap();

    let listing = listing_of(temp.path());

    assert_eq!(listing.item_count(&temp.path().join("link")), items(2));
}

#[test]
fn a_recount_reads_only_the_named_folders() {
    let temp = tempfile::tempdir().unwrap();
    let named = temp.path().join("named");
    let other = temp.path().join("other");
    make_folder(temp.path(), "named");
    make_folder(temp.path(), "other");
    touch(&named, "one");

    let read = recount(temp.path().to_path_buf(), vec![named.clone()], || {});
    let mut listing = Listing::new(temp.path().to_path_buf());
    while let Ok(event) = read.events().recv() {
        listing.apply(event);
    }

    assert_eq!(listing.item_count(&named), items(1));
    assert_eq!(listing.item_count(&other), None);
}

#[test]
fn a_count_read_earlier_never_replaces_one_read_later() {
    let temp = tempfile::tempdir().unwrap();
    let folder = temp.path().join("sub");
    let earlier = Instant::now();
    let later = earlier + Duration::from_millis(5);
    let count = |all, read_at| {
        ListingEvent::Counts(vec![FolderCount {
            path: folder.clone(),
            count: ItemCount::Items { all, shown: all },
            read_at,
        }])
    };
    let mut listing = Listing::new(temp.path().to_path_buf());

    listing.apply(count(2, later));
    listing.apply(count(1, earlier));
    assert_eq!(listing.item_count(&folder), items(2));

    listing.apply(count(3, later + Duration::from_millis(5)));
    assert_eq!(listing.item_count(&folder), items(3));
}

#[test]
fn names_with_control_characters_show_a_replacement_mark() {
    assert_eq!(
        display_name(OsStr::new("new\nline\t.txt")),
        "new\u{FFFD}line\u{FFFD}.txt"
    );
    assert_eq!(display_name(OsStr::new("éclair.txt")), "éclair.txt");
}

#[test]
fn a_failure_note_marks_control_characters_in_the_folder() {
    let error = ListingError {
        folder: PathBuf::from("/tmp/new\nline"),
        reason: FailureReason::Missing,
    };
    assert_eq!(error.to_string(), "/tmp/new\u{FFFD}line does not exist");
}

#[test]
fn direction_overrides_and_zero_width_spaces_show_a_mark_but_joiners_stay() {
    assert_eq!(
        display_name(OsStr::new("invoice\u{202E}fdp.sh")),
        "invoice\u{FFFD}fdp.sh"
    );
    assert_eq!(
        display_name(OsStr::new("same\u{200B}name\u{2066}.txt")),
        "same\u{FFFD}name\u{FFFD}.txt"
    );
    let joined = "\u{1F468}\u{200D}\u{1F4BB} notes.txt";
    assert_eq!(display_name(OsStr::new(joined)), joined);
}

#[test]
fn cancelling_stops_the_read_between_entries() {
    let temp = tempfile::tempdir().unwrap();
    for index in 0..10 {
        touch(temp.path(), &format!("file{index}"));
    }
    let cancel = CancelToken::new();
    let mut events = Vec::new();
    run(temp.path(), Pass::WithCounts, 2, &cancel, |event| {
        events.push(event);
        cancel.cancel();
        true
    });

    assert_eq!(events.len(), 1);
    assert!(matches!(&events[0], ListingEvent::Entries(batch) if batch.len() == 2));
}

#[test]
fn a_cancelled_token_reads_nothing() {
    let temp = tempfile::tempdir().unwrap();
    touch(temp.path(), "file");
    let cancel = CancelToken::new();
    cancel.cancel();

    assert!(collect(temp.path(), 2, &cancel).is_empty());
}

#[test]
fn cancelling_after_done_stops_the_count_pass() {
    let temp = tempfile::tempdir().unwrap();
    for name in ["a", "b", "c"] {
        make_folder(temp.path(), name);
    }
    let cancel = CancelToken::new();
    let mut events = Vec::new();
    run(temp.path(), Pass::WithCounts, 1, &cancel, |event| {
        if event == ListingEvent::Done {
            cancel.cancel();
        }
        events.push(event);
        true
    });

    assert_eq!(events.last(), Some(&ListingEvent::Done));
    assert!(!events
        .iter()
        .any(|event| matches!(event, ListingEvent::Counts(_))));
}

#[test]
fn a_names_only_pass_counts_nothing() {
    let temp = tempfile::tempdir().unwrap();
    make_folder(temp.path(), "a");
    touch(&temp.path().join("a"), "file");
    let mut events = Vec::new();
    run(
        temp.path(),
        Pass::NamesOnly,
        2,
        &CancelToken::new(),
        |event| {
            events.push(event);
            true
        },
    );

    assert_eq!(events.last(), Some(&ListingEvent::Done));
    assert!(!events
        .iter()
        .any(|event| matches!(event, ListingEvent::Counts(_))));
}

#[test]
fn a_closed_receiver_stops_the_read() {
    let temp = tempfile::tempdir().unwrap();
    for index in 0..10 {
        touch(temp.path(), &format!("file{index}"));
    }
    let mut calls = 0;
    run(
        temp.path(),
        Pass::WithCounts,
        2,
        &CancelToken::new(),
        |_| {
            calls += 1;
            false
        },
    );

    assert_eq!(calls, 1);
}

#[test]
fn read_errors_are_typed_and_name_the_folder() {
    let temp = tempfile::tempdir().unwrap();
    touch(temp.path(), "plain");
    let missing = temp.path().join("missing");
    let plain = temp.path().join("plain");
    let cases = [
        (missing.clone(), FailureReason::Missing),
        (plain.clone(), FailureReason::NotAFolder),
    ];
    for (folder, reason) in cases {
        let events = collect(&folder, 2, &CancelToken::new());
        assert_eq!(
            events,
            [ListingEvent::Failed(ListingError {
                folder: folder.clone(),
                reason
            })],
            "folder: {}",
            folder.display()
        );
    }
}

#[test]
fn io_errors_map_to_failure_reasons() {
    let folder = PathBuf::from("/data/ai/ollama/blobs");
    let cases = [
        (io::ErrorKind::PermissionDenied, FailureReason::Permission),
        (io::ErrorKind::NotFound, FailureReason::Missing),
        (io::ErrorKind::NotADirectory, FailureReason::NotAFolder),
    ];
    for (kind, reason) in cases {
        let error = ListingError::from_io(folder.clone(), &io::Error::from(kind));
        assert_eq!(error.reason, reason, "kind: {kind:?}");
    }
    let other = ListingError::from_io(folder.clone(), &io::Error::other("disk on fire"));
    assert_eq!(other.reason, FailureReason::Other("disk on fire".into()));
}

#[test]
fn errors_say_why_the_folder_cannot_be_read() {
    let folder = PathBuf::from("/data/ai/ollama/blobs");
    let cases = [
        (
            FailureReason::Permission,
            "No permission to read /data/ai/ollama/blobs",
        ),
        (
            FailureReason::Missing,
            "/data/ai/ollama/blobs does not exist",
        ),
        (
            FailureReason::NotAFolder,
            "/data/ai/ollama/blobs is not a folder",
        ),
        (
            FailureReason::Other("busy".into()),
            "Could not read /data/ai/ollama/blobs: busy",
        ),
    ];
    for (reason, message) in cases {
        let error = ListingError {
            folder: folder.clone(),
            reason,
        };
        assert_eq!(error.to_string(), message);
    }
}

#[test]
fn a_reason_reads_on_its_own_beside_the_typed_path() {
    let cases = [
        (FailureReason::Permission, "No permission"),
        (FailureReason::Missing, "Does not exist"),
        (FailureReason::NotAFolder, "Not a folder"),
        (FailureReason::Other("busy".into()), "Could not read"),
    ];
    for (reason, message) in cases {
        assert_eq!(reason.to_string(), message);
    }
}

fn run_in_background(folder: &Path, cancel: &CancelToken) -> mpsc::Receiver<ListingEvent> {
    let (sender, events) = mpsc::channel();
    let folder = folder.to_path_buf();
    let cancel = cancel.clone();
    thread::spawn(move || {
        run(&folder, Pass::WithCounts, 2, &cancel, |event| {
            sender.send(event).is_ok()
        });
    });
    events
}

fn apply_until(
    listing: &mut Listing,
    events: &mpsc::Receiver<ListingEvent>,
    done: impl Fn(&Listing) -> bool,
) {
    while !done(listing) {
        listing.apply(events.recv_timeout(Duration::from_secs(10)).unwrap());
    }
}

#[test]
fn a_subfolder_whose_count_is_held_elsewhere_is_counted_once_it_frees() {
    let temp = tempfile::tempdir().unwrap();
    make_folder(temp.path(), "stuck");
    make_folder(temp.path(), "quick");
    let stuck = temp.path().join("stuck");
    let quick = temp.path().join("quick");
    let claim = CountClaim::take(&stuck).unwrap();
    assert!(CountClaim::take(&stuck).is_none());

    let events = run_in_background(temp.path(), &CancelToken::new());
    let mut listing = Listing::new(temp.path().to_path_buf());
    apply_until(&mut listing, &events, |listing| {
        listing.item_count(&quick).is_some()
    });
    assert_eq!(listing.item_count(&quick), items(0));
    assert_eq!(listing.item_count(&stuck), None);

    drop(claim);
    apply_until(&mut listing, &events, |listing| {
        listing.item_count(&stuck).is_some()
    });
    assert_eq!(listing.item_count(&stuck), items(0));
}

#[test]
fn a_recount_counts_a_subfolder_held_elsewhere_once_it_frees() {
    let temp = tempfile::tempdir().unwrap();
    make_folder(temp.path(), "held");
    make_folder(temp.path(), "free");
    let held = temp.path().join("held");
    let free = temp.path().join("free");
    let claim = CountClaim::take(&held).unwrap();

    let read = recount(
        temp.path().to_path_buf(),
        vec![held.clone(), free.clone()],
        || {},
    );
    let counted = || match read.events().recv_timeout(Duration::from_secs(10)) {
        Ok(ListingEvent::Counts(counts)) => counts
            .into_iter()
            .map(|count| count.path)
            .collect::<Vec<_>>(),
        other => panic!("expected counts, got {other:?}"),
    };
    assert_eq!(counted(), [free]);
    drop(claim);
    assert_eq!(counted(), [held]);
    assert!(read.events().recv_timeout(Duration::from_secs(10)).is_err());
}

#[test]
fn a_count_pass_waiting_on_a_held_subfolder_ends_when_cancelled() {
    let temp = tempfile::tempdir().unwrap();
    make_folder(temp.path(), "stuck");
    let _claim = CountClaim::take(&temp.path().join("stuck")).unwrap();
    let cancel = CancelToken::new();
    let events = run_in_background(temp.path(), &cancel);
    let mut listing = Listing::new(temp.path().to_path_buf());
    apply_until(&mut listing, &events, |listing| {
        *listing.state() == ListingState::Done
    });

    cancel.cancel();
    while events.recv_timeout(Duration::from_secs(10)).is_ok() {}
    assert!(matches!(
        events.try_recv(),
        Err(mpsc::TryRecvError::Disconnected)
    ));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn pipes_sockets_devices_and_broken_links_have_their_own_kind_and_no_size() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::symlink;
    use std::os::unix::net::UnixListener;

    let temp = tempfile::tempdir().unwrap();
    let fifo = CString::new(temp.path().join("fifo").as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o644) }, 0);
    let _socket = UnixListener::bind(temp.path().join("socket")).unwrap();
    symlink(temp.path().join("nowhere"), temp.path().join("broken")).unwrap();
    symlink(temp.path().join("looped"), temp.path().join("looped")).unwrap();
    symlink("/dev/null", temp.path().join("null")).unwrap();
    symlink(temp.path().join("fifo/inside"), temp.path().join("through")).unwrap();

    let listing = listing_of(temp.path());
    let kinds: Vec<(&str, EntryKind, Size)> = listing
        .entries()
        .iter()
        .map(|entry| (entry.display.as_str(), entry.kind, entry.size))
        .collect();
    assert_eq!(
        kinds,
        [
            ("broken", EntryKind::BrokenLink, Size::Blank),
            ("fifo", EntryKind::Pipe, Size::Blank),
            ("looped", EntryKind::BrokenLink, Size::Blank),
            ("null", EntryKind::Link, Size::Blank),
            ("socket", EntryKind::Socket, Size::Blank),
            ("through", EntryKind::BrokenLink, Size::Blank),
        ]
    );
    let devices = listing_of(Path::new("/dev"));
    let null = devices
        .entries()
        .iter()
        .find(|entry| entry.display == "null")
        .unwrap();
    assert_eq!((null.kind, null.size), (EntryKind::Device, Size::Blank));
}

#[cfg(target_os = "linux")]
#[test]
fn files_on_proc_list_without_a_size() {
    let listing = listing_of(Path::new("/proc/self"));
    let status = listing
        .entries()
        .iter()
        .find(|entry| entry.display == "status")
        .unwrap();
    assert_eq!((status.kind, status.size), (EntryKind::File, Size::Blank));
}

#[cfg(unix)]
#[test]
fn entries_of_a_folder_that_cannot_be_searched_are_listed_without_a_size() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let blind = temp.path().join("blind");
    make_folder(temp.path(), "blind");
    touch(&blind, "file");
    make_folder(&blind, "sub");
    fs::set_permissions(&blind, fs::Permissions::from_mode(0o644)).unwrap();
    let readable_anyway = fs::metadata(blind.join("file")).is_ok();
    let parent = listing_of(temp.path());
    let listing = listing_of(&blind);
    fs::set_permissions(&blind, fs::Permissions::from_mode(0o755)).unwrap();
    if readable_anyway {
        return;
    }

    assert_eq!(parent.item_count(&blind), items(2));
    assert_eq!(names(&listing), ["sub", "file"]);
    let sub = &listing.entries()[0];
    assert_eq!(
        (sub.kind, sub.size, sub.opens_as_folder),
        (EntryKind::Folder, Size::Unreadable, true)
    );
    assert_eq!(
        listing.item_count(&blind.join("sub")),
        Some(ItemCount::Unreadable)
    );
    let file = &listing.entries()[1];
    assert_eq!(
        (file.kind, file.size, file.opens_as_folder),
        (EntryKind::File, Size::Unreadable, false)
    );
}

#[cfg(unix)]
#[test]
fn an_unreadable_folder_fails_with_permission() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let locked = temp.path().join("locked");
    make_folder(temp.path(), "locked");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let readable_anyway = fs::read_dir(&locked).is_ok();
    let events = collect(&locked, 2, &CancelToken::new());
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    if readable_anyway {
        return;
    }

    assert_eq!(
        events,
        [ListingEvent::Failed(ListingError {
            folder: locked,
            reason: FailureReason::Permission
        })]
    );
}

#[test]
fn a_listing_never_reports_empty_while_loading() {
    let temp = tempfile::tempdir().unwrap();
    let mut listing = Listing::new(temp.path().to_path_buf());

    assert_eq!(listing.state(), &ListingState::Loading);
    assert!(!listing.shows_empty(true));

    listing.apply(ListingEvent::Done);
    assert!(listing.shows_empty(true));
}

#[test]
fn a_failed_listing_is_not_empty() {
    let mut listing = Listing::new(PathBuf::from("/missing"));
    listing.apply(ListingEvent::Failed(ListingError {
        folder: PathBuf::from("/missing"),
        reason: FailureReason::Missing,
    }));

    assert!(matches!(listing.state(), ListingState::Failed(_)));
    assert!(!listing.shows_empty(true));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn dot_names_are_hidden_and_filtered_on_request() {
    let temp = tempfile::tempdir().unwrap();
    touch(temp.path(), ".secret");
    touch(temp.path(), "shown");

    let listing = listing_of(temp.path());

    assert_eq!(names(&listing), [".secret", "shown"]);
    assert!(listing.entries()[0].hidden);
    assert!(!listing.entries()[1].hidden);
    let visible: Vec<&str> = listing
        .visible(false)
        .iter()
        .map(|entry| entry.display.as_str())
        .collect();
    assert_eq!(visible, ["shown"]);
    assert!(!listing.shows_empty(false));
}

#[test]
fn start_streams_the_listing_from_a_background_thread() {
    let temp = tempfile::tempdir().unwrap();
    make_folder(temp.path(), "sub");
    touch(&temp.path().join("sub"), "inner");
    touch(temp.path(), "file");

    let wakes = Arc::new(AtomicUsize::new(0));
    let woken = Arc::clone(&wakes);
    let read = start(temp.path().to_path_buf(), Pass::WithCounts, move || {
        woken.fetch_add(1, Ordering::Relaxed);
    });
    assert_eq!(read.folder(), temp.path());
    let mut listing = Listing::new(temp.path().to_path_buf());
    let mut events = 0;
    while let Ok(event) = read.events().recv() {
        listing.apply(event);
        events += 1;
    }

    assert!(wakes.load(Ordering::Relaxed) >= events);
    assert_eq!(listing.state(), &ListingState::Done);
    assert_eq!(names(&listing), ["sub", "file"]);
    assert_eq!(listing.item_count(&temp.path().join("sub")), items(1));
}

fn kind_by_extension(entry: &Entry) -> &'static str {
    match entry.path.extension().and_then(OsStr::to_str) {
        Some("png") => "Image",
        Some("md") => "Markdown",
        _ => "Text",
    }
}

fn arranged(listing: &Listing, order: SortOrder) -> Vec<&str> {
    let rows = arrange(
        listing.entries(),
        listing.visible_indices(true),
        order,
        kind_by_extension,
    );
    rows.into_iter()
        .map(|index| listing.entries()[index].display.as_str())
        .collect()
}

fn sortable_folder() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    make_folder(temp.path(), "dir_a");
    make_folder(temp.path(), "dir_b");
    let files = [
        ("a.txt", 5, 40),
        ("b.md", 1, 10),
        ("c.png", 3, 30),
        ("d.txt", 3, 20),
    ];
    for (name, bytes, seconds) in files {
        let file = fs::File::create(temp.path().join(name)).unwrap();
        file.set_len(bytes).unwrap();
        file.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(seconds))
            .unwrap();
    }
    temp
}

#[test]
fn every_sort_keeps_folders_first_and_orders_files_by_its_column() {
    let temp = sortable_folder();
    let listing = listing_of(temp.path());
    let cases = [
        (SortBy::Name, false, ["a.txt", "b.md", "c.png", "d.txt"]),
        (SortBy::Name, true, ["d.txt", "c.png", "b.md", "a.txt"]),
        (SortBy::Size, false, ["b.md", "c.png", "d.txt", "a.txt"]),
        (SortBy::Size, true, ["a.txt", "c.png", "d.txt", "b.md"]),
        (SortBy::Modified, false, ["b.md", "d.txt", "c.png", "a.txt"]),
        (SortBy::Modified, true, ["a.txt", "c.png", "d.txt", "b.md"]),
        (SortBy::Kind, false, ["c.png", "b.md", "a.txt", "d.txt"]),
        (SortBy::Kind, true, ["a.txt", "d.txt", "b.md", "c.png"]),
    ];
    for (by, descending, files) in cases {
        let shown = arranged(&listing, SortOrder::new(by, descending));
        let (folders, rest) = shown.split_at(2);
        let mut folders = folders.to_vec();
        folders.sort();
        assert_eq!(folders, ["dir_a", "dir_b"], "{by:?} {descending}");
        assert_eq!(rest, files, "{by:?} {descending}");
    }
}

#[test]
fn folders_keep_name_order_under_a_size_sort_and_follow_the_direction_under_name() {
    let temp = sortable_folder();
    let listing = listing_of(temp.path());
    let folders = |order| arranged(&listing, order)[..2].to_vec();
    assert_eq!(
        folders(SortOrder::new(SortBy::Size, true)),
        ["dir_a", "dir_b"]
    );
    assert_eq!(
        folders(SortOrder::new(SortBy::Name, true)),
        ["dir_b", "dir_a"]
    );
}

#[test]
fn choosing_the_active_sort_again_flips_it_and_another_starts_ascending() {
    let by_size = SortOrder::default().chosen(SortBy::Size);
    assert_eq!(by_size, SortOrder::new(SortBy::Size, false));
    let flipped = by_size.chosen(SortBy::Size);
    assert_eq!(flipped, SortOrder::new(SortBy::Size, true));
    assert_eq!(flipped.chosen(SortBy::Size), by_size);
    assert_eq!(
        flipped.chosen(SortBy::Kind),
        SortOrder::new(SortBy::Kind, false)
    );
    assert_eq!(
        SortOrder::default().chosen(SortBy::Name),
        SortOrder::new(SortBy::Name, true)
    );
}

#[test]
fn a_hundred_thousand_entries_arrange_in_one_pass_per_column() {
    let entries: Vec<Entry> = (0..100_000u64)
        .map(|index| Entry {
            name: format!("file{index}").into(),
            display: format!("file{index}"),
            path: PathBuf::from(format!("/f/file{index}")),
            id: None,
            kind: EntryKind::File,
            size: Size::Bytes(index * 7919 % 10_007),
            hidden: false,
            opens_as_folder: index % 50 == 0,
            executable: false,
            modified: Some(std::time::UNIX_EPOCH + Duration::from_secs(index * 31 % 9973)),
            sort_key: SortKey::new(OsStr::new(&format!("file{index}")), index % 50 == 0),
        })
        .collect();
    let started = Instant::now();
    for by in SortBy::MENU {
        let kind = |entry: &Entry| {
            if entry.size == Size::Bytes(0) {
                "Text"
            } else {
                "Image"
            }
        };
        let rows = arrange(
            &entries,
            (0..entries.len()).collect(),
            SortOrder::new(by, true),
            kind,
        );
        assert_eq!(rows.len(), entries.len());
        let folders = rows
            .iter()
            .take_while(|&&index| entries[index].opens_as_folder)
            .count();
        assert_eq!(folders, 2_000, "{by:?}");
    }
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}
