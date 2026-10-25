use qol_platform::native::registry::{read_string, Hive, View};

use crate::report::{Blocker, BlockerKind};
use crate::role::Role;

const POLICY_KEY: &str = r"SOFTWARE\Policies\Microsoft\Windows\System";
const POLICY_VALUE: &str = "DefaultAssociationsConfiguration";

pub(super) fn current() -> Vec<Blocker> {
    let value = match read_string(Hive::LocalMachine, POLICY_KEY, POLICY_VALUE, View::Native) {
        Ok(value) => value,
        Err(error) => {
            log::debug!("host defaults: {POLICY_VALUE} unreadable: {error}");
            None
        }
    };
    policy_blocker(value.as_deref()).into_iter().collect()
}

fn policy_blocker(value: Option<&str>) -> Option<Blocker> {
    let file = value.map(str::trim).filter(|file| !file.is_empty())?;
    Some(Blocker {
        kind: BlockerKind::PolicyOverride,
        roles: vec![Role::FolderOpen],
        detail: format!("the policy file {file} resets file associations at sign-in"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_non_empty_policy_value_blocks_folder_open() {
        let cases = [
            ("unset", None, false),
            ("empty", Some(""), false),
            ("blank", Some("  "), false),
            (
                "a policy file",
                Some(r"\\server\share\associations.xml"),
                true,
            ),
            ("padded", Some(r" C:\policy\associations.xml "), true),
        ];
        for (label, value, blocked) in cases {
            assert_eq!(policy_blocker(value).is_some(), blocked, "case: {label}");
        }
    }

    #[test]
    fn the_blocker_names_the_policy_file_and_the_role() {
        let blocker = policy_blocker(Some(r" C:\policy\associations.xml ")).unwrap();
        assert_eq!(blocker.kind, BlockerKind::PolicyOverride);
        assert_eq!(blocker.roles, [Role::FolderOpen]);
        assert_eq!(
            blocker.detail,
            r"the policy file C:\policy\associations.xml resets file associations at sign-in"
        );
    }
}
