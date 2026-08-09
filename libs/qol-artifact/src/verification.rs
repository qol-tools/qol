use crate::{inspect_path, InspectedArtifact, InspectionError};
use qol_conventions::artifact::{
    BuildFlavor, BuildIdentity, BuildIntent, BuildProfile, BuildRole, DecodeError, SourceIdentity,
};
use std::fmt;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactExpectation {
    binary: String,
    package: String,
    role: BuildRole,
    intent: BuildIntent,
    flavor: BuildFlavor,
    version: Option<String>,
    target: Option<TargetExpectation>,
    exact_source: Option<SourceIdentity>,
    require_clean_source: bool,
    required_features: Vec<String>,
    forbidden_features: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TargetExpectation {
    Exact(String),
    Compatible(String),
}

impl ArtifactExpectation {
    pub fn production(binary: &str, package: &str, role: BuildRole) -> Self {
        Self {
            binary: binary.to_string(),
            package: package.to_string(),
            role,
            intent: BuildIntent::Production,
            flavor: BuildFlavor {
                profile: BuildProfile::Release,
                dev_features: false,
            },
            version: None,
            target: None,
            exact_source: None,
            require_clean_source: true,
            required_features: Vec::new(),
            forbidden_features: vec![
                qol_conventions::artifact::DEV_FEATURE_NAME.to_string(),
                qol_conventions::artifact::SANDBOX_FEATURE_NAME.to_string(),
            ],
        }
    }

    pub fn development_debug(
        binary: &str,
        package: &str,
        role: BuildRole,
        dev_features: bool,
    ) -> Self {
        Self {
            binary: binary.to_string(),
            package: package.to_string(),
            role,
            intent: BuildIntent::Development,
            flavor: BuildFlavor {
                profile: BuildProfile::Debug,
                dev_features,
            },
            version: None,
            target: None,
            exact_source: None,
            require_clean_source: false,
            required_features: Vec::new(),
            forbidden_features: Vec::new(),
        }
    }

    pub fn sandbox_debug(binary: &str, package: &str, role: BuildRole) -> Self {
        Self {
            binary: binary.to_string(),
            package: package.to_string(),
            role,
            intent: BuildIntent::Sandbox,
            flavor: BuildFlavor {
                profile: BuildProfile::Sandbox,
                dev_features: false,
            },
            version: None,
            target: None,
            exact_source: None,
            require_clean_source: false,
            required_features: vec![qol_conventions::artifact::SANDBOX_FEATURE_NAME.to_string()],
            forbidden_features: Vec::new(),
        }
    }

    pub fn development_release(
        binary: &str,
        package: &str,
        role: BuildRole,
        dev_features: bool,
    ) -> Self {
        Self {
            binary: binary.to_string(),
            package: package.to_string(),
            role,
            intent: BuildIntent::Development,
            flavor: BuildFlavor {
                profile: BuildProfile::Release,
                dev_features,
            },
            version: None,
            target: None,
            exact_source: None,
            require_clean_source: false,
            required_features: Vec::new(),
            forbidden_features: Vec::new(),
        }
    }

    pub fn with_version(mut self, version: &str) -> Self {
        self.version = Some(version.to_string());
        self
    }

    pub fn with_exact_target(mut self, target: &str) -> Self {
        self.target = Some(TargetExpectation::Exact(target.to_string()));
        self
    }

    pub fn with_compatible_target(mut self, target: &str) -> Self {
        self.target = Some(TargetExpectation::Compatible(target.to_string()));
        self
    }

    pub fn with_exact_source(mut self, source: &SourceIdentity) -> Self {
        self.exact_source = Some(source.clone());
        self
    }

    pub fn require_feature(mut self, feature: &str) -> Self {
        insert_sorted_unique(&mut self.required_features, feature);
        self
    }
}

#[derive(Debug)]
pub enum VerificationError {
    Inspection(InspectionError),
    InvalidIdentity(DecodeError),
    FieldMismatch {
        field: &'static str,
        expected: String,
        actual: String,
    },
    SourceUnspecified,
    SourceDirty,
    SourceMismatch,
    InvalidTargetPlatform(String),
    TargetPlatformMismatch {
        expected: String,
        actual: String,
    },
    MissingFeature(String),
    ForbiddenFeature(String),
}

impl fmt::Display for VerificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Inspection(error) => write!(formatter, "artifact inspection failed: {error}"),
            Self::InvalidIdentity(error) => {
                write!(formatter, "artifact identity contract is invalid: {error}")
            }
            Self::FieldMismatch {
                field,
                expected,
                actual,
            } => write!(
                formatter,
                "artifact {field} mismatch: expected {expected:?}, got {actual:?}"
            ),
            Self::SourceUnspecified => {
                formatter.write_str("artifact source identity is unspecified")
            }
            Self::SourceDirty => formatter.write_str("artifact source working tree is dirty"),
            Self::SourceMismatch => {
                formatter.write_str("artifact source identity does not match the build")
            }
            Self::InvalidTargetPlatform(error) => {
                write!(formatter, "artifact target platform is invalid: {error}")
            }
            Self::TargetPlatformMismatch { expected, actual } => write!(
                formatter,
                "artifact target platform mismatch: expected compatibility with {expected:?}, got {actual:?}"
            ),
            Self::MissingFeature(feature) => {
                write!(
                    formatter,
                    "artifact is missing required feature {feature:?}"
                )
            }
            Self::ForbiddenFeature(feature) => {
                write!(formatter, "artifact contains forbidden feature {feature:?}")
            }
        }
    }
}

