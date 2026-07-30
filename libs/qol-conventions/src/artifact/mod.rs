mod platform;

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;
use std::sync::OnceLock;

pub const SCHEMA_VERSION: u16 = 1;
pub const FRAME_MAGIC: &str = "QOL_BUILD_INFO_V1:";
pub const NATIVE_SECTION_NAME: &str = ".qolbi";
pub const ELF_SECTION_NAME: &str = NATIVE_SECTION_NAME;
pub const MACHO_SECTION_NAME: &str = "__qolbi";
pub const PE_SECTION_NAME: &str = NATIVE_SECTION_NAME;
pub const DEV_FEATURE_NAME: &str = "dev";
pub const TRAY_PACKAGE_NAME: &str = "qol-tray";
pub const TRAY_HOST_BINARY_NAME: &str = "qol-tray";
pub const TRAY_INSTALLER_BINARY_NAME: &str = "qol-tray-install";
pub const TRAY_DOCTOR_BINARY_NAME: &str = "qol-tray-doctor";
pub const TRAY_MIGRATOR_BINARY_NAME: &str = "qol-tray-migrate";

pub const ENV_BUILD_INTENT: &str = "QOL_BUILD_INTENT";
pub const ENV_SOURCE_COMMIT: &str = "QOL_BUILD_SOURCE_COMMIT";
pub const ENV_SOURCE_HEAD_TREE: &str = "QOL_BUILD_SOURCE_HEAD_TREE";
pub const ENV_SOURCE_WORKING_TREE: &str = "QOL_BUILD_SOURCE_WORKING_TREE";
pub const ENV_COMPILER_OVERFLOW_CHECKS: &str = "QOL_BUILD_COMPILER_OVERFLOW_CHECKS";

#[doc(hidden)]
pub const ENV_FRAME_FIELDS: &str = "QOL_BUILD_INFO_FIELDS";
#[doc(hidden)]
pub const ENV_FRAME_PREFIX: &str = "QOL_BUILD_INFO_PREFIX";
#[doc(hidden)]
pub const ENV_LINK_SECTION: &str = "QOL_BUILD_INFO_LINK_SECTION";

static CURRENT_IDENTITY: OnceLock<BuildIdentity> = OnceLock::new();

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BuildIntent {
    Production,
    Development,
    Sandbox,
    Unspecified,
}

