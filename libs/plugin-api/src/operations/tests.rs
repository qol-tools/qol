use super::*;

const _: fn(crate::manifest::PluginId) -> qol_conventions::plugin_id::PluginId = |value| value;
const _: fn(crate::PluginId) -> qol_conventions::plugin_id::PluginId = |value| value;
const _: fn(crate::manifest::PluginUid) -> qol_conventions::plugin_id::PluginUid = |value| value;
const _: fn(OperationKind) -> qol_conventions::operations::OperationKind = |value| value;
const _: fn(Invocation) -> qol_conventions::operations::Invocation = |value| value;
const _: fn(OperationIdentity) -> qol_conventions::operations::OperationIdentity = |value| value;
const _: fn(OperationKey) -> qol_conventions::operations::OperationKey = |value| value;

const MANIFEST: &str = r#"
[plugin]
id = "qol-lights"
uid = "uid-lights"
name = "Lights"
description = ""
version = "1.0.0"

[menu]
label = "Lights"
items = []

[action.blink]
label = "Blink"
args = ["blink"]

[action.settings]
label = "Settings"
kind = "settings"
args = ["settings"]
"#;

const MANIFEST_WITH_PEER: &str = r#"
[plugin]
id = "qol-lights"
uid = "uid-lights"
name = "Lights"
description = ""
version = "1.0.0"

[menu]
label = "Lights"
items = []

[action.blink]
label = "Blink"
args = ["blink"]
peer = { replay = "idempotent" }

[action.settings]
label = "Settings"
kind = "settings"
args = ["settings"]
"#;

const MANIFEST_WITHOUT_ACTIONS: &str = r#"
[plugin]
id = "qol-lights"
uid = "uid-lights"
name = "Lights"
description = ""
version = "1.0.0"

[menu]
label = "Lights"
items = []
"#;

const MANIFEST_WITHOUT_UID: &str = r#"
[plugin]
id = "qol-lights"
name = "Lights"
description = ""
version = "1.0.0"

[menu]
label = "Lights"
items = []

[action.blink]
label = "Blink"
args = ["blink"]
"#;

const MANIFEST_DECLARING_OTHER_ID: &str = r#"
[plugin]
id = "qol-installed"
uid = "uid-lights"
name = "Lights"
description = ""
version = "1.0.0"

[menu]
label = "Lights"
items = []

[action.blink]
label = "Blink"
args = ["blink"]
"#;

const MANIFEST_TOGGLE: &str = r#"
[plugin]
id = "qol-lights"
uid = "uid-lights"
name = "Lights"
description = ""
version = "1.0.0"

[menu]
label = "Lights"
items = []

[action.toggle]
label = "Toggle"
kind = "toggle-config"
config_key = "enabled"
"#;

const MANIFEST_EQUAL_LABELS: &str = r#"
[plugin]
id = "qol-lights"
uid = "uid-lights"
name = "Lights"
description = ""
version = "1.0.0"

[menu]
label = "Lights"
items = []

[action.alpha]
label = "Apply"
args = ["alpha"]
"#;

const RUNTIME: &str = r#"
schema_version = 1

[action.blink]
description = "Blink the light"

[query.status]
description = "Current light state"
poll_interval_ms = 1000

[stream.live]
description = "Live light state"
throttle_ms = 100
"#;

const RUNTIME_WITH_AGENT_TOOLS: &str = r#"
schema_version = 1

[action.blink]
description = "Blink the light"
agent_tool = true

[query.status]
description = "Current light state"
poll_interval_ms = 1000
agent_tool = true
"#;

const RUNTIME_MATCHING_PEER: &str = r#"
schema_version = 1

[action.blink]
description = "Blink the light"
peer = { replay = "idempotent" }
"#;

const RUNTIME_CONFLICTING_PEER: &str = r#"
schema_version = 1

[action.blink]
description = "Blink the light"
peer = { replay = "never" }
"#;

const RUNTIME_SETTINGS_PEER: &str = r#"
schema_version = 1