impl std::error::Error for VerificationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Inspection(error) => Some(error),
            Self::InvalidIdentity(error) => Some(error),
            Self::FieldMismatch { .. }
            | Self::SourceUnspecified
            | Self::SourceDirty
            | Self::SourceMismatch
            | Self::InvalidTargetPlatform(_)
            | Self::TargetPlatformMismatch { .. }
            | Self::MissingFeature(_)
            | Self::ForbiddenFeature(_) => None,
        }
    }
}

impl From<InspectionError> for VerificationError {
    fn from(error: InspectionError) -> Self {
        Self::Inspection(error)
    }
}

pub fn verify_path(
    path: impl AsRef<Path>,
    expectation: &ArtifactExpectation,
) -> Result<InspectedArtifact, VerificationError> {
    let artifact = inspect_path(path)?;
    for slice in &artifact.slices {
        verify_identity(&slice.identity, expectation)?;
    }
    Ok(artifact)
}

pub fn verify_identity(
    identity: &BuildIdentity,
    expectation: &ArtifactExpectation,
) -> Result<(), VerificationError> {
    qol_conventions::artifact::validate_identity(identity)
        .map_err(VerificationError::InvalidIdentity)?;
    require_field("binary", &expectation.binary, &identity.binary)?;
    require_field("package", &expectation.package, &identity.package)?;
    require_field(
        "role",
        &format!("{:?}", expectation.role),
        &format!("{:?}", identity.role),
    )?;
    require_field(
        "intent",
        expectation.intent.as_str(),
        identity.intent.as_str(),
    )?;
    require_field(
        "flavor",
        &format!("{:?}", expectation.flavor),
        &format!("{:?}", identity.flavor),
    )?;
    if let Some(version) = &expectation.version {
        require_field("version", version, &identity.version)?;
    }
    if let Some(target) = &expectation.target {
        match target {
            TargetExpectation::Exact(expected) => {
                require_field("target", expected, &identity.target)?;
            }
            TargetExpectation::Compatible(expected) => {
                let compatible = crate::target::same_platform(expected, &identity.target)
                    .map_err(VerificationError::InvalidTargetPlatform)?;
                if !compatible {
                    return Err(VerificationError::TargetPlatformMismatch {
                        expected: expected.clone(),
                        actual: identity.target.clone(),
                    });
                }
            }
        }
    }
    require_field(
        "compiler test mode",
        "false",
        if identity.compiler.test {
            "true"
        } else {
            "false"
        },
    )?;

    let SourceIdentity::Git {
        head_tree,
        working_tree,
        ..
    } = &identity.source
    else {
        return Err(VerificationError::SourceUnspecified);
    };
    if expectation.require_clean_source && head_tree != working_tree {
        return Err(VerificationError::SourceDirty);
    }
    if expectation
        .exact_source
        .as_ref()
        .is_some_and(|source| source != &identity.source)
    {
        return Err(VerificationError::SourceMismatch);
    }
    for feature in &expectation.required_features {
        if identity.features.binary_search(feature).is_err() {
            return Err(VerificationError::MissingFeature(feature.clone()));
        }
    }
    for feature in &expectation.forbidden_features {
        if identity.features.binary_search(feature).is_ok() {
            return Err(VerificationError::ForbiddenFeature(feature.clone()));
        }
    }
    Ok(())
}

