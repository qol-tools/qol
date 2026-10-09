use std::path::PathBuf;

use super::*;

fn paths(items: &[&str]) -> Vec<PathBuf> {
    items.iter().map(PathBuf::from).collect()
}

fn names(items: &[&str]) -> Vec<String> {
    entries_from_shortcuts(&paths(items))
        .into_iter()
        .map(|entry| entry.name)
        .collect()
}

#[test]
fn name_is_the_file_stem_and_exec_is_the_shortcut() {
    let path = PathBuf::from("/menu/Programs/Visual Studio Code/Visual Studio Code.lnk");
    let entries = entries_from_shortcuts(std::slice::from_ref(&path));

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "Visual Studio Code");
    assert_eq!(entries[0].path, path);
    assert_eq!(entries[0].exec, vec![path.to_string_lossy().into_owned()]);
}

#[test]
fn only_lnk_files_become_entries() {
    assert_eq!(
        names(&["/m/Notes.url", "/m/Notes.exe", "/m/Notes", "/m/Notes.LNK"]),
        ["Notes"]
    );
}

#[test]
fn noise_shortcuts_are_dropped() {
    let noise = [
        "/m/Uninstall Steam.lnk",
        "/m/uninstall.lnk",
        "/m/Steam Uninstall.lnk",
        "/m/Blender Help.lnk",
        "/m/Blender Documentation.lnk",
        "/m/Blender Release Notes.lnk",
        "/m/Readme.lnk",
        "/m/Read Me First.lnk",
        "/m/Foo Readme.lnk",
    ];
    for item in noise {
        assert!(names(&[item]).is_empty(), "{item} should be filtered");
    }
}

#[test]
fn real_apps_with_noise_like_words_survive() {
    assert_eq!(
        names(&[
            "/m/Helpdesk Client.lnk",
            "/m/Notepad.lnk",
            "/m/Readable.lnk"
        ]),
        ["Helpdesk Client", "Notepad", "Readable"]
    );
}

#[test]
fn first_shortcut_wins_for_a_repeated_name_ignoring_case() {
    let entries = entries_from_shortcuts(&paths(&[
        "/user/Programs/Firefox.lnk",
        "/common/Programs/firefox.lnk",
        "/common/Programs/Chrome.lnk",
    ]));

    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>(),
        paths(&["/user/Programs/Firefox.lnk", "/common/Programs/Chrome.lnk"])
    );
}

#[test]
fn shortcut_detection_ignores_extension_case() {
    assert!(is_start_menu_shortcut(Path::new("/m/A.lnk")));
    assert!(is_start_menu_shortcut(Path::new("/m/A.LNK")));
    assert!(!is_start_menu_shortcut(Path::new("/m/A.desktop")));
}
