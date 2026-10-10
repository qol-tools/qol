use qol_platform::native::registry::{self, Key};

use super::catalog::{entry_from, Hive, KeyLocation, RawValues, UninstallEntry, View};

const SOURCES: [(Hive, View); 3] = [
    (Hive::LocalMachine, View::Native),
    (Hive::LocalMachine, View::Wow32),
    (Hive::CurrentUser, View::Native),
];

fn registry_hive(hive: Hive) -> registry::Hive {
    match hive {
        Hive::LocalMachine => registry::Hive::LocalMachine,
        Hive::CurrentUser => registry::Hive::CurrentUser,
    }
}

fn registry_view(view: View) -> registry::View {
    match view {
        View::Native => registry::View::Native,
        View::Wow32 => registry::View::Wow32,
    }
}

fn open(hive: Hive, path: &str, view: View) -> Option<Key> {
    Key::open(registry_hive(hive), path, registry_view(view))
        .ok()
        .flatten()
}

fn values(key: &Key) -> RawValues {
    let string = |name| key.string(name).ok().flatten();
    let flag = |name| {
        key.dword(name)
            .ok()
            .flatten()
            .is_some_and(|value| value != 0)
    };
    RawValues {
        display_name: string("DisplayName"),
        publisher: string("Publisher"),
        uninstall_string: string("UninstallString"),
        quiet_uninstall_string: string("QuietUninstallString"),
        install_location: string("InstallLocation"),
        display_icon: string("DisplayIcon"),
        parent_key_name: string("ParentKeyName"),
        release_type: string("ReleaseType"),
        system_component: flag("SystemComponent"),
        windows_installer: flag("WindowsInstaller"),
        no_remove: flag("NoRemove"),
    }
}

pub(super) fn uninstall_entries() -> Vec<UninstallEntry> {
    let mut entries = Vec::new();
    for (hive, view) in SOURCES {
        let Some(parent) = open(hive, &KeyLocation::parent_path(hive, view), view) else {
            continue;
        };
        for key in parent.subkey_names() {
            let Ok(Some(child)) = parent.open_child(&key, registry_view(view)) else {
                continue;
            };
            let location = KeyLocation { hive, view, key };
            entries.extend(entry_from(location, values(&child)));
        }
    }
    entries
}

pub(super) fn is_registered(location: &KeyLocation) -> bool {
    let path = format!(
        r"{}\{}",
        KeyLocation::parent_path(location.hive, location.view),
        location.key
    );
    open(location.hive, &path, location.view).is_some()
}
