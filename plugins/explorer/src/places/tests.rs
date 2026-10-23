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
fn pin_paths_expand_against_home() {
    let home = Path::new("/home/me");
    let cases = [
        ("~", PathBuf::from("/home/me")),
        ("~/Projects/qol", home.join("Projects/qol")),
        ("/srv/music", PathBuf::from("/srv/music")),
        ("~other", PathBuf::from("~other")),
    ];
    for (pin_path, expected) in cases {
        assert_eq!(expand(pin_path, home), expected, "{pin_path}");
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
fn devices_start_with_this_computer() {
    let devices = devices();

    assert_eq!(devices[0].kind, DeviceKind::ThisComputer);
    assert_eq!(devices[0].label, "This computer");
    assert!(devices[1..]
        .iter()
        .all(|device| device.kind == DeviceKind::Drive && device.path != devices[0].path));
}
