use super::command::{self, CapturedOutcome, CapturedOutput};
use super::report::FormattedFile;
use crate::progress::{step_label, StepKind};
use anyhow::{anyhow, bail, Context, Result};
use qol_process::CancellationToken;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

const FORMATTER_STDERR_LIMIT: usize = 2000;

pub(super) struct FormatOutcome {
    pub(super) files: Vec<FormattedFile>,
}

#[derive(Debug)]
pub(super) struct FormatFailure {
    pub(super) files: Vec<FormattedFile>,
    pub(super) failed_file: Option<String>,
    pub(super) error: anyhow::Error,
}

pub(super) fn run(
    source_root: &Path,
    requested: &[OsString],
    cancellation: &CancellationToken,
) -> Result<FormatOutcome, FormatFailure> {
    let plan = match FormatPlan::build(source_root, requested) {
        Ok(plan) => plan,
        Err(error) => {
            return Err(FormatFailure {
                files: Vec::new(),
                failed_file: None,
                error,
            })
        }
    };
    if let Err(error) = plan.verify_formatter() {
        return Err(FormatFailure {
            files: Vec::new(),
            failed_file: None,
            error,
        });
    }
    let mut files = Vec::new();
    for file in &plan.files {
        if cancellation.is_cancelled() {
            return Err(FormatFailure {
                files,
                failed_file: Some(file.relative.clone()),
                error: anyhow!("check cancelled before formatting {}", file.relative),
            });
        }
        match format_file(file, cancellation) {
            Ok(formatted) => files.push(formatted),
            Err(error) => {
                return Err(FormatFailure {
                    files,
                    failed_file: Some(file.relative.clone()),
                    error,
                })
            }
        }
    }
    Ok(FormatOutcome { files })
}

struct FormatPlan {
    files: Vec<PlannedFile>,
}

struct PlannedFile {
    path: PathBuf,
    canonical: PathBuf,
    relative: String,
    parent: PathBuf,
    edition: String,
    config_dir: Option<PathBuf>,
}

impl FormatPlan {
    fn build(source_root: &Path, requested: &[OsString]) -> Result<Self> {
        let canonical_root = source_root
            .canonicalize()
            .with_context(|| format!("failed to resolve {}", source_root.display()))?;
        let mut seen = HashSet::new();
        let mut files = Vec::new();
        for value in requested {
            let file = plan_file(source_root, &canonical_root, value)?;
            if seen.insert(file.canonical.clone()) {
                files.push(file);
            }
        }
        if files.is_empty() {
            bail!("--format-owned requires at least one path");
        }
        Ok(Self { files })
    }

    fn verify_formatter(&self) -> Result<()> {
        let output = Command::new("rustfmt")
            .arg("--version")
            .output()
            .context("--format-owned requires rustfmt on PATH")?;
        if !output.status.success() {
            bail!("installed rustfmt failed its version probe");
        }
        Ok(())
    }
}

fn plan_file(source_root: &Path, canonical_root: &Path, value: &OsString) -> Result<PlannedFile> {
    let text = value
        .to_str()
        .context("--format-owned paths must be valid UTF-8")?;
    if text.is_empty() {
        bail!("--format-owned requires a repository-relative path");
    }
    let relative_request = Path::new(text);
    for component in relative_request.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => {
                bail!("--format-owned path must be repository-relative: {text}")
            }
            Component::ParentDir => {
                bail!("--format-owned path must not contain `..`: {text}")
            }
            Component::CurDir => {}
            Component::Normal(name) => reject_glob(name, text)?,
        }
    }
    if relative_request.extension().and_then(OsStr::to_str) != Some("rs") {
        bail!("--format-owned path must name a Rust source file: {text}");
    }
    let path = source_root.join(relative_request);
    let metadata = fs::symlink_metadata(&path)
        .with_context(|| format!("failed to inspect --format-owned path {text}"))?;
    if metadata.file_type().is_symlink() {
        bail!("--format-owned path is a symlink: {text}");
    }
    if !metadata.is_file() {
        bail!("--format-owned path is not a regular file: {text}");
    }
    let canonical = path
        .canonicalize()
        .with_context(|| format!("failed to resolve --format-owned path {text}"))?;
    if !canonical.starts_with(canonical_root) {
        bail!("--format-owned path resolves outside the repository: {text}");
    }
    let parent = path
        .parent()
        .with_context(|| format!("--format-owned path has no parent directory: {text}"))?
        .to_path_buf();
    let edition = discover_edition(source_root, &parent)?;
    let config_dir = discover_rustfmt_config(&parent, source_root);
    let relative = display_path(source_root, &path);
    Ok(PlannedFile {
        path,
        canonical,
        relative,
        parent,
        edition,
        config_dir,
    })
}