fn require_field(
    field: &'static str,
    expected: &str,
    actual: &str,
) -> Result<(), VerificationError> {
    if expected == actual {
        return Ok(());
    }
    Err(VerificationError::FieldMismatch {
        field,
        expected: expected.to_string(),
        actual: actual.to_string(),
    })
}

fn insert_sorted_unique(values: &mut Vec<String>, value: &str) {
    match values.binary_search_by(|candidate| candidate.as_str().cmp(value)) {
        Ok(_) => {}
        Err(index) => values.insert(index, value.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{verify_identity, ArtifactExpectation, VerificationError};
    use qol_conventions::artifact::{
        BuildFlavor, BuildIdentity, BuildIntent, BuildProfile, BuildRole, CompilerFacts,
        SourceIdentity, SCHEMA_VERSION,
    };

    fn identity() -> BuildIdentity {
        BuildIdentity {
            schema: SCHEMA_VERSION,
            binary: "qol-tray".to_string(),
            role: BuildRole::Host,
            package: "qol-tray".to_string(),
            version: "3.40.6".to_string(),
            target: "x86_64-unknown-linux-gnu".to_string(),
            intent: BuildIntent::Production,
            flavor: BuildFlavor {
                profile: BuildProfile::Release,
                dev_features: false,
            },
            compiler: CompilerFacts {
                cargo_profile: "release".to_string(),
                opt_level: "3".to_string(),
                debuginfo: false,
                debug_assertions: false,
                overflow_checks: None,
                test: false,
            },
            features: vec!["default".to_string()],
            source: SourceIdentity::Git {
                commit: "a".repeat(40),
                head_tree: "b".repeat(40),
                working_tree: "b".repeat(40),
            },
        }
    }

    fn expectation() -> ArtifactExpectation {
        ArtifactExpectation::production("qol-tray", "qol-tray", BuildRole::Host)
            .with_version("3.40.6")
    }

    #[test]
    fn production_policy_accepts_only_the_expected_clean_artifact() {
        verify_identity(&identity(), &expectation()).unwrap();

        let mut dirty = identity();
        dirty.source = SourceIdentity::Git {
            commit: "a".repeat(40),
            head_tree: "b".repeat(40),
            working_tree: "c".repeat(40),
        };
        assert!(matches!(
            verify_identity(&dirty, &expectation()),
            Err(VerificationError::SourceDirty)
        ));

        let mut development = identity();
        development.intent = BuildIntent::Development;
        assert!(matches!(
            verify_identity(&development, &expectation()),
            Err(VerificationError::FieldMismatch {
                field: "intent",
                ..
            })
        ));
    }

    #[test]
    fn production_policy_rejects_dev_features_and_unspecified_sources() {
        let mut dev_feature = identity();
        dev_feature.features.push("dev".to_string());
        dev_feature.features.sort();
        assert!(matches!(
            verify_identity(&dev_feature, &expectation()),
            Err(VerificationError::InvalidIdentity(_))
        ));

        let mut unspecified = identity();
        unspecified.source = SourceIdentity::Unspecified;
        assert!(matches!(
            verify_identity(&unspecified, &expectation()),
            Err(VerificationError::SourceUnspecified)
        ));
    }

    #[test]
    fn production_policy_rejects_the_sandbox_feature() {
        let mut sandbox_feature = identity();
        sandbox_feature.features.push("sandbox".to_string());
        sandbox_feature.features.sort();
        assert!(matches!(
            verify_identity(&sandbox_feature, &expectation()),
            Err(VerificationError::InvalidIdentity(_))
        ));
    }

    #[test]
    fn declared_production_intent_cannot_mask_debug_compiler_facts() {
        let mut debug = identity();
        debug.compiler.cargo_profile = "debug".to_string();
        debug.compiler.opt_level = "0".to_string();
        debug.compiler.debug_assertions = true;

        assert!(matches!(
            verify_identity(&debug, &expectation()),
            Err(VerificationError::InvalidIdentity(_))
        ));
    }

    #[test]
    fn exact_source_and_required_features_are_additive_constraints() {
        let expected_source = identity().source;
        let expectation =
            ArtifactExpectation::development_debug("qol-tray", "qol-tray", BuildRole::Host, true)
                .with_exact_source(&expected_source)
                .require_feature("dev");
        let mut development = identity();
        development.intent = BuildIntent::Development;
        development.flavor = BuildFlavor {
            profile: BuildProfile::Debug,
            dev_features: true,
        };
        development.compiler.cargo_profile = "debug".to_string();
        development.compiler.opt_level = "0".to_string();
        development.compiler.debug_assertions = true;
        development.features.push("dev".to_string());
        development.features.sort();
        verify_identity(&development, &expectation).unwrap();

        development.source = SourceIdentity::Git {
            commit: "c".repeat(40),
            head_tree: "d".repeat(40),
            working_tree: "d".repeat(40),
        };
        assert!(matches!(
            verify_identity(&development, &expectation),
            Err(VerificationError::SourceMismatch)
        ));
    }

    #[test]
    fn sandbox_debug_requires_the_sandbox_feature() {
        let expectation =
            ArtifactExpectation::sandbox_debug("qol-tray", "qol-tray", BuildRole::Host);
        let mut sandboxed = identity();
        sandboxed.intent = BuildIntent::Sandbox;
        sandboxed.flavor = BuildFlavor {
            profile: BuildProfile::Sandbox,
            dev_features: false,
        };
        sandboxed.features.push("sandbox".to_string());
        sandboxed.features.sort();
        verify_identity(&sandboxed, &expectation).unwrap();

        let mut missing = identity();
        missing.intent = BuildIntent::Sandbox;
        missing.flavor = BuildFlavor {
            profile: BuildProfile::Sandbox,
            dev_features: false,
        };
        assert!(matches!(
            verify_identity(&missing, &expectation),
            Err(VerificationError::MissingFeature(feature)) if feature == "sandbox"
        ));
    }

    #[test]
    fn optimized_development_is_distinct_from_production() {
        let expected_source = identity().source;
        let expectation =
            ArtifactExpectation::development_release("qol-tray", "qol-tray", BuildRole::Host, true)
                .with_exact_source(&expected_source);
        let mut development = identity();
        development.intent = BuildIntent::Development;
        development.flavor.dev_features = true;
        development.features.push("dev".to_string());
        development.features.sort();

        verify_identity(&development, &expectation).unwrap();
    }

    #[test]
    fn compatible_target_allows_architecture_changes_but_not_platform_changes() {
        let expectation = expectation().with_compatible_target("aarch64-unknown-linux-gnu");
        verify_identity(&identity(), &expectation).unwrap();

        let mut windows = identity();
        windows.target = "x86_64-pc-windows-msvc".to_string();
        assert!(matches!(
            verify_identity(&windows, &expectation),
            Err(VerificationError::TargetPlatformMismatch { .. })
        ));
    }
}
