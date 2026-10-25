use std::path::{Path, PathBuf};

use plist::Value;

use crate::report::{Blocker, BlockerKind};
use crate::role::Role;

const MANAGED_PREFERENCES: &str = "/Library/Managed Preferences";
const LAUNCH_SERVICES_PLIST: &str = "com.apple.LaunchServices.plist";
const HANDLERS_KEY: &str = "LSHandlers";

pub(super) fn current() -> Vec<Blocker> {
    let policy = policy_blocker(&managed_handler_files());
    let secure_input = secure_input_blocker(qol_platform::secure_input_holder());
    policy.into_iter().chain(secure_input).collect()
}

fn managed_paths(user: Option<&str>) -> Vec<PathBuf> {
    let root = Path::new(MANAGED_PREFERENCES);
    let mut paths = vec![root.join(LAUNCH_SERVICES_PLIST)];
    if let Some(user) = user.filter(|user| !user.is_empty()) {
        paths.push(root.join(user).join(LAUNCH_SERVICES_PLIST));
    }
    paths
}

fn managed_handler_files() -> Vec<PathBuf> {
    let user = std::env::var("USER").ok();
    managed_paths(user.as_deref())
        .into_iter()
        .filter(|path| declares_handlers_on_disk(path))
        .collect()
}

fn declares_handlers_on_disk(path: &Path) -> bool {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return false,
        Err(error) => {
            log::debug!("host defaults: {} unreadable: {error}", path.display());
            return false;
        }
    };
    match declares_handlers(&bytes) {
        Ok(declared) => declared,
        Err(error) => {
            log::debug!("host defaults: {} is not a plist: {error}", path.display());
            false
        }
    }
}

fn declares_handlers(bytes: &[u8]) -> Result<bool, plist::Error> {
    let value = Value::from_reader(std::io::Cursor::new(bytes))?;
    Ok(value
        .as_dictionary()
        .and_then(|dictionary| dictionary.get(HANDLERS_KEY))
        .and_then(Value::as_array)
        .is_some_and(|handlers| !handlers.is_empty()))
}

fn policy_blocker(files: &[PathBuf]) -> Option<Blocker> {
    if files.is_empty() {
        return None;
    }
    let named: Vec<String> = files
        .iter()
        .map(|file| file.display().to_string())
        .collect();
    Some(Blocker {
        kind: BlockerKind::PolicyOverride,
        roles: vec![Role::FolderOpen],
        detail: format!(
            "a managed profile sets the default handlers in {}",
            named.join(", ")
        ),
    })
}

fn secure_input_blocker(holder: Option<qol_platform::SecureInputHolder>) -> Option<Blocker> {
    holder.map(|holder| Blocker {
        kind: BlockerKind::SecureInputHeld,
        roles: Role::slot_roles(),
        detail: holder.app,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xml(body: &str) -> Vec<u8> {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">{body}</plist>"
        )
        .into_bytes()
    }

    #[test]
    fn a_plist_declares_handlers_only_with_a_non_empty_lshandlers_array() {
        let cases = [
            (
                "one handler",
                xml("<dict><key>LSHandlers</key><array><dict><key>LSHandlerContentType</key><string>public.folder</string></dict></array></dict>"),
                Some(true),
            ),
            (
                "empty array",
                xml("<dict><key>LSHandlers</key><array/></dict>"),
                Some(false),
            ),
            (
                "other keys only",
                xml("<dict><key>LSQuarantine</key><true/></dict>"),
                Some(false),
            ),
            (
                "handlers is not an array",
                xml("<dict><key>LSHandlers</key><string>x</string></dict>"),
                Some(false),
            ),
            ("not a dictionary", xml("<array/>"), Some(false)),
            ("not a plist", b"garbage".to_vec(), None),
            ("empty file", Vec::new(), None),
        ];
        for (label, bytes, expected) in cases {
            assert_eq!(declares_handlers(&bytes).ok(), expected, "case: {label}");
        }
    }

    #[test]
    fn managed_paths_cover_the_machine_and_the_user_folder() {
        let cases = [(None, 1), (Some(""), 1), (Some("ada"), 2)];
        for (user, count) in cases {
            let paths = managed_paths(user);
            assert_eq!(paths.len(), count, "user: {user:?}");
            assert_eq!(
                paths[0],
                Path::new("/Library/Managed Preferences/com.apple.LaunchServices.plist")
            );
        }
        assert_eq!(
            managed_paths(Some("ada"))[1],
            Path::new("/Library/Managed Preferences/ada/com.apple.LaunchServices.plist")
        );
    }

    #[test]
    fn a_policy_blocker_names_each_file_and_stops_folder_open() {
        assert_eq!(policy_blocker(&[]), None);
        let blocker = policy_blocker(&[
            PathBuf::from("/Library/Managed Preferences/com.apple.LaunchServices.plist"),
            PathBuf::from("/Library/Managed Preferences/ada/com.apple.LaunchServices.plist"),
        ])
        .unwrap();
        assert_eq!(blocker.kind, BlockerKind::PolicyOverride);
        assert_eq!(blocker.roles, [Role::FolderOpen]);
        assert!(blocker.detail.contains("Managed Preferences/com.apple"));
        assert!(blocker.detail.contains("Managed Preferences/ada/com.apple"));
    }

    #[test]
    fn secure_input_blocks_every_slot_role_and_names_the_holder() {
        assert_eq!(secure_input_blocker(None), None);
        let blocker = secure_input_blocker(Some(qol_platform::SecureInputHolder {
            pid: 4242,
            app: "kitty".to_string(),
        }))
        .unwrap();
        assert_eq!(blocker.kind, BlockerKind::SecureInputHeld);
        assert_eq!(blocker.roles, Role::slot_roles());
        assert_eq!(blocker.detail, "kitty");
    }
}
