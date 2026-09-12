use super::snapshot::sanitize_git_environment;
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CONTENT_BUFFER: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SourceFingerprint {
    pub(super) digest: String,
}

impl SourceFingerprint {
    pub(super) fn capture(root: &Path, exclusions: &[PathBuf]) -> Result<Self> {
        let canonical_root = root
            .canonicalize()
            .with_context(|| format!("failed to resolve {}", root.display()))?;
        let status = git_output(
            root,
            &[
                "status",
                "--porcelain=v2",
                "-z",
                "--untracked-files=all",
                "--no-renames",
            ],
        )?;
        let mut records = status
            .stdout
            .split(|byte| *byte == 0)
            .filter(|record| !record.is_empty() && !record.starts_with(b"#"))
            .collect::<Vec<_>>();
        records.sort_unstable();
        for record in &records {
            validate_record(record)?;
        }
        reject_nonstandard_index_flags(root)?;
        let head = git_stdout(root, &["rev-parse", "HEAD"], "source HEAD")?;
        let index_tree = git_stdout(root, &["write-tree"], "source index tree")?;
        let mut hasher = Sha256::new();
        hash_field(&mut hasher, b"algorithm", b"git-delta-sha256");
        hash_field(&mut hasher, b"head", head.as_bytes());
        hash_field(&mut hasher, b"index-tree", index_tree.as_bytes());
        for record in records {
            hash_record(&mut hasher, root, &canonical_root, exclusions, record)?;
        }
        Ok(Self {
            digest: format!("{:x}", hasher.finalize()),
        })
    }
}

pub(super) fn compare(before: &SourceFingerprint, after: &SourceFingerprint) -> Result<()> {
    if before.digest == after.digest {
        return Ok(());
    }
    bail!(
        "source changed while qol check was running: {} became {}",
        before.digest,
        after.digest
    )
}

fn hash_record(
    hasher: &mut Sha256,
    root: &Path,
    canonical_root: &Path,
    exclusions: &[PathBuf],
    record: &[u8],
) -> Result<()> {
    match record[0] {
        b'1' => {
            let fields = record.splitn(9, |byte| *byte == b' ').collect::<Vec<_>>();
            hash_field(hasher, b"kind", b"1");
            for (label, field) in [
                ("xy", fields[1]),
                ("sub", fields[2]),
                ("mode-head", fields[3]),
                ("mode-index", fields[4]),
                ("mode-worktree", fields[5]),
                ("blob-head", fields[6]),
                ("blob-index", fields[7]),
            ] {
                hash_field(hasher, label.as_bytes(), field);
            }
            let path = fields[8];
            hash_field(hasher, b"path", path);
            let mode = fields[4];
            let relative = path_from_bytes(path)?;
            hash_content(hasher, root, canonical_root, &relative, mode)?;
        }
        b'?' => {
            let path = record.get(2..).unwrap_or_default();
            let relative = path_from_bytes(path)?;
            if excluded(&root.join(&relative), exclusions) {
                return Ok(());
            }
            hash_field(hasher, b"kind", b"?");
            hash_field(hasher, b"path", path);
            hash_content(hasher, root, canonical_root, &relative, b"")?;
        }
        _ => bail!("unsupported git status record for {}", record_label(record)),
    }
    Ok(())
}

fn validate_record(record: &[u8]) -> Result<()> {
    match record[0] {
        b'1' => {
            if record.splitn(9, |byte| *byte == b' ').count() != 9 {
                bail!("unsupported git status record for {}", record_label(record));
            }
        }
        b'2' => bail!("git reported a rename record; source identity is unavailable"),
        b'u' => bail!("the index has unresolved merge conflicts; source identity is unavailable"),
        b'?' => {}
        _ => bail!("unsupported git status record for {}", record_label(record)),
    }
    Ok(())
}

fn reject_nonstandard_index_flags(root: &Path) -> Result<()> {
    let output = git_output(root, &["ls-files", "-v", "-z"])?;
    let nonstandard = output
        .stdout
        .split(|byte| *byte == 0)
        .any(|entry| !entry.is_empty() && !entry.starts_with(b"H "));
    if nonstandard {
        bail!(
            "the index has skip-worktree or assume-unchanged flags; source identity is unavailable"
        );
    }
    Ok(())
}

