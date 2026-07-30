use qol_conventions::artifact::{BuildIntent, BuildRole, SourceIdentity, SCHEMA_VERSION};

#[test]
fn every_deployable_binary_embeds_its_typed_identity() {
    let expected_profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let expected_opt_level = if cfg!(debug_assertions) { "0" } else { "3" };
    let cases = [
        (env!("CARGO_BIN_EXE_qol-tray"), "qol-tray", BuildRole::Host),
        (
            env!("CARGO_BIN_EXE_qol-tray-install"),
            "qol-tray-install",
            BuildRole::Installer,
        ),
        (
            env!("CARGO_BIN_EXE_qol-tray-doctor"),
            "qol-tray-doctor",
            BuildRole::Doctor,
        ),
        (
            env!("CARGO_BIN_EXE_qol-tray-migrate"),
            "qol-tray-migrate",
            BuildRole::Migrator,
        ),
    ];

    for (path, binary, role) in cases {
        let inspected = qol_artifact::inspect_path(path).unwrap();
        assert_eq!(inspected.slices.len(), 1, "binary {binary} at {path}");
        let identity = &inspected.slices[0].identity;
        assert_eq!(identity.schema, SCHEMA_VERSION, "binary {binary}");
        assert_eq!(identity.binary, binary, "binary {binary}");
        assert_eq!(identity.role, role, "binary {binary}");
        assert_eq!(identity.package, "qol-tray", "binary {binary}");
        assert_eq!(
            identity.version,
            env!("CARGO_PKG_VERSION"),
            "binary {binary}"
        );
        assert_eq!(identity.intent, BuildIntent::Unspecified, "binary {binary}");
        assert_eq!(
            identity.compiler.cargo_profile, expected_profile,
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
        assert_eq!(
            identity.features,
            ["default".to_string()],
            "binary {binary}"
        );
        assert_eq!(
            identity.source,
            SourceIdentity::Unspecified,
            "binary {binary}"
        );
    }
}