[action.settings]
description = "Settings"
peer = { replay = "idempotent" }
"#;

const RUNTIME_TOGGLE_ACTION: &str = r#"
schema_version = 1

[action.toggle]
description = "Toggle the light"
"#;

const RUNTIME_BETA_APPLY: &str = r#"
schema_version = 1

[action.beta]
description = "Apply"
"#;

fn parse_manifest(source: &str) -> PluginManifest {
    let manifest: PluginManifest = toml::from_str(source).expect("manifest parses");
    manifest.validate().expect("manifest validates");
    manifest
}

fn derive(
    plugin_id: &str,
    manifest_source: &str,
    runtime_source: Option<&str>,
) -> Result<OperationCatalog, OperationCatalogError> {
    let manifest = parse_manifest(manifest_source);
    let runtime =
        runtime_source.map(|source| qol_config::contract::parse_runtime_spec_str(source).unwrap());
    OperationCatalog::derive(&PluginId::new(plugin_id), &manifest, runtime.as_ref())
}

fn key(uid: &str, kind: OperationKind, name: &str) -> OperationKey {
    OperationKey::new(PluginUid::new(uid), kind, name)
}

#[test]
fn merges_activation_and_runtime_action_into_one_entry() {
    let catalog = derive("qol-lights", MANIFEST, Some(RUNTIME)).expect("derive");

    assert_eq!(catalog.len(), 4, "blink, settings, status and live");
    let blink = catalog
        .get(&key("uid-lights", OperationKind::Action, "blink"))
        .expect("blink is one entry");
    assert_eq!(blink.description, "Blink the light");
    assert_eq!(blink.invocation(), Some(Invocation::Action));
    assert!(catalog
        .get(&key("uid-lights", OperationKind::Action, "settings"))
        .is_some());
}

#[test]
fn activation_only_manifest_keeps_argv_actions_addressable() {
    let catalog = derive("qol-lights", MANIFEST, None).expect("derive");

    assert_eq!(catalog.len(), 2);
    let blink = catalog
        .get(&key("uid-lights", OperationKind::Action, "blink"))
        .expect("blink");
    assert_eq!(blink.description, "Blink");
    assert_eq!(blink.plugin_id.as_str(), "qol-lights");
    assert_eq!(blink.invocation(), Some(Invocation::Action));
    assert!(blink.peer.is_none());
}

#[test]
fn runtime_only_plugin_keeps_every_declared_kind() {
    let catalog = derive("qol-lights", MANIFEST_WITHOUT_ACTIONS, Some(RUNTIME)).expect("derive");

    assert_eq!(catalog.len(), 3);
    assert!(catalog
        .get(&key("uid-lights", OperationKind::Action, "blink"))
        .is_some());
    assert!(catalog
        .get(&key("uid-lights", OperationKind::Query, "status"))
        .is_some());
    assert!(catalog
        .get(&key("uid-lights", OperationKind::Stream, "live"))
        .is_some());
}

#[test]
fn identity_is_the_frozen_uid_while_dispatch_keeps_the_local_plugin_id() {
    let catalog =
        derive("qol-located", MANIFEST_DECLARING_OTHER_ID, Some(RUNTIME)).expect("derive");

    let blink = catalog
        .get(&key("uid-lights", OperationKind::Action, "blink"))
        .expect("keyed by frozen uid");
    assert_eq!(blink.plugin_id.as_str(), "qol-located");
    assert!(catalog
        .get(&key("qol-located", OperationKind::Action, "blink"))
        .is_none());
}

#[test]
fn manifest_without_uid_stays_representable_locally() {
    let catalog = derive("qol-lights", MANIFEST_WITHOUT_UID, None).expect("derive");

    let blink = catalog
        .get(&OperationKey {
            identity: OperationIdentity::Local(PluginId::new("qol-lights")),
            kind: OperationKind::Action,
            name: "blink".to_string(),
        })
        .expect("fallback identity");
    assert!(blink.peer.is_none());
}