fn hash_content(
    hasher: &mut Sha256,
    root: &Path,
    canonical_root: &Path,
    relative: &Path,
    index_mode: &[u8],
) -> Result<()> {
    let path = root.join(relative);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            hash_field(hasher, b"content", b"missing");
            return Ok(());
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect source path {}", path.display()))
        }
    };
    if metadata.file_type().is_symlink() {
        let target = fs::read_link(&path)
            .with_context(|| format!("failed to read source symlink {}", path.display()))?;
        hash_field(hasher, b"content", b"symlink");
        hash_field(hasher, b"target", target.as_os_str().as_encoded_bytes());
        return Ok(());
    }
    if metadata.is_dir() {
        if index_mode == b"160000".as_slice() {
            hash_field(hasher, b"content", b"gitlink");
            return Ok(());
        }
        bail!(
            "source path {} is a directory where git tracks a file",
            path.display()
        );
    }
    if !metadata.is_file() {
        bail!("unsupported source file type at {}", path.display());
    }
    let parent = path
        .parent()
        .with_context(|| format!("source path {} has no parent", path.display()))?;
    let canonical_parent = parent
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", parent.display()))?;
    if !canonical_parent.starts_with(canonical_root) {
        bail!("source path {} escapes the repository root", path.display());
    }
    hash_field(hasher, b"content", b"file");
    hash_field(hasher, b"length", &metadata.len().to_le_bytes());
    let mut file = File::open(&path)
        .with_context(|| format!("failed to read source file {}", path.display()))?;
    let mut buffer = vec![0_u8; CONTENT_BUFFER];
    loop {
        let count = file
            .read(&mut buffer)
            .with_context(|| format!("failed to read source file {}", path.display()))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(())
}

fn excluded(path: &Path, exclusions: &[PathBuf]) -> bool {
    exclusions.iter().any(|exclusion| {
        if path == exclusion || path.starts_with(exclusion) {
            return true;
        }
        match (path.canonicalize(), exclusion.canonicalize()) {
            (Ok(path), Ok(exclusion)) => path == exclusion || path.starts_with(&exclusion),
            _ => false,
        }
    })
}

fn path_from_bytes(bytes: &[u8]) -> Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        Ok(PathBuf::from(OsStr::from_bytes(bytes)))
    }
    #[cfg(not(unix))]
    {
        let text = std::str::from_utf8(bytes).context("git returned a non-UTF-8 source path")?;
        Ok(PathBuf::from(text))
    }
}

fn hash_field(hasher: &mut Sha256, label: &[u8], value: &[u8]) {
    hasher.update((label.len() as u64).to_le_bytes());
    hasher.update(label);
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value);
}

fn record_label(record: &[u8]) -> String {
    String::from_utf8_lossy(&record[..record.len().min(64)]).into_owned()
}

