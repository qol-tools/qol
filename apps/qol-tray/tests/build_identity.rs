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

fn manifest_bin_names() -> Vec<String> {
    let manifest_path = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let manifest = std::fs::read_to_string(manifest_path).unwrap();
    let mut names = Vec::new();
    let mut in_bin = false;
    for line in manifest.lines() {
        if line.trim_start().starts_with("[[bin]]") {
            in_bin = true;
            continue;
        }
        if in_bin {
            if line.trim_start().starts_with("name = ") {
                let value = line
                    .split_once('=')
                    .map(|(_, value)| value.trim().trim_matches('"').to_string())
                    .unwrap_or_default();
                if !value.is_empty() {
                    names.push(value);
                }
                in_bin = false;
            } else if line.trim_start().starts_with('[') {
                in_bin = false;
            }
        }
    }
    names.sort();
    names
}

fn registry_binary_names() -> Vec<&'static str> {
    let mut names = vec![
        qol_conventions::artifact::TRAY_HOST_BINARY_NAME,
        qol_conventions::artifact::TRAY_INSTALLER_BINARY_NAME,
        qol_conventions::artifact::TRAY_DOCTOR_BINARY_NAME,
        qol_conventions::artifact::TRAY_MIGRATOR_BINARY_NAME,
        qol_conventions::artifact::TRAY_RESIDENT_POLICY_BINARY_NAME,
    ];
    names.sort();
    names
}

fn deployable_binaries() -> Vec<(&'static str, &'static str, BuildRole)> {
    let manifest_names = manifest_bin_names();
    let registry_names = registry_binary_names();
    assert_eq!(
        manifest_names,
        registry_names,
        "every manifest [[bin]] must have a typed registry entry and every registry entry a manifest bin"
    );
    manifest_names
        .iter()
        .map(|name| {
            let (binary, path) = match name.as_str() {
                qol_conventions::artifact::TRAY_HOST_BINARY_NAME => (
                    qol_conventions::artifact::TRAY_HOST_BINARY_NAME,
                    env!("CARGO_BIN_EXE_qol-tray"),
                ),
                qol_conventions::artifact::TRAY_INSTALLER_BINARY_NAME => (
                    qol_conventions::artifact::TRAY_INSTALLER_BINARY_NAME,
                    env!("CARGO_BIN_EXE_qol-tray-install"),
                ),
                qol_conventions::artifact::TRAY_DOCTOR_BINARY_NAME => (
                    qol_conventions::artifact::TRAY_DOCTOR_BINARY_NAME,
                    env!("CARGO_BIN_EXE_qol-tray-doctor"),
                ),
                qol_conventions::artifact::TRAY_MIGRATOR_BINARY_NAME => (
                    qol_conventions::artifact::TRAY_MIGRATOR_BINARY_NAME,
                    env!("CARGO_BIN_EXE_qol-tray-migrate"),
                ),
                qol_conventions::artifact::TRAY_RESIDENT_POLICY_BINARY_NAME => (
                    qol_conventions::artifact::TRAY_RESIDENT_POLICY_BINARY_NAME,
                    env!("CARGO_BIN_EXE_qol-resident-policy"),
                ),
                other => panic!("registry entry without a manifest bin: {other}"),
            };
            let role = qol_conventions::artifact::tray_binary_role(binary)
                .unwrap_or_else(|| panic!("manifest bin {binary} has no typed role"));
            (path, binary, role)
        })
        .collect()
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
    let mut expected_features = vec!["default".to_string()];
    if cfg!(feature = "dev") {
        expected_features.push("dev".to_string());
    }
    if cfg!(feature = "sandbox") {
        expected_features.push("sandbox".to_string());
    }
    if cfg!(feature = "embedded-ui") {
        expected_features.push("embedded-ui".to_string());
    }
    if cfg!(feature = "linux_evdev") {
        expected_features.push("linux_evdev".to_string());
    }
    expected_features.sort();
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