#[test]
fn peer_exposure_defaults_to_denied_even_for_agent_tools() {
    let catalog = derive("qol-lights", MANIFEST, Some(RUNTIME_WITH_AGENT_TOOLS)).expect("derive");

    let action = catalog
        .get(&key("uid-lights", OperationKind::Action, "blink"))
        .expect("blink");
    assert!(action.agent_tool);
    assert!(action.peer.is_none());
    let query = catalog
        .get(&key("uid-lights", OperationKind::Query, "status"))
        .expect("status");
    assert!(query.agent_tool);
    assert!(query.peer.is_none());
}

#[test]
fn runtime_owned_peer_exposure_survives_the_merge() {
    let catalog = derive("qol-lights", MANIFEST, Some(RUNTIME_MATCHING_PEER)).expect("derive");

    let blink = catalog
        .get(&key("uid-lights", OperationKind::Action, "blink"))
        .expect("blink");
    assert_eq!(
        blink.peer,
        Some(qol_config::contract::PeerExposure {
            replay: qol_config::contract::PeerReplay::Idempotent,
        })
    );
}

#[test]
fn conflicting_peer_exposure_on_one_operation_fails() {
    let result = derive(
        "qol-lights",
        MANIFEST_WITH_PEER,
        Some(RUNTIME_CONFLICTING_PEER),
    );

    match result {
        Err(OperationCatalogError::ActivationOwnsRuntimePeerExposure { name, .. }) => {
            assert_eq!(name, "blink");
        }
        other => panic!("expected conflicting exposure, got {other:?}"),
    }
}

#[test]
fn activation_cannot_own_exposure_for_a_typed_runtime_action() {
    let result = derive("qol-lights", MANIFEST_WITH_PEER, Some(RUNTIME));

    match result {
        Err(OperationCatalogError::ActivationOwnsRuntimePeerExposure { name, .. }) => {
            assert_eq!(name, "blink");
        }
        other => panic!("expected conflicting exposure, got {other:?}"),
    }
}

#[test]
fn peer_exposure_requires_a_frozen_uid() {
    let cases = [
        (
            "action",
            r#"
schema_version = 1

[action.blink]
description = "Blink"
peer = { replay = "idempotent" }
"#,
        ),
        (
            "query",
            r#"
schema_version = 1

[query.status]
description = "Status"
poll_interval_ms = 1000
peer = { replay = "idempotent" }
"#,
        ),
        (
            "stream",
            r#"
schema_version = 1

[stream.live]
description = "Live"
throttle_ms = 100
peer = { replay = "idempotent" }
"#,
        ),
    ];
    for (kind, runtime) in cases {
        match derive("qol-lights", MANIFEST_WITHOUT_UID, Some(runtime)) {
            Err(OperationCatalogError::PeerExposureWithoutStableUid { .. }) => {}
            other => panic!("expected stable uid requirement for {kind}, got {other:?}"),
        }
    }
}

#[test]
fn peer_exposure_on_a_settings_operation_fails() {
    let result = derive("qol-lights", MANIFEST, Some(RUNTIME_SETTINGS_PEER));

    match result {
        Err(OperationCatalogError::PeerExposureOnNonRunnable { name, .. }) => {
            assert_eq!(name, "settings");
        }
        other => panic!("expected non-runnable rejection, got {other:?}"),
    }
}

#[test]
fn runtime_action_shadowing_a_toggle_config_fails() {
    let result = derive("qol-lights", MANIFEST_TOGGLE, Some(RUNTIME_TOGGLE_ACTION));

    match result {
        Err(OperationCatalogError::RuntimeActionNotExecutable { name, .. }) => {
            assert_eq!(name, "toggle");
        }
        other => panic!("expected non-executable shadow rejection, got {other:?}"),
    }
}

