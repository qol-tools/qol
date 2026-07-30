use qol_conventions::artifact::{
    BuildFlavor, BuildIntent, BuildProfile, BuildRole, SourceIdentity, SCHEMA_VERSION,
};

fn expected_intent() -> BuildIntent {
    match option_env!("QOL_BUILD_INTENT") {
        Some("production") => BuildIntent::Production,
        Some("development") => BuildIntent::Development,
        Some("sandbox") => BuildIntent::Sandbox,
        Some("unspecified") | None => BuildIntent::Unspecified,
        Some(value) => panic!("unexpected build intent {value:?}"),
    }
}

fn expected_source() -> SourceIdentity {
    match (
        option_env!("QOL_BUILD_SOURCE_COMMIT"),
        option_env!("QOL_BUILD_SOURCE_HEAD_TREE"),
        option_env!("QOL_BUILD_SOURCE_WORKING_TREE"),
    ) {
        (Some(commit), Some(head_tree), Some(working_tree)) => SourceIdentity::Git {
            commit: commit.to_string(),
            head_tree: head_tree.to_string(),
            working_tree: working_tree.to_string(),
        },
        (None, None, None) => SourceIdentity::Unspecified,
        _ => panic!("source identity environment must be complete"),
    }
}

fn deployable_binaries() -> [(&'static str, &'static str, BuildRole); 4] {
    [
        (
            env!("CARGO_BIN_EXE_qol-tray"),
            qol_conventions::artifact::TRAY_HOST_BINARY_NAME,
            BuildRole::Host,
        ),
        (
            env!("CARGO_BIN_EXE_qol-tray-install"),
            qol_conventions::artifact::TRAY_INSTALLER_BINARY_NAME,
            BuildRole::Installer,
        ),
        (
            env!("CARGO_BIN_EXE_qol-tray-doctor"),
            qol_conventions::artifact::TRAY_DOCTOR_BINARY_NAME,
            BuildRole::Doctor,
        ),
        (
            env!("CARGO_BIN_EXE_qol-tray-migrate"),
            qol_conventions::artifact::TRAY_MIGRATOR_BINARY_NAME,
            BuildRole::Migrator,
        ),
    ]
}

fn identity_frame_prefix() -> Vec<u8> {
    let mut prefix = qol_conventions::artifact::FRAME_MAGIC.as_bytes().to_vec();
    prefix.extend_from_slice(b"{\"binary\":\"");
    prefix
}

fn count_frames(bytes: &[u8], prefix: &[u8]) -> usize {
    let first = prefix[0];
    bytes
        .iter()
        .enumerate()
        .filter(|(index, byte)| **byte == first && bytes[*index..].starts_with(prefix))
        .count()
}

#[test]
fn every_deployable_binary_embeds_exactly_one_identity_frame() {
    let prefix = identity_frame_prefix();
    for (path, binary, _) in deployable_binaries() {
        let bytes = std::fs::read(path).unwrap();
        let frames = count_frames(&bytes, &prefix);
        assert_eq!(frames, 1, "binary {binary} at {path}");
    }
}

#[test]
fn every_deployable_binary_embeds_its_typed_identity() {
    let expected_cargo_profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let expected_opt_level = if cfg!(debug_assertions) { "0" } else { "3" };
    let expected_intent = expected_intent();
    let expected_build_profile = if expected_intent == BuildIntent::Sandbox {
        BuildProfile::Sandbox
    } else if cfg!(debug_assertions) {
        BuildProfile::Debug
    } else {
        BuildProfile::Release
    };
    let expected_flavor = BuildFlavor {
        profile: expected_build_profile,
        dev_features: cfg!(feature = "dev"),
    };
    let expected_features = if cfg!(feature = "dev") {
        vec!["default".to_string(), "dev".to_string()]
    } else {
        vec!["default".to_string()]
    };
    let expected_source = expected_source();

    for (path, binary, role) in deployable_binaries() {
        let inspected = qol_artifact::inspect_path(path).unwrap();
        assert_eq!(inspected.slices.len(), 1, "binary {binary} at {path}");
        let identity = &inspected.slices[0].identity;
        assert_eq!(identity.schema, SCHEMA_VERSION, "binary {binary}");
        assert_eq!(identity.binary, binary, "binary {binary}");
        assert_eq!(identity.role, role, "binary {binary}");
        assert_eq!(
            identity.package,
            qol_conventions::artifact::TRAY_PACKAGE_NAME,
            "binary {binary}"
        );
        assert_eq!(
            identity.version,
            env!("CARGO_PKG_VERSION"),
            "binary {binary}"
        );
        assert_eq!(identity.intent, expected_intent, "binary {binary}");
        assert_eq!(identity.flavor, expected_flavor, "binary {binary}");
        assert_eq!(
            identity.compiler.cargo_profile, expected_cargo_profile,
            "binary {binary}"
        );
        assert_eq!(
            identity.compiler.opt_level, expected_opt_level,
            "binary {binary}"
        );
        assert_eq!(
            identity.compiler.debug_assertions,
            cfg!(debug_assertions),
            "binary {binary}"
        );
        assert!(!identity.compiler.test, "binary {binary}");
        assert_eq!(identity.compiler.overflow_checks, None, "binary {binary}");
        assert_eq!(identity.features, expected_features, "binary {binary}");
        assert_eq!(identity.source, expected_source, "binary {binary}");
    }
}