pub(super) fn git_tracks(root: &Path, relative: &Path) -> Result<bool> {
    let output = git_paths_output(root, &["ls-files", "-z"], relative)?;
    if !output.status.success() {
        bail!(
            "git ls-files failed with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(!output.stdout.is_empty())
}

pub(super) fn git_ignores(root: &Path, relative: &Path) -> Result<bool> {
    let output = git_paths_output(root, &["check-ignore", "-q"], relative)?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => bail!(
            "git check-ignore failed with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    }
}

fn git_paths_output(root: &Path, prefix: &[&str], path: &Path) -> Result<Output> {
    let mut command = Command::new("git");
    command.current_dir(root).args(prefix).arg("--").arg(path);
    sanitize_git_environment(&mut command);
    command
        .output()
        .with_context(|| format!("failed to run git {}", prefix.join(" ")))
}

fn git_output(root: &Path, args: &[&str]) -> Result<Output> {
    let mut command = Command::new("git");
    command.current_dir(root).args(args);
    sanitize_git_environment(&mut command);
    let output = command
        .output()
        .with_context(|| format!("failed to run git {}", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "git {} failed with {}: {}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output)
}

fn git_stdout(root: &Path, args: &[&str], label: &str) -> Result<String> {
    let output = git_output(root, args)?;
    let value = String::from_utf8(output.stdout)
        .with_context(|| format!("git returned a non-UTF-8 {label}"))?;
    let value = value.trim().to_string();
    if value.is_empty() {
        bail!("git returned an empty {label}");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{commit_file, git, head, repository};
    use super::*;

    fn capture(root: &Path) -> SourceFingerprint {
        SourceFingerprint::capture(root, &[]).unwrap()
    }

    #[test]
    fn fingerprint_is_stable_for_an_unchanged_tree() {
        let repository = repository();
        assert_eq!(
            capture(repository.path()).digest,
            capture(repository.path()).digest
        );
    }

    #[test]
    fn fingerprint_tracks_modified_untracked_and_deleted_sources() {
        let repository = repository();
        let root = repository.path();
        let baseline = capture(root);

        fs::write(root.join("tracked.txt"), "changed\n").unwrap();
        assert_ne!(capture(root).digest, baseline.digest);

        fs::write(root.join("tracked.txt"), "base\n").unwrap();
        assert_eq!(capture(root).digest, baseline.digest);

        fs::write(root.join("untracked new.rs"), "fn main() {}\n").unwrap();
        let untracked = capture(root);
        assert_ne!(untracked.digest, baseline.digest);

        fs::write(root.join("untracked new.rs"), "fn main() { }\n").unwrap();
        assert_ne!(capture(root).digest, untracked.digest);

        fs::remove_file(root.join("untracked new.rs")).unwrap();
        fs::remove_file(root.join("tracked.txt")).unwrap();
        assert_ne!(capture(root).digest, baseline.digest);
    }

    #[test]
    fn fingerprint_covers_paths_with_spaces_and_newlines() {
        let repository = repository();
        let root = repository.path();
        let baseline = capture(root);
        let odd = root.join("odd dir").join("a b\nc.rs");
        fs::create_dir_all(odd.parent().unwrap()).unwrap();
        fs::write(&odd, "fn  main() {}\n").unwrap();
        let with_odd = capture(root);
        assert_ne!(with_odd.digest, baseline.digest);

        fs::write(&odd, "fn main() {}\n").unwrap();
        assert_ne!(capture(root).digest, with_odd.digest);

        fs::remove_file(&odd).unwrap();
        assert_eq!(capture(root).digest, baseline.digest);

        let tracked = root.join("spaced name.rs");
        fs::write(&tracked, "fn  spaced() {}\n").unwrap();
        git(root, ["add", "spaced name.rs"]);
        git(root, ["commit", "--quiet", "-m", "spaced"]);
        let committed = capture(root);
        fs::write(&tracked, "fn spaced() {}\n").unwrap();
        assert_ne!(capture(root).digest, committed.digest);
    }

    #[cfg(unix)]
    #[test]
    fn fingerprint_tracks_worktree_modes() {
        use std::os::unix::fs::PermissionsExt;

        let repository = repository();
        let root = repository.path();
        let baseline = capture(root);
        fs::set_permissions(root.join("tracked.txt"), fs::Permissions::from_mode(0o755)).unwrap();

        assert_ne!(capture(root).digest, baseline.digest);
    }

    #[test]
    fn fingerprint_ignores_ignored_build_outputs() {
        let repository = repository();
        let root = repository.path();
        fs::write(root.join(".gitignore"), "target/\n").unwrap();
        git(root, ["add", ".gitignore"]);
        git(root, ["commit", "--quiet", "-m", "ignore target"]);
        let baseline = capture(root);

        fs::create_dir_all(root.join("target/qol-check")).unwrap();
        fs::write(root.join("target/qol-check/report.json"), "{}\n").unwrap();
        fs::create_dir_all(root.join("target/debug")).unwrap();
        fs::write(root.join("target/debug/qol"), "binary").unwrap();

        assert_eq!(capture(root).digest, baseline.digest);
    }

    #[test]
    fn fingerprint_excludes_only_named_node_owned_paths() {
        let repository = repository();
        let root = repository.path();
        let run_dir = root.join("target/qol-check/run-1");
        let exclusions = [run_dir.clone()];
        let baseline = SourceFingerprint::capture(root, &exclusions).unwrap();

        fs::create_dir_all(root.join("verification")).unwrap();
        fs::write(root.join("verification/report.json"), "{}\n").unwrap();
        let with_report = SourceFingerprint::capture(root, &exclusions).unwrap();
        assert_ne!(
            with_report.digest, baseline.digest,
            "a caller-chosen report path must stay visible to source identity"
        );

        fs::create_dir_all(&run_dir).unwrap();
        fs::write(run_dir.join("report.json"), "{}\n").unwrap();
        assert_eq!(
            SourceFingerprint::capture(root, &exclusions)
                .unwrap()
                .digest,
            with_report.digest,
            "a named node-owned path must stay excluded"
        );
    }

    #[test]
    fn fingerprint_skips_excluded_untracked_artifacts_but_keeps_tracked_sources() {
        let repository = repository();
        let root = repository.path();
        let build = root.join("build");
        let artifacts = build.join("artifacts");
        let exclusions = [build.clone()];
        let baseline = SourceFingerprint::capture(root, &exclusions).unwrap();

        fs::create_dir_all(&artifacts).unwrap();
        fs::write(artifacts.join("one.bin"), "one\n").unwrap();
        assert_eq!(
            SourceFingerprint::capture(root, &exclusions)
                .unwrap()
                .digest,
            baseline.digest,
            "creating an excluded untracked artifact must not change the fingerprint"
        );

        fs::write(artifacts.join("one.bin"), "two\n").unwrap();
        assert_eq!(
            SourceFingerprint::capture(root, &exclusions)
                .unwrap()
                .digest,
            baseline.digest,
            "mutating an excluded untracked artifact must not change the fingerprint"
        );

        fs::write(artifacts.join("two.bin"), "two\n").unwrap();
        assert_eq!(
            SourceFingerprint::capture(root, &exclusions)
                .unwrap()
                .digest,
            baseline.digest,
            "adding an excluded untracked artifact must not change the fingerprint"
        );

        fs::rename(artifacts.join("one.bin"), artifacts.join("renamed.bin")).unwrap();
        assert_eq!(
            SourceFingerprint::capture(root, &exclusions)
                .unwrap()
                .digest,
            baseline.digest,
            "renaming an excluded untracked artifact must not change the fingerprint"
        );

        fs::remove_dir_all(&artifacts).unwrap();
        assert_eq!(
            SourceFingerprint::capture(root, &exclusions)
                .unwrap()
                .digest,
            baseline.digest,
            "deleting an excluded untracked artifact must not change the fingerprint"
        );

        fs::write(root.join("scratch.rs"), "fn main() {}\n").unwrap();
        assert_ne!(
            SourceFingerprint::capture(root, &exclusions)
                .unwrap()
                .digest,
            baseline.digest,
            "ordinary untracked source must stay visible"
        );

        fs::write(build.join("tracked.rs"), "fn tracked() {}\n").unwrap();
        git(root, ["add", "build/tracked.rs"]);
        git(root, ["commit", "--quiet", "-m", "tracked under exclusion"]);
        let committed = SourceFingerprint::capture(root, &exclusions).unwrap();

        fs::write(build.join("tracked.rs"), "fn tracked_changed() {}\n").unwrap();
        assert_ne!(
            SourceFingerprint::capture(root, &exclusions)
                .unwrap()
                .digest,
            committed.digest,
            "tracked source under an exclusion must stay visible"
        );
    }

    #[test]
    fn fingerprint_reports_an_unresolved_index_explicitly() {
        let repository = repository();
        let root = repository.path();
        git(root, ["checkout", "-b", "side"]);
        commit_file(root, "tracked.txt", "side\n");
        git(root, ["checkout", "-"]);
        commit_file(root, "tracked.txt", "main\n");
        let status = Command::new("git")
            .current_dir(root)
            .args(["merge", "side"])
            .status()
            .unwrap();
        assert!(!status.success());

        let error = SourceFingerprint::capture(root, &[])
            .unwrap_err()
            .to_string();

        assert!(error.contains("unresolved merge conflicts"), "got: {error}");
    }

    #[test]
    fn head_changes_are_part_of_the_fingerprint() {
        let repository = repository();
        let root = repository.path();
        let baseline = capture(root);

        let moved = commit_file(root, "other.txt", "other\n");

        assert_eq!(head(root), moved);
        assert_ne!(capture(root).digest, baseline.digest);
    }

    #[test]
    fn fingerprint_rejects_nonstandard_index_flags() {
        let repository = repository();
        let root = repository.path();
        git(root, ["update-index", "--skip-worktree", "tracked.txt"]);

        let error = SourceFingerprint::capture(root, &[])
            .unwrap_err()
            .to_string();

        assert!(
            error.contains("nonstandard") || error.contains("skip-worktree"),
            "got: {error}"
        );
    }

    #[test]
    fn compare_accepts_equal_fingerprints_and_rejects_drift() {
        let before = SourceFingerprint {
            digest: "before".to_string(),
        };
        let same = SourceFingerprint {
            digest: "before".to_string(),
        };
        let after = SourceFingerprint {
            digest: "after".to_string(),
        };

        assert!(compare(&before, &same).is_ok());
        let error = compare(&before, &after).unwrap_err().to_string();

        assert!(error.contains("source changed"), "got: {error}");
    }
}