#[test]
fn streams_are_not_invocable_as_ordinary_operations() {
    let catalog = derive("qol-lights", MANIFEST, Some(RUNTIME)).expect("derive");

    let stream = catalog
        .get(&key("uid-lights", OperationKind::Stream, "live"))
        .expect("live");
    assert_eq!(stream.key.kind, OperationKind::Stream);
    assert_eq!(stream.invocation(), None);
    assert_eq!(stream.key.kind.as_str(), "stream");

    let query = catalog
        .get(&key("uid-lights", OperationKind::Query, "status"))
        .expect("status");
    assert_eq!(query.invocation(), Some(Invocation::Query));
}

#[test]
fn equal_labels_never_merge_distinct_operations() {
    let catalog = derive(
        "qol-lights",
        MANIFEST_EQUAL_LABELS,
        Some(RUNTIME_BETA_APPLY),
    )
    .expect("derive");

    assert_eq!(catalog.len(), 2);
    let alpha = catalog
        .get(&key("uid-lights", OperationKind::Action, "alpha"))
        .expect("alpha");
    let beta = catalog
        .get(&key("uid-lights", OperationKind::Action, "beta"))
        .expect("beta");
    assert_eq!(alpha.description, beta.description);
}

#[test]
fn extend_rejects_a_second_plugin_claiming_one_identity() {
    let mut first = derive("qol-lights", MANIFEST, None).expect("derive");
    let second = derive("qol-lights", MANIFEST, None).expect("derive");

    let conflict = first.extend(second).expect_err("duplicate identity");
    assert_eq!(
        conflict.identity,
        OperationIdentity::Stable(PluginUid::new("uid-lights"))
    );
    assert_eq!(conflict.name, "blink");
    assert_eq!(first.len(), 2, "failed merge leaves the catalog unchanged");
}

#[test]
fn shipped_template_manifest_stays_representable() {
    let source = include_str!("../../../../plugins/template/plugin.toml");
    let manifest: PluginManifest = toml::from_str(source).expect("template manifest parses");
    manifest.validate().expect("template manifest validates");
    let catalog =
        OperationCatalog::derive(&PluginId::new("qol-template"), &manifest, None).expect("derive");

    let run = catalog
        .get(&OperationKey {
            identity: OperationIdentity::Local(PluginId::new("qol-template")),
            kind: OperationKind::Action,
            name: "run".to_string(),
        })
        .expect("run");
    assert_eq!(run.description, "Run");
    assert!(catalog
        .get(&OperationKey {
            identity: OperationIdentity::Local(PluginId::new("qol-template")),
            kind: OperationKind::Action,
            name: "settings".to_string(),
        })
        .is_some());
    assert!(catalog.iter().all(|operation| operation.peer.is_none()));
}

#[test]
fn duplicate_exposure_is_rejected_even_when_values_agree() {
    assert!(matches!(
        derive(
            "qol-lights",
            MANIFEST_WITH_PEER,
            Some(RUNTIME_MATCHING_PEER)
        ),
        Err(OperationCatalogError::ActivationOwnsRuntimePeerExposure { .. })
    ));
}

#[test]
fn activation_only_exposure_preserves_exact_argv() {
    for args in [vec![], vec!["subcommand", "space value", "$(literal)"]] {
        let mut manifest = parse_manifest(MANIFEST_WITH_PEER);
        let expected: Vec<String> = args.into_iter().map(str::to_string).collect();
        manifest.actions["blink"].args = Some(expected.clone());
        let catalog = OperationCatalog::derive(&PluginId::new("qol-lights"), &manifest, None)
            .expect("activation-only catalog");
        let operation = catalog
            .get(&key("uid-lights", OperationKind::Action, "blink"))
            .unwrap();
        assert_eq!(operation.runtime_args, Some(expected));
        assert!(operation.peer.is_some());
    }
}

