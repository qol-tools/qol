#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::fs;
use std::path::Path;

use super::*;

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn kinds(places: &[Place]) -> Vec<PlaceKind> {
    places.iter().map(|place| place.kind).collect()
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
    assert_eq!(PlaceKind::Home.label(), "Home");
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

    let places = places_in(home.path());

    assert_eq!(
        kinds(&places),
        [PlaceKind::Home, PlaceKind::Documents, PlaceKind::Videos]
    );
    assert_eq!(places[0].path, home.path());
    assert_eq!(places[1].path, home.path().join("Documents"));
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

    let places = places_in(home.path());

    assert_eq!(
        kinds(&places),
        [PlaceKind::Home, PlaceKind::Music, PlaceKind::Pictures]
    );
    assert_eq!(places[1].path, home.path().join("Muzak"));
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
