mod platform;

use anyhow::Result;
use qol_headless::{DoctorCheck, DoctorCheckResult};
use serde_json::json;

use self::platform::{DirectoryState, PlatformInspection};

pub(crate) fn checks() -> Vec<DoctorCheck> {
    vec![
        DoctorCheck::new(
            "platform_supported",
            "Verify the current platform is declared by Remove App.",
            platform_supported_check,
        ),
        DoctorCheck::new(
            "config_readable",
            "Verify Remove App does not depend on persistent configuration.",
            config_readable_check,
        ),
        DoctorCheck::new(
            "inventory_readable",
            "Inspect application inventory roots without enumerating applications.",
            inventory_readable_check,
        ),
        DoctorCheck::new(
            "removal_prerequisites",
            "Inspect Trash and removal prerequisites without changing the filesystem.",
            removal_prerequisites_check,
        ),
    ]
}

fn platform_supported_check() -> Result<DoctorCheckResult> {
    let inspection = platform::inspect();
    let result = if inspection.supported {
        DoctorCheckResult::ok(
            "platform_supported",
            format!(
                "{} is declared and has a Remove App backend.",
                inspection.name
            ),
        )
    } else {
        DoctorCheckResult::fail(
            "platform_supported",
            format!("{} is not declared by Remove App.", inspection.name),
        )
        .with_fix("Run Remove App on Linux or macOS.")
    };
    Ok(result.with_details(json!({
        "platform": inspection.name,
        "declared": inspection.supported,
        "inspection": "metadata_only",
    })))
}

fn config_readable_check() -> Result<DoctorCheckResult> {
    Ok(DoctorCheckResult::ok(
        "config_readable",
        "Remove App has no persistent plugin configuration to read or create.",
    )
    .with_details(json!({
        "persistent_config": false,
        "inspection": "none",
    })))
}

fn inventory_readable_check() -> Result<DoctorCheckResult> {
    inventory_result(&platform::inspect())
}

fn inventory_result(inspection: &PlatformInspection) -> Result<DoctorCheckResult> {
    let details = json!({
        "platform": inspection.name,
        "inspection": "metadata_only",
        "roots": inspection
            .inventory_roots
            .iter()
            .map(platform::DirectoryInspection::details)
            .collect::<Vec<_>>(),
    });

    if !inspection.supported {
        return Ok(DoctorCheckResult::fail(
            "inventory_readable",
            "Application inventory inspection is unavailable on this platform.",
        )
        .with_fix("Run Remove App on Linux or macOS.")
        .with_details(details));
    }

    let available = inspection
        .inventory_roots
        .iter()
        .filter(|root| root.state == DirectoryState::Directory)
        .count();
    let invalid = inspection
        .inventory_roots
        .iter()
        .filter(|root| {
            matches!(
                root.state,
                DirectoryState::WrongType | DirectoryState::Unreadable(_)
            )
        })
        .count();

    let result = if invalid > 0 {
        DoctorCheckResult::fail(
            "inventory_readable",
            format!(
                "{invalid} application inventory root(s) could not be inspected as directories."
            ),
        )
        .with_fix("Repair the reported inventory root permissions or path types.")
    } else if available == 0 {
        DoctorCheckResult::fail(
            "inventory_readable",
            "No application inventory directory is available.",
        )
        .with_fix("Restore a standard application directory for this platform.")
    } else {
        DoctorCheckResult::ok(
            "inventory_readable",
            format!("{available} application inventory root(s) are observable by metadata."),
        )
    };

    Ok(result.with_details(details))
}

fn removal_prerequisites_check() -> Result<DoctorCheckResult> {
    removal_prerequisites_result(&platform::inspect())
}