#[test]
fn typed_actions_preserve_cli_aliases_and_fixed_or_empty_arguments() {
    for args in [
        vec![],
        vec!["different"],
        vec!["session", "start"],
        vec!["blink", "fixed argument", "$(literal)", ""],
    ] {
        let mut manifest = parse_manifest(MANIFEST);
        let expected: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        manifest.actions["blink"].args = Some(expected.clone());
        let runtime =
            qol_config::contract::parse_runtime_spec_str(RUNTIME_WITH_AGENT_TOOLS).unwrap();
        let catalog =
            OperationCatalog::derive(&PluginId::new("qol-lights"), &manifest, Some(&runtime))
                .unwrap_or_else(|error| panic!("{args:?}: {error}"));
        let operation = catalog
            .get(&key("uid-lights", OperationKind::Action, "blink"))
            .unwrap();
        assert_eq!(operation.runtime_args, Some(expected), "{args:?}");
        assert_eq!(operation.invocation(), Some(Invocation::Action), "{args:?}");
        assert_eq!(operation.description, "Blink the light", "{args:?}");
        assert!(operation.agent_tool, "{args:?}");
        assert!(operation.peer.is_none(), "{args:?}");
        assert_eq!(catalog.len(), 3, "{args:?}");
    }
}

#[test]
fn legacy_menu_argv_uses_the_existing_runtime_mapping() {
    let source = r#"
[plugin]
id = "qol-legacy"
uid = "legacy-uid"
name = "Legacy"
description = ""
version = "1.0.0"
[menu]
label = "Legacy"
items = [{ type = "action", id = "blink", label = "Blink", action = "run" }]
[runtime]
command = "plugin-bin"
[runtime.actions]
blink = ["apply", "fixed value"]
"#;
    let catalog = derive("qol-legacy", source, None).unwrap();
    let operation = catalog
        .get(&key("legacy-uid", OperationKind::Action, "blink"))
        .unwrap();
    assert_eq!(
        operation.runtime_args,
        Some(vec!["apply".into(), "fixed value".into()])
    );
    let typed = derive("qol-legacy", source, Some(RUNTIME)).unwrap();
    let merged = typed
        .get(&key("legacy-uid", OperationKind::Action, "blink"))
        .unwrap();
    assert_eq!(merged.runtime_args, operation.runtime_args);
    assert_eq!(merged.description, "Blink the light");
}

#[test]
fn cross_kind_names_keep_exposure_and_schemas_separate() {
    for (kind, fields) in [
        (
            OperationKind::Query,
            "poll_interval_ms = 1000\nagent_tool = true\ninput = { zone = \"Zone\" }",
        ),
        (OperationKind::Stream, "throttle_ms = 100"),
    ] {
        for activation_peer in [false, true] {
            let runtime = format!(
                "schema_version = 1\n[{}.blink]\ndescription = \"Typed read\"\n{fields}\n{}",
                kind.as_str(),
                if activation_peer {
                    ""
                } else {
                    "peer = { replay = \"idempotent\" }"
                },
            );
            let manifest = if activation_peer {
                MANIFEST_WITH_PEER
            } else {
                MANIFEST
            };
            let catalog = derive("qol-lights", manifest, Some(&runtime)).unwrap();
            let activation = catalog
                .get(&key("uid-lights", OperationKind::Action, "blink"))
                .unwrap();
            let typed = catalog.get(&key("uid-lights", kind, "blink")).unwrap();
            assert_eq!(
                catalog.len(),
                3,
                "{kind:?}, activation_peer={activation_peer}"
            );
            assert!(
                !activation.agent_tool,
                "{kind:?}, activation_peer={activation_peer}"
            );
            assert!(
                activation.input.is_none(),
                "{kind:?}, activation_peer={activation_peer}"
            );
            assert_eq!(activation.peer.is_some(), activation_peer, "{kind:?}");
            assert_eq!(typed.peer.is_some(), !activation_peer, "{kind:?}");
            assert_eq!(typed.agent_tool, kind == OperationKind::Query, "{kind:?}");
            assert_eq!(
                typed.input.is_some(),
                kind == OperationKind::Query,
                "{kind:?}"
            );
            assert_eq!(typed.description, "Typed read", "{kind:?}");
            assert_eq!(activation.description, "Blink", "{kind:?}");
        }
    }
}

