use serde::Deserialize;
use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CargoArtifact {
    pub package_id: String,
    pub manifest_path: PathBuf,
    pub target_name: String,
    pub target_kind: Vec<String>,
    pub crate_types: Vec<String>,
    pub features: Vec<String>,
    pub filenames: Vec<PathBuf>,
    pub executable: Option<PathBuf>,
    pub fresh: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CargoMessage {
    Artifact(CargoArtifact),
    Diagnostic(String),
    Other,
}

#[derive(Debug)]
pub enum CargoMessageError {
    InvalidJson(serde_json::Error),
    MissingReason,
    InvalidArtifact(serde_json::Error),
}

impl fmt::Display for CargoMessageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(error) => write!(formatter, "Cargo message is invalid JSON: {error}"),
            Self::MissingReason => formatter.write_str("Cargo message has no reason"),
            Self::InvalidArtifact(error) => {
                write!(formatter, "Cargo compiler artifact is invalid: {error}")
            }
        }
    }
}

impl std::error::Error for CargoMessageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidJson(error) | Self::InvalidArtifact(error) => Some(error),
            Self::MissingReason => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CargoArtifactSelectionError {
    Missing {
        manifest_path: PathBuf,
        target_name: String,
    },
    Ambiguous {
        manifest_path: PathBuf,
        target_name: String,
        executables: Vec<PathBuf>,
    },
}

