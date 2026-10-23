#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::fs;
use std::path::Path;

use super::*;
use crate::config::default_pins;

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn names(places: &[Place]) -> Vec<&str> {
    places.iter().map(|place| place.name.as_str()).collect()
}

#[test]
fn place_labels_name_each_folder() {
    let labels: Vec<&str> = PlaceKind::USER_FOLDERS
        .into_iter()
        .map(PlaceKind::label)
        .collect();
    assert_eq!(
        labels,
        [
            "Desktop",
            "Documents",
            "Downloads",
            "Music",
            "Pictures",
            "Videos"
        ]
    );
}

#[test]
fn the_user_folder_falls_back_to_the_named_folder_in_home() {
    let home = Path::new("/home/me");
    let known = [(PlaceKind::Music, PathBuf::from("/srv/music"))];
    assert_eq!(
        user_folder(PlaceKind::Music, &known, home),
        PathBuf::from("/srv/music")
    );
    assert_eq!(
        user_folder(PlaceKind::Videos, &known, home),
        home.join("Videos")
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn places_list_home_and_the_user_folders_that_exist() {
    let home = tempfile::tempdir().unwrap();
    for folder in ["Videos", "Documents"] {
        fs::create_dir(home.path().join(folder)).unwrap();
    }

    let places = places_in(home.path(), &default_pins());

    assert_eq!(names(&places), ["Home", "Documents", "Videos"]);
    assert_eq!(places[0].path, home.path());
    assert!(places[0].home);
    assert!(!places[1].home);
    assert!(places.iter().all(|place| place.connected));
    assert_eq!(places[1].path, home.path().join("Documents"));
    assert_eq!(
        places.iter().map(|place| place.pin).collect::<Vec<_>>(),
        [0, 2, 6]
    );
}

#[cfg(target_os = "linux")]
#[test]
fn places_follow_user_dirs_and_skip_folders_set_to_home() {
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join(".config");
    fs::create_dir(&config).unwrap();
    fs::write(
        config.join("user-dirs.dirs"),
        "XDG_MUSIC_DIR=\"$HOME/Muzak\"\nXDG_DESKTOP_DIR=\"$HOME/\"\n",
    )
    .unwrap();
    for folder in ["Muzak", "Music", "Desktop", "Pictures"] {
        fs::create_dir(home.path().join(folder)).unwrap();
    }

    let places = places_in(home.path(), &default_pins());

    assert_eq!(names(&places), ["Home", "Music", "Pictures"]);
    assert_eq!(places[1].path, home.path().join("Muzak"));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn a_renamed_usual_pin_keeps_its_name() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir(home.path().join("Documents")).unwrap();
    let pins = [Pin {
        name: "Papers".to_string(),
        path: "~/Documents".to_string(),
    }];

    let places = places_in(home.path(), &pins);

    assert_eq!(names(&places), ["Papers"]);
    assert_eq!(places[0].path, home.path().join("Documents"));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn a_missing_pin_outside_home_is_listed_as_not_connected() {
    let home = tempfile::tempdir().unwrap();
    let gone = home.path().join("gone");
    let pins = [Pin {
        name: "Backup".to_string(),
        path: gone.to_string_lossy().into_owned(),
    }];

    let places = places_in(home.path(), &pins);

    assert_eq!(names(&places), ["Backup"]);
    assert_eq!(places[0].path, gone);
    assert!(!places[0].connected);
    assert!(!places[0].home);
}

#[test]
fn pin_paths_resolve_against_home() {
    let home = Path::new("/home/me");
    let cases = [
        ("~", PathBuf::from("/home/me")),
        ("~/Projects/qol", home.join("Projects/qol")),
        ("~//Projects", home.join("Projects")),
        ("/srv/music", PathBuf::from("/srv/music")),
        ("projects/qol", home.join("projects/qol")),
        ("~other", home.join("~other")),
    ];
    for (pin_path, expected) in cases {
        let pin = pin("Pin", pin_path);
        let place = place(0, &pin, home, &[]).unwrap();
        assert_eq!(place.path, expected, "{pin_path}");
    }
}

#[test]
fn a_pin_with_a_blank_path_is_not_listed() {
    let home = Path::new("/home/me");
    for path in ["", "   ", "\t"] {
        assert!(
            place(0, &pin("Nowhere", path), home, &[]).is_none(),
            "{path:?}"
        );
    }
}

#[test]
fn a_pin_with_a_blank_name_is_listed_under_its_folder() {
    let home = Path::new("/home/me");
    let cases = [
        ("", "/srv/music", "music"),
        ("  ", "~/Projects/qol", "qol"),
        ("", "~", "Home"),
    ];
    for (name, path, expected) in cases {
        let place = place(0, &pin(name, path), home, &[]).unwrap();
        assert_eq!(place.name, expected, "{path:?}");
    }
}

fn pin(name: &str, path: &str) -> Pin {
    Pin {
        name: name.to_string(),
        path: path.to_string(),
    }
}

fn listed(pin: usize, path: impl Into<PathBuf>) -> Place {
    let path = path.into();
    Place {
        pin,
        name: String::new(),
        home: path == Path::new("/home/me"),
        path,
        connected: true,
    }
}

fn pin_names(pins: &[Pin]) -> Vec<&str> {
    pins.iter().map(|pin| pin.name.as_str()).collect()
}

fn hidden_between() -> (Vec<Pin>, Vec<Place>) {
    let pins = vec![
        pin("Home", "~"),
        pin("Desktop", "~/Desktop"),
        pin("Documents", "~/Documents"),
    ];
    let places = vec![listed(0, "/home/me"), listed(2, "/home/me/Documents")];
    (pins, places)
}

#[test]
fn unpin_removes_the_pin_behind_a_listed_place() {
    let (pins, places) = hidden_between();
    let cases: [(usize, Option<&[&str]>); 3] = [
        (0, Some(&["Desktop", "Documents"])),
        (1, Some(&["Home", "Desktop"])),
        (2, None),
    ];
    for (place, expected) in cases {
        let result = unpin(&pins, &places, place);
        assert_eq!(
            result.as_deref().map(pin_names).as_deref(),
            expected,
            "{place}"
        );
    }
    assert!(unpin(&pins[..1], &places, 1).is_none());
}

#[test]
fn move_pin_swaps_with_the_neighbouring_listed_place() {
    let (pins, places) = hidden_between();
    let cases: [(usize, bool, Option<&[&str]>); 5] = [
        (0, true, Some(&["Documents", "Desktop", "Home"])),
        (1, false, Some(&["Documents", "Desktop", "Home"])),
        (0, false, None),
        (1, true, None),
        (5, true, None),
    ];
    for (place, down, expected) in cases {
        let result = move_pin(&pins, &places, place, down);
        assert_eq!(
            result.as_deref().map(pin_names).as_deref(),
            expected,
            "{place} {down}"
        );
    }
    assert!(move_pin(&pins[..2], &places, 0, true).is_none());
}

#[test]
fn toggle_pin_removes_a_listed_folder_and_pins_an_unlisted_one() {
    let (pins, places) = hidden_between();
    let added = || pin("Music", "~/Music");
    let cases: [(&str, Option<&[&str]>); 3] = [
        ("/home/me/Documents", Some(&["Home", "Desktop"])),
        ("/home/me", Some(&["Desktop", "Documents"])),
        (
            "/home/me/Music",
            Some(&["Home", "Desktop", "Documents", "Music"]),
        ),
    ];
    for (folder, expected) in cases {
        let result = toggle_pin(&pins, &places, Path::new(folder), added);
        assert_eq!(
            result.as_deref().map(pin_names).as_deref(),
            expected,
            "{folder}"
        );
    }
    assert!(toggle_pin(&pins[..1], &places, Path::new("/home/me/Documents"), added).is_none());
}

#[test]
fn rename_pin_trims_and_ignores_blank_unchanged_or_missing() {
    let (pins, _) = hidden_between();
    let cases: [(usize, &str, Option<&[&str]>); 6] = [
        (2, "Papers", Some(&["Home", "Desktop", "Papers"])),
        (0, "  Base  ", Some(&["Base", "Desktop", "Documents"])),
        (0, "   ", None),
        (0, "", None),
        (1, " Desktop ", None),
        (3, "Nope", None),
    ];
    for (pin, name, expected) in cases {
        let result = rename_pin(&pins, pin, name);
        assert_eq!(
            result.as_deref().map(pin_names).as_deref(),
            expected,
            "{pin} {name:?}"
        );
    }
}

#[test]
fn a_pin_for_a_folder_names_and_places_it_by_its_home() {
    let home = Path::new("/home/me");
    let cases = [
        (home, "Home", "~"),
        (Path::new("/home/me/Projects/qol"), "qol", "~/Projects/qol"),
        (Path::new("/srv/music"), "music", "/srv/music"),
        (Path::new("/"), "/", "/"),
    ];
    for (folder, name, path) in cases {
        let pin = pin_for(folder, home);
        assert_eq!(
            (pin.name.as_str(), pin.path.as_str()),
            (name, path),
            "{folder:?}"
        );
    }
}

#[test]
fn a_usual_folder_that_user_dirs_moved_elsewhere_pins_by_absolute_path() {
    let home = Path::new("/home/me");
    let desktop = home.join("Desktop");
    let moved = [(PlaceKind::Desktop, home.join("Schreibtisch"))];

    let pin = pin_in(&desktop, home, &moved);

    assert_eq!(pin.name, "Desktop");
    assert_eq!(pin.path, desktop.to_string_lossy());
    let place = place(0, &pin, home, &moved).unwrap();
    assert_eq!(place.path, desktop);
}

#[test]
fn a_usual_folder_where_user_dirs_agrees_pins_as_the_usual_path() {
    let home = Path::new("/home/me");
    let desktop = home.join("Desktop");
    let same = [(PlaceKind::Desktop, desktop.clone())];
    let unlisted: [(PlaceKind, PathBuf); 0] = [];

    for known in [&same[..], &unlisted[..]] {
        let pin = pin_in(&desktop, home, known);
        assert_eq!(
            (pin.name.as_str(), pin.path.as_str()),
            ("Desktop", "~/Desktop")
        );
    }
}

#[test]
fn devices_start_with_this_computer() {
    let devices = devices();

    assert_eq!(devices[0].kind, DeviceKind::ThisComputer);
    assert_eq!(devices[0].label, "This computer");
    assert!(devices[1..]
        .iter()
        .all(|device| device.kind == DeviceKind::Drive && device.path != devices[0].path));
}