#[test]
fn legacy_local_identity_never_collides_with_a_frozen_uid() {
    let mut local = derive("qol-lights", MANIFEST_WITHOUT_UID, None).unwrap();
    let manifest = MANIFEST.replace("uid-lights", "qol-lights");
    let stable = derive("another-plugin", &manifest, None).unwrap();
    local.extend(stable).unwrap();
    assert_eq!(local.len(), 3);
    assert!(local
        .get(&key("qol-lights", OperationKind::Action, "blink"))
        .is_some());
}

#[test]
fn duplicate_uid_is_rejected_even_with_disjoint_operations() {
    let mut first = derive("first", MANIFEST, None).unwrap();
    let second = derive("second", MANIFEST_WITHOUT_ACTIONS, Some(RUNTIME_BETA_APPLY)).unwrap();
    assert!(first.extend(second).is_err());
    assert_eq!(first.len(), 2);
}

#[test]
fn programmatically_constructed_contracts_are_validated() {
    let mut manifest = parse_manifest(MANIFEST);
    for uid in ["", " ", " uid", "uid\nvalue"] {
        manifest.plugin.uid = Some(PluginUid::new(uid));
        assert!(
            matches!(
                OperationCatalog::derive(&PluginId::new("qol-lights"), &manifest, None),
                Err(OperationCatalogError::InvalidContract { .. })
            ),
            "{uid:?}"
        );
    }
    let manifest = parse_manifest(MANIFEST);
    let mut runtime = qol_config::contract::parse_runtime_spec_str(RUNTIME).unwrap();
    runtime.queries["status"].peer = Some(qol_config::contract::PeerExposure {
        replay: qol_config::contract::PeerReplay::Never,
    });
    assert!(matches!(
        OperationCatalog::derive(&PluginId::new("qol-lights"), &manifest, Some(&runtime)),
        Err(OperationCatalogError::InvalidContract { .. })
    ));
}

#[test]
fn normalization_preserves_input_order_and_description_override() {
    let runtime = r#"
schema_version = 1
[action.blink]
description = "Blink"
tool_description = "Detailed blink"
input = { zone = "zone", agent_home = "caller", intensity = "intensity" }
agent_tool = true
"#;
    let catalog = derive("qol-lights", MANIFEST, Some(runtime)).unwrap();
    let operation = catalog
        .get(&key("uid-lights", OperationKind::Action, "blink"))
        .unwrap();
    assert_eq!(
        operation
            .input
            .as_ref()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["zone", "agent_home", "intensity"]
    );
    assert_eq!(operation.tool_description(), "Detailed blink");
    assert_eq!(operation.runtime_args, Some(vec!["blink".into()]));
}

#[test]
fn shipped_lights_contract_keeps_streams_separate_without_peer_exposure() {
    let catalog = derive(
        "qol-lights",
        include_str!("../../../../plugins/lights/plugin.toml"),
        Some(include_str!("../../../../plugins/lights/qol-runtime.toml")),
    )
    .unwrap();
    let stream = catalog
        .iter()
        .find(|operation| operation.key.name == "live_color")
        .unwrap();
    assert_eq!(stream.invocation(), None);
    assert!(catalog.iter().all(|operation| operation.peer.is_none()));
}

#[test]
fn omitted_activation_args_resolve_to_the_typed_operation_name() {
    let mut manifest = parse_manifest(MANIFEST);
    manifest.actions["blink"].args = None;
    let runtime = qol_config::contract::parse_runtime_spec_str(RUNTIME_MATCHING_PEER).unwrap();
    let catalog =
        OperationCatalog::derive(&PluginId::new("qol-lights"), &manifest, Some(&runtime)).unwrap();
    let operation = catalog
        .get(&key("uid-lights", OperationKind::Action, "blink"))
        .unwrap();
    assert_eq!(operation.runtime_args, Some(vec!["blink".into()]));
    assert!(operation.peer.is_some());
}