impl BuildIntent {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Development => "development",
            Self::Sandbox => "sandbox",
            Self::Unspecified => "unspecified",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BuildProfile {
    Release,
    Debug,
    Sandbox,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BuildFlavor {
    pub profile: BuildProfile,
    pub dev_features: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BuildRole {
    Host,
    Installer,
    Doctor,
    Migrator,
    Cli,
    Plugin,
    GuestRunner,
}

pub fn tray_binary_role(binary: &str) -> Option<BuildRole> {
    match binary {
        TRAY_HOST_BINARY_NAME => Some(BuildRole::Host),
        TRAY_INSTALLER_BINARY_NAME => Some(BuildRole::Installer),
        TRAY_DOCTOR_BINARY_NAME => Some(BuildRole::Doctor),
        TRAY_MIGRATOR_BINARY_NAME => Some(BuildRole::Migrator),
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CompilerFacts {
    pub cargo_profile: String,
    pub opt_level: String,
    pub debuginfo: bool,
    pub debug_assertions: bool,
    pub overflow_checks: Option<bool>,
    pub test: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildFlavorError {
    UnsupportedCargoProfile,
    DevFeatureMismatch,
    SandboxIntentMismatch,
    ProductionRequiresRelease,
    ProductionForbidsDevFeatures,
    ReleaseCompilerMismatch,
    DebugCompilerMismatch,
}

impl BuildFlavorError {
    pub const fn reason(self) -> &'static str {
        match self {
            Self::UnsupportedCargoProfile => "Cargo profile has no artifact flavor",
            Self::DevFeatureMismatch => "flavor dev_features disagrees with Cargo features",
            Self::SandboxIntentMismatch => "sandbox intent and profile disagree",
            Self::ProductionRequiresRelease => "production intent requires release profile",
            Self::ProductionForbidsDevFeatures => "production intent forbids dev features",
            Self::ReleaseCompilerMismatch => "release flavor disagrees with compiler facts",
            Self::DebugCompilerMismatch => "debug flavor disagrees with compiler facts",
        }
    }
}

impl fmt::Display for BuildFlavorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.reason())
    }
}

impl std::error::Error for BuildFlavorError {}

impl BuildFlavor {
    pub fn derive(
        intent: BuildIntent,
        compiler: &CompilerFacts,
        features: &[String],
    ) -> Result<Self, BuildFlavorError> {
        let profile = if intent == BuildIntent::Sandbox {
            BuildProfile::Sandbox
        } else {
            match compiler.cargo_profile.as_str() {
                "release" => BuildProfile::Release,
                "debug" => BuildProfile::Debug,
                _ => return Err(BuildFlavorError::UnsupportedCargoProfile),
            }
        };
        let flavor = Self {
            profile,
            dev_features: has_dev_feature(features),
        };
        flavor.validate(intent, compiler, features)?;
        Ok(flavor)
    }

    pub fn validate(
        self,
        intent: BuildIntent,
        compiler: &CompilerFacts,
        features: &[String],
    ) -> Result<(), BuildFlavorError> {
        if self.dev_features != has_dev_feature(features) {
            return Err(BuildFlavorError::DevFeatureMismatch);
        }
        match (intent, self.profile) {
            (BuildIntent::Sandbox, BuildProfile::Sandbox) => {}
            (BuildIntent::Sandbox, _) | (_, BuildProfile::Sandbox) => {
                return Err(BuildFlavorError::SandboxIntentMismatch);
            }
            (BuildIntent::Production, BuildProfile::Debug) => {
                return Err(BuildFlavorError::ProductionRequiresRelease);
            }
            _ => {}
        }
        if intent == BuildIntent::Production && self.dev_features {
            return Err(BuildFlavorError::ProductionForbidsDevFeatures);
        }
        match self.profile {
            BuildProfile::Release
                if compiler.cargo_profile != "release"
                    || compiler.debug_assertions
                    || compiler.opt_level == "0" =>
            {
                Err(BuildFlavorError::ReleaseCompilerMismatch)
            }
            BuildProfile::Debug
                if compiler.cargo_profile != "debug" || !compiler.debug_assertions =>
            {
                Err(BuildFlavorError::DebugCompilerMismatch)
            }
            BuildProfile::Release | BuildProfile::Debug | BuildProfile::Sandbox => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceIdentity {
    Unspecified,
    Git {
        commit: String,
        head_tree: String,
        working_tree: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BuildIdentity {
    pub schema: u16,
    pub binary: String,
    pub role: BuildRole,
    pub package: String,
    pub version: String,
    pub target: String,
    pub intent: BuildIntent,
    pub flavor: BuildFlavor,
    pub compiler: CompilerFacts,
    pub features: Vec<String>,
    pub source: SourceIdentity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RunningBuildInfo {
    pub identity: BuildIdentity,
    pub executable: PathBuf,
}

#[derive(Debug)]
pub enum DecodeError {
    MissingMagic,
    InvalidJson(serde_json::Error),
    UnsupportedSchema(u16),
    InvalidContract(&'static str),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingMagic => formatter.write_str("artifact identity magic is missing"),
            Self::InvalidJson(error) => {
                write!(formatter, "artifact identity JSON is invalid: {error}")
            }
            Self::UnsupportedSchema(schema) => {
                write!(
                    formatter,
                    "artifact identity schema {schema} is unsupported"
                )
            }
            Self::InvalidContract(reason) => {
                write!(formatter, "artifact identity contract is invalid: {reason}")
            }
        }
    }
}

impl std::error::Error for DecodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidJson(error) => Some(error),
            Self::MissingMagic | Self::UnsupportedSchema(_) | Self::InvalidContract(_) => None,
        }
    }
}

pub fn decode_frame(frame: &[u8]) -> Result<BuildIdentity, DecodeError> {
    let json = frame
        .strip_prefix(FRAME_MAGIC.as_bytes())
        .ok_or(DecodeError::MissingMagic)?;
    let identity =
        serde_json::from_slice::<BuildIdentity>(json).map_err(DecodeError::InvalidJson)?;
    if identity.schema != SCHEMA_VERSION {
        return Err(DecodeError::UnsupportedSchema(identity.schema));
    }
    validate_identity(&identity)?;
    Ok(identity)
}

pub fn validate_identity(identity: &BuildIdentity) -> Result<(), DecodeError> {
    let required = [
        ("binary is empty", identity.binary.as_str()),
        ("package is empty", identity.package.as_str()),
        ("version is empty", identity.version.as_str()),
        ("target is empty", identity.target.as_str()),
        (
            "Cargo profile is empty",
            identity.compiler.cargo_profile.as_str(),
        ),
        (
            "optimization level is empty",
            identity.compiler.opt_level.as_str(),
        ),
    ];
    for (reason, value) in required {
        if value.is_empty() {
            return Err(DecodeError::InvalidContract(reason));
        }
    }
    if identity.features.iter().any(|feature| feature.is_empty()) {
        return Err(DecodeError::InvalidContract("feature name is empty"));
    }
    if identity.features.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(DecodeError::InvalidContract(
            "features are not sorted and unique",
        ));
    }
    identity
        .flavor
        .validate(identity.intent, &identity.compiler, &identity.features)
        .map_err(|error| DecodeError::InvalidContract(error.reason()))?;
    if let SourceIdentity::Git {
        commit,
        head_tree,
        working_tree,
    } = &identity.source
    {
        if !valid_git_oid(commit) || !valid_git_oid(head_tree) || !valid_git_oid(working_tree) {
            return Err(DecodeError::InvalidContract("Git object id is invalid"));
        }
    }
    Ok(())
}

fn has_dev_feature(features: &[String]) -> bool {
    features
        .binary_search_by(|feature| feature.as_str().cmp(DEV_FEATURE_NAME))
        .is_ok()
}

fn valid_git_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub fn register_current(frame: &'static str) -> &'static BuildIdentity {
    let identity = decode_frame(frame.as_bytes())
        .unwrap_or_else(|error| panic!("generated artifact identity is invalid: {error}"));
    if let Some(current) = CURRENT_IDENTITY.get() {
        assert_eq!(
            current, &identity,
            "a different artifact identity is already registered"
        );
        return current;
    }
    let _ = CURRENT_IDENTITY.set(identity);
    CURRENT_IDENTITY
        .get()
        .expect("artifact identity was registered")
}

pub fn current() -> Option<&'static BuildIdentity> {
    CURRENT_IDENTITY.get()
}

#[doc(hidden)]
pub const fn frame_bytes<const LENGTH: usize>(frame: &str) -> [u8; LENGTH] {
    let source = frame.as_bytes();
    let mut result = [0; LENGTH];
    let mut index = 0;
    while index < LENGTH {
        result[index] = source[index];
        index += 1;
    }
    result
}

#[doc(hidden)]
pub fn link_section_for_target(target_os: &str) -> String {
    platform::link_section(target_os)
}

#[macro_export]
macro_rules! declare_build_identity {
    (Host) => {
        $crate::declare_build_identity!(@emit "host");
    };
    (Installer) => {
        $crate::declare_build_identity!(@emit "installer");
    };
    (Doctor) => {
        $crate::declare_build_identity!(@emit "doctor");
    };
    (Migrator) => {
        $crate::declare_build_identity!(@emit "migrator");
    };
    (Cli) => {
        $crate::declare_build_identity!(@emit "cli");
    };
    (Plugin) => {
        $crate::declare_build_identity!(@emit "plugin");
    };
    (GuestRunner) => {
        $crate::declare_build_identity!(@emit "guest_runner");
    };
    (@emit $role:literal) => {
        const QOL_BUILD_IDENTITY_FRAME: &str = concat!(
            env!("QOL_BUILD_INFO_PREFIX"),
            env!("CARGO_BIN_NAME"),
            "\",\"role\":\"",
            $role,
            "\",",
            env!("QOL_BUILD_INFO_FIELDS"),
            "}"
        );

        #[used]
        #[link_section = env!("QOL_BUILD_INFO_LINK_SECTION")]
        static QOL_BUILD_IDENTITY_SECTION: [u8; QOL_BUILD_IDENTITY_FRAME.len()] =
            $crate::artifact::frame_bytes::<{ QOL_BUILD_IDENTITY_FRAME.len() }>(
                QOL_BUILD_IDENTITY_FRAME,
            );

        fn register_build_identity() -> &'static $crate::artifact::BuildIdentity {
            let _ = std::hint::black_box(&QOL_BUILD_IDENTITY_SECTION);
            $crate::artifact::register_current(QOL_BUILD_IDENTITY_FRAME)
        }
    };
}

#[cfg(test)]
mod tests {
    use super::{
        decode_frame, BuildFlavor, BuildIdentity, BuildIntent, BuildProfile, BuildRole,
        CompilerFacts, DecodeError, SourceIdentity, FRAME_MAGIC, SCHEMA_VERSION,
    };
    use proptest::prelude::*;

    fn identity() -> BuildIdentity {
        BuildIdentity {
            schema: SCHEMA_VERSION,
            binary: "foo".to_string(),
            role: BuildRole::Host,
            package: "foo-package".to_string(),
            version: "1.2.3".to_string(),
            target: "x86_64-unknown-linux-gnu".to_string(),
            intent: BuildIntent::Sandbox,
            flavor: BuildFlavor {
                profile: BuildProfile::Sandbox,
                dev_features: true,
            },
            compiler: CompilerFacts {
                cargo_profile: "release".to_string(),
                opt_level: "3".to_string(),
                debuginfo: false,
                debug_assertions: true,
                overflow_checks: Some(false),
                test: false,
            },
            features: vec!["dev".to_string(), "linux_evdev".to_string()],
            source: SourceIdentity::Git {
                commit: "a".repeat(40),
                head_tree: "b".repeat(40),
                working_tree: "c".repeat(40),
            },
        }
    }

    fn frame(identity: &BuildIdentity) -> Vec<u8> {
        let mut frame = FRAME_MAGIC.as_bytes().to_vec();
        frame.extend(serde_json::to_vec(identity).unwrap());
        frame
    }

    #[test]
    fn frame_round_trips_the_typed_contract() {
        let expected = identity();
        assert_eq!(decode_frame(&frame(&expected)).unwrap(), expected);
    }

    #[test]
    fn frame_rejects_unknown_schema() {
        let mut identity = identity();
        identity.schema = SCHEMA_VERSION + 1;
        assert!(matches!(
            decode_frame(&frame(&identity)),
            Err(DecodeError::UnsupportedSchema(schema)) if schema == SCHEMA_VERSION + 1
        ));
    }

    #[test]
    fn identity_json_shape_is_stable() {
        assert_eq!(
            serde_json::to_value(identity()).unwrap(),
            serde_json::json!({
                "schema": 1,
                "binary": "foo",
                "role": "host",
                "package": "foo-package",
                "version": "1.2.3",
                "target": "x86_64-unknown-linux-gnu",
                "intent": "sandbox",
                "flavor": {
                    "profile": "sandbox",
                    "dev_features": true
                },
                "compiler": {
                    "cargo_profile": "release",
                    "opt_level": "3",
                    "debuginfo": false,
                    "debug_assertions": true,
                    "overflow_checks": false,
                    "test": false
                },
                "features": ["dev", "linux_evdev"],
                "source": {
                    "kind": "git",
                    "commit": "a".repeat(40),
                    "head_tree": "b".repeat(40),
                    "working_tree": "c".repeat(40)
                }
            })
        );
    }

    #[test]
    fn frame_rejects_unsorted_features_and_unknown_fields() {
        let mut unsorted = identity();
        unsorted.features.reverse();
        assert!(matches!(
            decode_frame(&frame(&unsorted)),
            Err(DecodeError::InvalidContract(
                "features are not sorted and unique"
            ))
        ));

        let mut value = serde_json::to_value(identity()).unwrap();
        value["unexpected"] = serde_json::Value::Bool(true);
        let mut unknown = FRAME_MAGIC.as_bytes().to_vec();
        unknown.extend(serde_json::to_vec(&value).unwrap());
        assert!(matches!(
            decode_frame(&unknown),
            Err(DecodeError::InvalidJson(_))
        ));
    }

    #[test]
    fn frame_rejects_flavor_that_disagrees_with_build_facts() {
        let mut feature_mismatch = identity();
        feature_mismatch.flavor.dev_features = false;
        assert!(matches!(
            decode_frame(&frame(&feature_mismatch)),
            Err(DecodeError::InvalidContract(
                "flavor dev_features disagrees with Cargo features"
            ))
        ));

        let mut production_debug = identity();
        production_debug.intent = BuildIntent::Production;
        production_debug.flavor.profile = BuildProfile::Debug;
        production_debug.compiler.cargo_profile = "debug".to_string();
        assert!(matches!(
            decode_frame(&frame(&production_debug)),
            Err(DecodeError::InvalidContract(
                "production intent requires release profile"
            ))
        ));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn arbitrary_frames_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..2048)) {
            let _ = decode_frame(&bytes);
        }
    }
}