fn removal_prerequisites_result(inspection: &PlatformInspection) -> Result<DoctorCheckResult> {
    let details = json!({
        "platform": inspection.name,
        "inspection": "metadata_only",
        "trash": inspection
            .trash
            .as_ref()
            .map(platform::DirectoryInspection::details),
        "creation_anchor": inspection
            .trash_creation_anchor
            .as_ref()
            .map(platform::DirectoryInspection::details),
        "permanent_delete_backend": inspection.supported,
    });

    if !inspection.supported {
        return Ok(DoctorCheckResult::fail(
            "removal_prerequisites",
            "Trash and permanent-removal backends are unavailable on this platform.",
        )
        .with_fix("Run Remove App on Linux or macOS.")
        .with_details(details));
    }

    let Some(trash) = inspection.trash.as_ref() else {
        return Ok(DoctorCheckResult::fail(
            "removal_prerequisites",
            "The current user's Trash location could not be resolved.",
        )
        .with_fix("Set HOME to an absolute user directory before running Remove App.")
        .with_details(details));
    };

    let result = match &trash.state {
        DirectoryState::Directory => DoctorCheckResult::ok(
            "removal_prerequisites",
            "The Trash directory is observable; permanent deletion uses the platform filesystem backend.",
        ),
        DirectoryState::Missing
            if inspection
                .trash_creation_anchor
                .as_ref()
                .is_some_and(|anchor| anchor.state == DirectoryState::Directory) =>
        {
            DoctorCheckResult::ok(
                "removal_prerequisites",
                "The Trash target resolves beneath an observable user directory and may be created by an actual removal.",
            )
        }
        DirectoryState::Missing => DoctorCheckResult::fail(
            "removal_prerequisites",
            "The Trash target and its user-directory anchor are unavailable.",
        )
        .with_fix("Restore the user home directory before running Remove App."),
        DirectoryState::WrongType => DoctorCheckResult::fail(
            "removal_prerequisites",
            "The resolved Trash path is not a directory.",
        )
        .with_fix("Move the conflicting path and restore a Trash directory."),
        DirectoryState::Unreadable(_) => DoctorCheckResult::fail(
            "removal_prerequisites",
            "The resolved Trash path metadata is not readable.",
        )
        .with_fix("Repair the Trash path permissions before running Remove App."),
    };

    Ok(result.with_details(details))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use qol_headless::DoctorStatus;

    use super::*;
    use crate::doctor::platform::DirectoryInspection;

    fn inspection(
        supported: bool,
        roots: Vec<DirectoryInspection>,
        trash: Option<DirectoryInspection>,
        anchor: Option<DirectoryInspection>,
    ) -> PlatformInspection {
        PlatformInspection {
            name: "Test OS",
            supported,
            inventory_roots: roots,
            trash,
            trash_creation_anchor: anchor,
        }
    }

    fn directory(path: &str, state: DirectoryState) -> DirectoryInspection {
        DirectoryInspection {
            path: PathBuf::from(path),
            state,
        }
    }

    #[test]
    fn inventory_check_uses_directory_observations() {
        let report = inventory_result(&inspection(
            true,
            vec![directory("/apps", DirectoryState::Directory)],
            None,
            None,
        ))
        .unwrap();

        assert_eq!(report.status, DoctorStatus::Ok);
        assert_eq!(report.details.unwrap()["inspection"], "metadata_only");
    }

    #[test]
    fn inventory_check_rejects_wrong_path_types() {
        let report = inventory_result(&inspection(
            true,
            vec![directory("/apps", DirectoryState::WrongType)],
            None,
            None,
        ))
        .unwrap();

        assert_eq!(report.status, DoctorStatus::Fail);
    }

    #[test]
    fn missing_trash_is_safe_when_the_user_directory_exists() {
        let report = removal_prerequisites_result(&inspection(
            true,
            Vec::new(),
            Some(directory("/home/test/.Trash", DirectoryState::Missing)),
            Some(directory("/home/test", DirectoryState::Directory)),
        ))
        .unwrap();

        assert_eq!(report.status, DoctorStatus::Ok);
        assert!(report.message.contains("may be created"));
    }

    #[test]
    fn unsupported_platform_fails_without_platform_io() {
        let inspection = inspection(false, Vec::new(), None, None);

        assert_eq!(
            inventory_result(&inspection).unwrap().status,
            DoctorStatus::Fail
        );
        assert_eq!(
            removal_prerequisites_result(&inspection).unwrap().status,
            DoctorStatus::Fail
        );
    }
}