fn reject_glob(name: &OsStr, text: &str) -> Result<()> {
    let contains_glob = name
        .to_string_lossy()
        .chars()
        .any(|character| matches!(character, '*' | '?' | '[' | ']' | '{' | '}'));
    if contains_glob {
        bail!("--format-owned path must be an exact file path, not a glob: {text}");
    }
    Ok(())
}

fn format_file(file: &PlannedFile, cancellation: &CancellationToken) -> Result<FormattedFile> {
    let original =
        fs::read(&file.path).with_context(|| format!("failed to read {}", file.relative))?;
    let before_sha256 = format!("{:x}", Sha256::digest(&original));
    let output = match rustfmt_bytes(file, &original, cancellation) {
        Ok(output) => output,
        Err(error) => {
            step_label(
                "format",
                StepKind::Info,
                &format!("{} failed", file.relative),
            );
            return Err(error);
        }
    };
    if output.is_empty() && !original.is_empty() {
        bail!("rustfmt produced no output for {}", file.relative);
    }
    let after_sha256 = format!("{:x}", Sha256::digest(&output));
    if output == original {
        step_label(
            "format",
            StepKind::Info,
            &format!("{} unchanged", file.relative),
        );
        return Ok(FormattedFile {
            path: file.relative.clone(),
            before_sha256,
            after_sha256,
            changed: false,
        });
    }
    commit_formatted(file, &original, &output, cancellation)?;
    step_label(
        "format",
        StepKind::Success,
        &format!("{} rewritten", file.relative),
    );
    Ok(FormattedFile {
        path: file.relative.clone(),
        before_sha256,
        after_sha256,
        changed: true,
    })
}

fn commit_formatted(
    file: &PlannedFile,
    original: &[u8],
    formatted: &[u8],
    cancellation: &CancellationToken,
) -> Result<()> {
    if cancellation.is_cancelled() {
        bail!("check cancelled before writing {}", file.relative);
    }
    let metadata = fs::symlink_metadata(&file.path)
        .with_context(|| format!("failed to re-inspect {}", file.relative))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!(
            "formatting target {} changed identity before write",
            file.relative
        );
    }
    let canonical = file
        .path
        .canonicalize()
        .with_context(|| format!("failed to re-resolve {}", file.relative))?;
    if canonical != file.canonical {
        bail!(
            "formatting target {} was replaced before write",
            file.relative
        );
    }
    let current =
        fs::read(&file.path).with_context(|| format!("failed to re-read {}", file.relative))?;
    if current != original {
        bail!(
            "concurrent edit detected for {}; refusing to overwrite it",
            file.relative
        );
    }
    qol_fs::atomic_write_durable(&file.path, formatted)
        .with_context(|| format!("failed to write {}", file.relative))
}

fn rustfmt_bytes(
    file: &PlannedFile,
    content: &[u8],
    cancellation: &CancellationToken,
) -> Result<Vec<u8>> {
    let mut command = Command::new("rustfmt");
    command
        .current_dir(&file.parent)
        .arg("--edition")
        .arg(&file.edition)
        .args(["--emit", "stdout"]);
    if let Some(config_dir) = &file.config_dir {
        command.arg("--config-path").arg(config_dir);
    }
    let CapturedOutput {
        stdout,
        stderr,
        outcome,
    } = command::run_captured(
        &mut command,
        content,
        cancellation,
        command::Containment::Preferred,
    );
    match outcome {
        CapturedOutcome::Failed(error) => {
            Err(error).with_context(|| format!("rustfmt did not complete for {}", file.relative))
        }
        CapturedOutcome::Exited(status) if status.success() => Ok(stdout),
        CapturedOutcome::Exited(status) => {
            let stderr = bounded(String::from_utf8_lossy(&stderr).trim());
            if stderr.is_empty() {
                bail!("rustfmt failed for {} with {}", file.relative, status);
            }
            bail!(
                "rustfmt failed for {} with {}: {stderr}",
                file.relative,
                status
            );
        }
    }
}