impl fmt::Display for CargoArtifactSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing {
                manifest_path,
                target_name,
            } => {
                write!(
                    formatter,
                    "Cargo did not report an executable for binary {target_name:?} from {}",
                    manifest_path.display()
                )
            }
            Self::Ambiguous {
                manifest_path,
                target_name,
                executables,
            } => write!(
                formatter,
                "Cargo reported multiple executables for binary {target_name:?} from {}: {}",
                manifest_path.display(),
                executables
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

impl std::error::Error for CargoArtifactSelectionError {}

#[derive(Deserialize)]
struct ArtifactMessage {
    package_id: String,
    manifest_path: PathBuf,
    target: ArtifactTarget,
    #[serde(default)]
    features: Vec<String>,
    #[serde(default)]
    filenames: Vec<PathBuf>,
    executable: Option<PathBuf>,
    fresh: bool,
}

#[derive(Deserialize)]
struct ArtifactTarget {
    name: String,
    #[serde(default)]
    kind: Vec<String>,
    #[serde(default)]
    crate_types: Vec<String>,
}

pub fn parse_cargo_message(line: &str) -> Result<CargoMessage, CargoMessageError> {
    let value =
        serde_json::from_str::<serde_json::Value>(line).map_err(CargoMessageError::InvalidJson)?;
    let reason = value
        .get("reason")
        .and_then(serde_json::Value::as_str)
        .ok_or(CargoMessageError::MissingReason)?;
    if reason == "compiler-artifact" {
        let message = serde_json::from_value::<ArtifactMessage>(value)
            .map_err(CargoMessageError::InvalidArtifact)?;
        return Ok(CargoMessage::Artifact(CargoArtifact {
            package_id: message.package_id,
            manifest_path: message.manifest_path,
            target_name: message.target.name,
            target_kind: message.target.kind,
            crate_types: message.target.crate_types,
            features: message.features,
            filenames: message.filenames,
            executable: message.executable,
            fresh: message.fresh,
        }));
    }
    if reason == "compiler-message" {
        let rendered = value
            .get("message")
            .and_then(|message| message.get("rendered"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        return Ok(CargoMessage::Diagnostic(rendered));
    }
    Ok(CargoMessage::Other)
}

pub fn select_binary_executable(
    artifacts: &[CargoArtifact],
    manifest_path: &Path,
    target_name: &str,
) -> Result<PathBuf, CargoArtifactSelectionError> {
    let expected_manifest = comparable_path(manifest_path);
    let executables = artifacts
        .iter()
        .filter(|artifact| comparable_path(&artifact.manifest_path) == expected_manifest)
        .filter(|artifact| artifact.target_name == target_name)
        .filter(|artifact| artifact.target_kind.iter().any(|kind| kind == "bin"))
        .filter_map(|artifact| artifact.executable.as_deref())
        .collect::<BTreeSet<&Path>>()
        .into_iter()
        .map(Path::to_path_buf)
        .collect::<Vec<_>>();
    match executables.as_slice() {
        [] => Err(CargoArtifactSelectionError::Missing {
            manifest_path: manifest_path.to_path_buf(),
            target_name: target_name.to_string(),
        }),
        [executable] => Ok(executable.clone()),
        _ => Err(CargoArtifactSelectionError::Ambiguous {
            manifest_path: manifest_path.to_path_buf(),
            target_name: target_name.to_string(),
            executables,
        }),
    }
}

fn comparable_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::{
        parse_cargo_message, select_binary_executable, CargoArtifact, CargoArtifactSelectionError,
        CargoMessage, CargoMessageError,
    };
    use std::path::{Path, PathBuf};

    fn artifact(
        manifest_path: &str,
        name: &str,
        kind: &[&str],
        executable: Option<&str>,
    ) -> CargoArtifact {
        CargoArtifact {
            package_id: "path+file:///repo#fixture@1.0.0".to_string(),
            manifest_path: PathBuf::from(manifest_path),
            target_name: name.to_string(),
            target_kind: kind.iter().map(|value| (*value).to_string()).collect(),
            crate_types: kind.iter().map(|value| (*value).to_string()).collect(),
            features: Vec::new(),
            filenames: Vec::new(),
            executable: executable.map(PathBuf::from),
            fresh: false,
        }
    }

    #[test]
    fn parses_the_exact_compiler_artifact_contract() {
        let message = parse_cargo_message(
            r#"{"reason":"compiler-artifact","package_id":"path+file:///repo#qol-tray@3.40.6","manifest_path":"/repo/apps/qol-tray/Cargo.toml","target":{"kind":["bin"],"crate_types":["bin"],"name":"qol-tray-install","src_path":"/repo/apps/qol-tray/src/installer/main.rs","edition":"2021","doc":true,"doctest":false,"test":true},"profile":{"opt_level":"3","debuginfo":0,"debug_assertions":false,"overflow_checks":false,"test":false},"features":["default"],"filenames":["/repo/target/release/qol-tray-install"],"executable":"/repo/target/release/qol-tray-install","fresh":true}"#,
        )
        .unwrap();

        let CargoMessage::Artifact(artifact) = message else {
            panic!("expected compiler artifact");
        };
        assert_eq!(
            artifact.manifest_path,
            PathBuf::from("/repo/apps/qol-tray/Cargo.toml")
        );
        assert_eq!(artifact.target_name, "qol-tray-install");
        assert_eq!(artifact.target_kind, ["bin"]);
        assert_eq!(artifact.features, ["default"]);
        assert_eq!(
            artifact.executable,
            Some(PathBuf::from("/repo/target/release/qol-tray-install"))
        );
        assert!(artifact.fresh);
    }

    #[test]
    fn parses_diagnostics_and_ignores_other_cargo_messages() {
        assert_eq!(
            parse_cargo_message(
                r#"{"reason":"compiler-message","message":{"rendered":"warning: fixture\n"}}"#,
            )
            .unwrap(),
            CargoMessage::Diagnostic("warning: fixture\n".to_string())
        );
        assert_eq!(
            parse_cargo_message(r#"{"reason":"build-finished","success":true}"#).unwrap(),
            CargoMessage::Other
        );
    }

    #[test]
    fn malformed_compiler_artifacts_fail_closed() {
        assert!(matches!(
            parse_cargo_message(r#"{"reason":"compiler-artifact"}"#),
            Err(CargoMessageError::InvalidArtifact(_))
        ));
        assert!(matches!(
            parse_cargo_message(r#"{"success":true}"#),
            Err(CargoMessageError::MissingReason)
        ));
    }

    #[test]
    fn executable_selection_requires_one_unique_binary() {
        let artifacts = [
            artifact(
                "/repo/Cargo.toml",
                "fixture",
                &["lib"],
                Some("/target/libfixture.rlib"),
            ),
            artifact(
                "/repo/Cargo.toml",
                "fixture",
                &["bin"],
                Some("/target/fixture"),
            ),
            artifact(
                "/repo/Cargo.toml",
                "fixture",
                &["bin"],
                Some("/target/fixture"),
            ),
            artifact(
                "/other/Cargo.toml",
                "fixture",
                &["bin"],
                Some("/target/impostor"),
            ),
        ];
        assert_eq!(
            select_binary_executable(&artifacts, Path::new("/repo/Cargo.toml"), "fixture").unwrap(),
            PathBuf::from("/target/fixture")
        );
        assert!(matches!(
            select_binary_executable(&artifacts, Path::new("/repo/Cargo.toml"), "missing"),
            Err(CargoArtifactSelectionError::Missing { .. })
        ));

        let ambiguous = [
            artifact(
                "/repo/Cargo.toml",
                "fixture",
                &["bin"],
                Some("/target/a/fixture"),
            ),
            artifact(
                "/repo/Cargo.toml",
                "fixture",
                &["bin"],
                Some("/target/b/fixture"),
            ),
        ];
        assert!(matches!(
            select_binary_executable(&ambiguous, Path::new("/repo/Cargo.toml"), "fixture"),
            Err(CargoArtifactSelectionError::Ambiguous { .. })
        ));
    }
}