#[test]
fn typed_settings_keep_local_metadata_and_exact_cli_mapping() {
    let mut manifest = parse_manifest(MANIFEST);
    manifest.actions["settings"].args = Some(vec!["--action".into(), "settings".into()]);
    let runtime = qol_config::contract::parse_runtime_spec_str(
        r#"
schema_version = 1
[action.settings]
description = "Open settings"
tool_description = "Configure the plugin"
agent_tool = true
input = { page = "Page to open" }
"#,
    )
    .unwrap();
    let catalog =
        OperationCatalog::derive(&PluginId::new("qol-lights"), &manifest, Some(&runtime)).unwrap();
    let settings = catalog
        .get(&key("uid-lights", OperationKind::Action, "settings"))
        .unwrap();
    assert_eq!(catalog.len(), 2);
    assert_eq!(settings.action_kind, Some(ActionType::Settings));
    assert_eq!(settings.runtime_args, manifest.actions["settings"].args);
    assert_eq!(settings.description, "Open settings");
    assert_eq!(settings.tool_description(), "Configure the plugin");
    assert_eq!(settings.input, runtime.actions["settings"].input);
    assert!(settings.agent_tool);
    assert!(settings.peer.is_none());
}

#[test]
fn runtime_own_name_collisions_still_fail_catalog_validation() {
    for kind in [OperationKind::Query, OperationKind::Stream] {
        let mut runtime = qol_config::contract::parse_runtime_spec_str(RUNTIME).unwrap();
        match kind {
            OperationKind::Query => {
                let query = runtime.queries.shift_remove("status").unwrap();
                runtime.queries.insert("blink".into(), query);
            }
            OperationKind::Stream => {
                let stream = runtime.streams.shift_remove("live").unwrap();
                runtime.streams.insert("blink".into(), stream);
            }
            OperationKind::Action => unreachable!(),
        }
        assert!(
            matches!(
                OperationCatalog::derive(
                    &PluginId::new("qol-lights"),
                    &parse_manifest(MANIFEST),
                    Some(&runtime),
                ),
                Err(OperationCatalogError::InvalidContract { .. })
            ),
            "{kind:?}"
        );
    }
}

#[test]
fn argv_naming_another_action_never_merges_canonical_identities() {
    let mut manifest = parse_manifest(MANIFEST);
    manifest.actions["blink"].args = Some(vec!["settings".into()]);
    let runtime = qol_config::contract::parse_runtime_spec_str(RUNTIME_MATCHING_PEER).unwrap();
    let catalog =
        OperationCatalog::derive(&PluginId::new("qol-lights"), &manifest, Some(&runtime)).unwrap();
    let blink = catalog
        .get(&key("uid-lights", OperationKind::Action, "blink"))
        .unwrap();
    let settings = catalog
        .get(&key("uid-lights", OperationKind::Action, "settings"))
        .unwrap();
    assert_eq!(catalog.len(), 2);
    assert_eq!(blink.runtime_args, settings.runtime_args);
    assert!(blink.peer.is_some());
    assert!(settings.peer.is_none());
    assert_ne!(blink.key, settings.key);
}

#[test]
fn non_run_activation_peer_metadata_is_rejected_without_a_typed_action() {
    for source in [MANIFEST, MANIFEST_TOGGLE] {
        let mut manifest = parse_manifest(source);
        let declaration = manifest
            .actions
            .values_mut()
            .find(|action| action.kind != ActionType::Run)
            .unwrap();
        declaration.peer = Some(qol_config::contract::PeerExposure {
            replay: qol_config::contract::PeerReplay::Idempotent,
        });
        assert!(
            matches!(
                OperationCatalog::derive(&PluginId::new("qol-lights"), &manifest, None),
                Err(OperationCatalogError::InvalidContract { .. })
            ),
            "{source}"
        );
    }
}