fn discover_edition(source_root: &Path, directory: &Path) -> Result<String> {
    let mut current = Some(directory);
    while let Some(directory) = current {
        let manifest = directory.join("Cargo.toml");
        if manifest.is_file() {
            if let Some(edition) = edition_from_manifest(&manifest)? {
                return Ok(edition);
            }
        }
        if directory == source_root {
            break;
        }
        current = directory.parent();
    }
    bail!(
        "cannot determine the Rust edition for {}",
        directory.display()
    )
}

fn discover_rustfmt_config(file_parent: &Path, source_root: &Path) -> Option<PathBuf> {
    let mut current = Some(file_parent);
    while let Some(directory) = current {
        for name in ["rustfmt.toml", ".rustfmt.toml"] {
            if directory.join(name).is_file() {
                return Some(directory.to_path_buf());
            }
        }
        if directory == source_root {
            return None;
        }
        current = directory.parent();
    }
    None
}

fn edition_from_manifest(path: &Path) -> Result<Option<String>> {
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let document: toml::Value =
        toml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))?;
    if let Some(edition) = document
        .get("package")
        .and_then(|package| package.get("edition"))
        .and_then(toml::Value::as_str)
    {
        return Ok(Some(edition.to_string()));
    }
    if let Some(edition) = document
        .get("workspace")
        .and_then(|workspace| workspace.get("package"))
        .and_then(|package| package.get("edition"))
        .and_then(toml::Value::as_str)
    {
        return Ok(Some(edition.to_string()));
    }
    Ok(None)
}

fn display_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

fn bounded(text: &str) -> String {
    if text.chars().count() <= FORMATTER_STDERR_LIMIT {
        return text.to_string();
    }
    text.chars().take(FORMATTER_STDERR_LIMIT).collect()
}

#[cfg(test)]
mod tests {
    use super::super::test_support::rustfmt_available;
    use super::*;

    fn token() -> CancellationToken {
        CancellationToken::new()
    }

