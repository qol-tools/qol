use super::fingerprint::git_tracks;
use super::options::resolve_existing_ancestors;
use anyhow::{bail, Context, Result};
use std::fs;
use std::io::Read;
use std::path::Path;

const REPORT_INSPECTION_LIMIT: u64 = 4 * 1024 * 1024;

pub(super) fn validate_report_destination(root: &Path, destination: &Path) -> Result<()> {
    let canonical_root = root
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", root.display()))?;
    let metadata = match fs::symlink_metadata(destination) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect {}", destination.display()))
        }
    };
    if let Some(metadata) = metadata {
        if metadata.file_type().is_symlink() {
            bail!("--report path is a symlink: {}", destination.display());
        }
        if metadata.is_dir() {
            bail!("--report path is a directory: {}", destination.display());
        }
        if !metadata.is_file() {
            bail!(
                "--report path is not a regular file: {}",
                destination.display()
            );
        }
        if !is_prior_report(destination)? {
            bail!(
                "--report path exists and is not a prior qol-check report: {}",
                destination.display()
            );
        }
    }
    let resolved = resolve_existing_ancestors(destination)?;
    match (
        destination.strip_prefix(root).ok(),
        resolved.strip_prefix(&canonical_root).ok(),
    ) {
        (None, None) => {}
        (Some(lexical), Some(resolved)) if lexical == resolved => {}
        _ => bail!(
            "--report path changes location through a symlinked ancestor: {}",
            destination.display()
        ),
    }
    if let Ok(relative) = resolved.strip_prefix(&canonical_root) {
        if git_tracks(root, relative)? {
            bail!(
                "--report path is a tracked source path: {}",
                destination.display()
            );
        }
    }
    Ok(())
}

fn is_prior_report(path: &Path) -> Result<bool> {
    let file =
        fs::File::open(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut content = Vec::new();
    file.take(REPORT_INSPECTION_LIMIT)
        .read_to_end(&mut content)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let Ok(document) = serde_json::from_slice::<serde_json::Value>(&content) else {
        return Ok(false);
    };
    Ok(
        document.get("name").and_then(serde_json::Value::as_str) == Some("qol-check")
            && document
                .get("status")
                .and_then(serde_json::Value::as_str)
                .is_some()
            && document
                .get("inputs")
                .is_some_and(serde_json::Value::is_object),
    )
}

#[cfg(test)]
mod tests {
    use super::super::report::CheckReport;
    use super::super::test_support::repository;
    use super::*;
    use chrono::Utc;
    use std::path::PathBuf;

    fn report_file(root: &Path) -> PathBuf {
        let path = root.join("verification/report.json");
        let report = CheckReport::new(root, "worktree", "linux", &path, Utc::now());
        report.write(&path).unwrap();
        path
    }

    #[test]
    fn destination_accepts_new_in_tree_and_external_paths() {
        let repository = repository();
        let root = repository.path();
        let external = tempfile::tempdir().unwrap();

        assert!(validate_report_destination(root, &root.join("verification/report.json")).is_ok());
        assert!(validate_report_destination(root, &external.path().join("report.json")).is_ok());
    }

    #[test]
    fn destination_accepts_only_authored_qol_check_reports() {
        let repository = repository();
        let root = repository.path();
        let report = report_file(root);

        assert!(validate_report_destination(root, &report).is_ok());

        fs::write(&report, "{\"name\":\"other\",\"status\":\"pass\"}\n").unwrap();
        assert!(validate_report_destination(root, &report).is_err());

        fs::write(&report, "not json\n").unwrap();
        assert!(validate_report_destination(root, &report).is_err());
    }

    #[test]
    fn destination_rejects_existing_non_report_files() {
        let repository = repository();
        let root = repository.path();
        let tracked = root.join("tracked.txt");

        let error = validate_report_destination(root, &tracked)
            .unwrap_err()
            .to_string();

        assert!(error.contains("prior qol-check report"), "got: {error}");
    }

    #[test]
    fn destination_rejects_tracked_deleted_paths() {
        let repository = repository();
        let root = repository.path();
        fs::remove_file(root.join("tracked.txt")).unwrap();

        let error = validate_report_destination(root, &root.join("tracked.txt"))
            .unwrap_err()
            .to_string();

        assert!(error.contains("tracked source path"), "got: {error}");
    }

    #[test]
    fn destination_rejects_directories() {
        let repository = repository();
        let root = repository.path();
        let directory = root.join("verification");
        fs::create_dir_all(&directory).unwrap();

        let error = validate_report_destination(root, &directory)
            .unwrap_err()
            .to_string();

        assert!(error.contains("is a directory"), "got: {error}");
    }

    #[cfg(unix)]
    #[test]
    fn destination_rejects_symlinks() {
        let repository = repository();
        let root = repository.path();
        let target = root.join("verification/report.json");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, "{}\n").unwrap();
        let link = root.join("link.json");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let error = validate_report_destination(root, &link)
            .unwrap_err()
            .to_string();

        assert!(error.contains("is a symlink"), "got: {error}");
    }

    #[cfg(unix)]
    #[test]
    fn destination_rejects_an_ancestor_symlink_that_escapes_the_repository() {
        let repository = repository();
        let root = repository.path();
        let outside = tempfile::tempdir().unwrap();
        let link = root.join("escape");
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();

        let error = validate_report_destination(root, &link.join("report.json"))
            .unwrap_err()
            .to_string();

        assert!(error.contains("symlinked ancestor"), "got: {error}");
    }

    #[cfg(unix)]
    #[test]
    fn destination_rejects_an_ancestor_symlink_that_relocates_inside_the_repository() {
        let repository = repository();
        let root = repository.path();
        let source = root.join("src");
        fs::create_dir_all(&source).unwrap();
        let link = root.join("alias");
        std::os::unix::fs::symlink(&source, &link).unwrap();

        let error = validate_report_destination(root, &link.join("report.json"))
            .unwrap_err()
            .to_string();

        assert!(error.contains("symlinked ancestor"), "got: {error}");
    }
}
