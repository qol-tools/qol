mod platform;

use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::OnceLock;

pub const SCHEMA_VERSION: u16 = 1;
pub const FRAME_MAGIC: &str = "QOL_BUILD_INFO_V1:";
pub const NATIVE_SECTION_NAME: &str = ".qolbi";
pub const ELF_SECTION_NAME: &str = NATIVE_SECTION_NAME;
pub const MACHO_SECTION_NAME: &str = "__qolbi";
pub const PE_SECTION_NAME: &str = NATIVE_SECTION_NAME;

pub const ENV_BUILD_INTENT: &str = "QOL_BUILD_INTENT";
pub const ENV_SOURCE_COMMIT: &str = "QOL_BUILD_SOURCE_COMMIT";
pub const ENV_SOURCE_HEAD_TREE: &str = "QOL_BUILD_SOURCE_HEAD_TREE";
pub const ENV_SOURCE_WORKING_TREE: &str = "QOL_BUILD_SOURCE_WORKING_TREE";
pub const ENV_COMPILER_OVERFLOW_CHECKS: &str = "QOL_BUILD_COMPILER_OVERFLOW_CHECKS";

pub(crate) const ENV_FRAME_FIELDS: &str = "QOL_BUILD_INFO_FIELDS";
pub(crate) const ENV_FRAME_PREFIX: &str = "QOL_BUILD_INFO_PREFIX";
pub(crate) const ENV_LINK_SECTION: &str = "QOL_BUILD_INFO_LINK_SECTION";

static CURRENT_IDENTITY: OnceLock<BuildIdentity> = OnceLock::new();

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BuildIntent {
    Production,
    Development,
    Sandbox,
    Unspecified,
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
    pub compiler: CompilerFacts,
    pub features: Vec<String>,
    pub source: SourceIdentity,
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

fn validate_identity(identity: &BuildIdentity) -> Result<(), DecodeError> {
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

pub(crate) fn link_section_for_target(target_os: &str) -> String {
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
        decode_frame, BuildIdentity, BuildIntent, BuildRole, CompilerFacts, DecodeError,
        SourceIdentity, FRAME_MAGIC, SCHEMA_VERSION,
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

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn arbitrary_frames_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..2048)) {
            let _ = decode_frame(&bytes);
        }
    }
}