    fn root_with_edition() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("Cargo.toml"),
            "[workspace.package]\nedition = \"2021\"\n",
        )
        .unwrap();
        root
    }

    fn source(root: &Path, relative: &str, content: &str) -> PathBuf {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn plan_rejects_unsafe_paths_before_any_write() {
        let root = root_with_edition();
        let untouched = source(root.path(), "src/lib.rs", "fn  main() {}\n");
        fs::create_dir_all(root.path().join("src/nested")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&untouched, root.path().join("src/link.rs")).unwrap();

        let cases = [
            "",
            "/tmp/outside.rs",
            "../outside.rs",
            "src/../outside.rs",
            "src/*.rs",
            "src/?.rs",
            "src/nested",
            "src/lib.txt",
            "src/missing.rs",
        ];
        for case in cases {
            let requested = [OsString::from(case)];
            assert!(
                FormatPlan::build(root.path(), &requested).is_err(),
                "case: {case}"
            );
            assert_eq!(
                fs::read_to_string(&untouched).unwrap(),
                "fn  main() {}\n",
                "case: {case}"
            );
        }
        #[cfg(unix)]
        {
            let requested = [OsString::from("src/link.rs")];
            assert!(FormatPlan::build(root.path(), &requested).is_err());
        }
    }

    #[test]
    fn plan_validates_every_path_before_any_write() {
        let root = root_with_edition();
        let good = source(root.path(), "src/good.rs", "fn  main() {}\n");
        let requested = [
            OsString::from("src/good.rs"),
            OsString::from("src/missing.rs"),
        ];

        assert!(FormatPlan::build(root.path(), &requested).is_err());
        assert_eq!(fs::read_to_string(&good).unwrap(), "fn  main() {}\n");
    }

    #[test]
    fn plan_deduplicates_alias_spellings() {
        let root = root_with_edition();
        source(root.path(), "src/lib.rs", "fn main() {}\n");
        let requested = [
            OsString::from("src/lib.rs"),
            OsString::from("./src/lib.rs"),
            OsString::from("src/./lib.rs"),
        ];

        let plan = FormatPlan::build(root.path(), &requested).unwrap();

        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.files[0].relative, "src/lib.rs");
    }

    #[test]
    fn plan_reads_the_owning_manifest_edition() {
        let root = root_with_edition();
        fs::create_dir_all(root.path().join("crates/a")).unwrap();
        fs::write(
            root.path().join("crates/a/Cargo.toml"),
            "[package]\nname = \"a\"\nedition = \"2024\"\n",
        )
        .unwrap();
        source(root.path(), "crates/a/src/a.rs", "fn main() {}\n");
        source(root.path(), "src/b.rs", "fn main() {}\n");
        let requested = [
            OsString::from("crates/a/src/a.rs"),
            OsString::from("src/b.rs"),
        ];

        let plan = FormatPlan::build(root.path(), &requested).unwrap();

        assert_eq!(plan.files[0].edition, "2024");
        assert_eq!(plan.files[1].edition, "2021");
    }

    #[test]
    fn plan_finds_the_nearest_rustfmt_config_or_none() {
        let root = root_with_edition();
        source(root.path(), "src/nested/a.rs", "fn main() {}\n");
        let requested = [OsString::from("src/nested/a.rs")];

        assert_eq!(
            FormatPlan::build(root.path(), &requested).unwrap().files[0].config_dir,
            None
        );

        fs::write(root.path().join("rustfmt.toml"), "max_width = 90\n").unwrap();
        assert_eq!(
            FormatPlan::build(root.path(), &requested).unwrap().files[0].config_dir,
            Some(root.path().to_path_buf())
        );

        fs::write(root.path().join("src/.rustfmt.toml"), "max_width = 80\n").unwrap();
        assert_eq!(
            FormatPlan::build(root.path(), &requested).unwrap().files[0].config_dir,
            Some(root.path().join("src"))
        );
    }

    #[test]
    fn plan_requires_a_discoverable_edition() {
        let root = tempfile::tempdir().unwrap();
        source(root.path(), "src/lib.rs", "fn main() {}\n");
        let requested = [OsString::from("src/lib.rs")];

        let error = FormatPlan::build(root.path(), &requested)
            .err()
            .unwrap()
            .to_string();

        assert!(
            error.contains("cannot determine the Rust edition"),
            "got: {error}"
        );
    }

    #[test]
    fn formatting_rewrites_only_the_named_file() {
        if !rustfmt_available() {
            return;
        }
        let root = root_with_edition();
        let main = source(root.path(), "src/lib.rs", "mod child;\n\nfn  main() {}\n");
        let child = source(root.path(), "src/child.rs", "fn  child() {}\n");
        let sibling = source(root.path(), "src/sibling.rs", "fn  sibling() {}\n");
        let original = fs::read(&main).unwrap();
        let requested = [OsString::from("src/lib.rs")];

        let formatted = run(root.path(), &requested, &token()).unwrap().files;

        assert_eq!(formatted.len(), 1);
        assert_eq!(formatted[0].path, "src/lib.rs");
        assert!(formatted[0].changed);
        assert_eq!(
            formatted[0].before_sha256,
            format!("{:x}", Sha256::digest(&original))
        );
        let rewritten = fs::read(&main).unwrap();
        assert_eq!(
            formatted[0].after_sha256,
            format!("{:x}", Sha256::digest(&rewritten))
        );
        assert_eq!(
            String::from_utf8(rewritten).unwrap(),
            "mod child;\n\nfn main() {}\n"
        );
        assert_eq!(
            fs::read_to_string(&child).unwrap(),
            "fn  child() {}\n",
            "module-following must stay inside the named file"
        );
        assert_eq!(
            fs::read_to_string(&sibling).unwrap(),
            "fn  sibling() {}\n",
            "a parent module sibling must stay untouched"
        );
    }

    #[test]
    fn formatting_is_idempotent_and_records_unchanged_files() {
        if !rustfmt_available() {
            return;
        }
        let root = root_with_edition();
        source(root.path(), "src/lib.rs", "fn  main() {}\n");
        let requested = [OsString::from("src/lib.rs")];

        run(root.path(), &requested, &token()).unwrap();
        let repeated = run(root.path(), &requested, &token()).unwrap().files;

        assert!(!repeated[0].changed);
        assert_eq!(repeated[0].before_sha256, repeated[0].after_sha256);
    }

    #[test]
    fn formatting_fails_closed_for_unparsable_source() {
        if !rustfmt_available() {
            return;
        }
        let root = root_with_edition();
        let broken = source(root.path(), "src/broken.rs", "fn broken( {\n");
        let requested = [OsString::from("src/broken.rs")];

        let failure = run(root.path(), &requested, &token()).err().unwrap();

        assert_eq!(failure.failed_file.as_deref(), Some("src/broken.rs"));
        assert!(failure.files.is_empty());
        assert_eq!(fs::read_to_string(&broken).unwrap(), "fn broken( {\n");
    }

    #[test]
    fn partial_formatting_keeps_completed_file_evidence() {
        if !rustfmt_available() {
            return;
        }
        let root = root_with_edition();
        source(root.path(), "src/a.rs", "fn  a() {}\n");
        source(root.path(), "src/b.rs", "fn broken( {\n");
        let requested = [OsString::from("src/a.rs"), OsString::from("src/b.rs")];

        let failure = run(root.path(), &requested, &token()).err().unwrap();

        assert_eq!(failure.failed_file.as_deref(), Some("src/b.rs"));
        assert_eq!(failure.files.len(), 1);
        assert_eq!(failure.files[0].path, "src/a.rs");
        assert!(failure.files[0].changed);
        assert!(fs::read_to_string(root.path().join("src/a.rs"))
            .unwrap()
            .contains("fn a()"));
    }

    #[test]
    fn formatting_uses_the_owning_rustfmt_config() {
        if !rustfmt_available() {
            return;
        }
        let root = root_with_edition();
        fs::write(root.path().join("rustfmt.toml"), "hard_tabs = true\n").unwrap();
        let file = source(
            root.path(),
            "src/lib.rs",
            "fn main() {\n    if true {\n        println!(\"x\");\n    }\n}\n",
        );
        let requested = [OsString::from("src/lib.rs")];

        run(root.path(), &requested, &token()).unwrap();

        let formatted = fs::read_to_string(&file).unwrap();
        assert!(
            formatted.contains("\t"),
            "owning rustfmt config was not applied"
        );
    }

    #[test]
    fn concurrent_edits_are_refused_before_write() {
        let root = root_with_edition();
        let file = source(root.path(), "src/lib.rs", "fn  main() {}\n");
        let requested = [OsString::from("src/lib.rs")];
        let plan = FormatPlan::build(root.path(), &requested).unwrap();
        let original = fs::read(&file).unwrap();
        fs::write(&file, "fn edited_by_someone_else() {}\n").unwrap();

        let error = commit_formatted(&plan.files[0], &original, b"fn main() {}\n", &token())
            .unwrap_err()
            .to_string();

        assert!(error.contains("concurrent edit"), "got: {error}");
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            "fn edited_by_someone_else() {}\n"
        );
    }

    #[test]
    fn cancelled_writes_are_refused() {
        let root = root_with_edition();
        let file = source(root.path(), "src/lib.rs", "fn  main() {}\n");
        let requested = [OsString::from("src/lib.rs")];
        let plan = FormatPlan::build(root.path(), &requested).unwrap();
        let original = fs::read(&file).unwrap();
        let cancellation = token();
        cancellation.cancel();

        let error = commit_formatted(&plan.files[0], &original, b"fn main() {}\n", &cancellation)
            .unwrap_err()
            .to_string();

        assert!(error.contains("cancelled"), "got: {error}");
        assert_eq!(fs::read_to_string(&file).unwrap(), "fn  main() {}\n");
    }

    #[test]
    fn a_cancelled_run_writes_nothing() {
        let root = root_with_edition();
        source(root.path(), "src/a.rs", "fn  a() {}\n");
        source(root.path(), "src/b.rs", "fn  b() {}\n");
        let requested = [OsString::from("src/a.rs"), OsString::from("src/b.rs")];
        let cancellation = token();
        cancellation.cancel();

        let failure = run(root.path(), &requested, &cancellation).err().unwrap();

        assert!(failure.files.is_empty());
        assert_eq!(
            failure.failed_file.as_deref(),
            Some("src/a.rs"),
            "cancellation must be reported before any file is touched"
        );
        assert_eq!(
            fs::read_to_string(root.path().join("src/a.rs")).unwrap(),
            "fn  a() {}\n"
        );
        assert_eq!(
            fs::read_to_string(root.path().join("src/b.rs")).unwrap(),
            "fn  b() {}\n"
        );
    }
}
