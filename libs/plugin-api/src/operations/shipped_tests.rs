use super::*;
use qol_config::contract::PeerReplay;
use std::path::Path;

type Exposed = (String, OperationKind, String, PeerReplay);

const SHIPPED_PEER_OPERATIONS: &[(&str, OperationKind, &str, PeerReplay)] = &[
    (
        "qol-bluetooth",
        OperationKind::Query,
        "handoff_state",
        PeerReplay::Idempotent,
    ),
    (
        "qol-bluetooth",
        OperationKind::Action,
        "release_for_handoff",
        PeerReplay::Never,
    ),
    (
        "qol-bluetooth",
        OperationKind::Action,
        "resume_reconnect",
        PeerReplay::Idempotent,
    ),
];

#[test]
fn every_shipped_plugin_preserves_its_declared_agent_tools() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins");
    let mut manifests = Vec::new();
    for entry in std::fs::read_dir(&root).expect("shipped plugins directory") {
        let entry = entry.expect("plugin directory entry");
        if !entry.file_type().expect("plugin entry type").is_dir() {
            continue;
        }
        let path = entry.path().join("plugin.toml");
        match std::fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() => manifests.push(path),
            Ok(_) => panic!("{}: manifest is not a file", path.display()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("{}: {error}", path.display()),
        }
    }
    manifests.sort();
    assert!(
        !manifests.is_empty(),
        "{}: empty plugin corpus",
        root.display()
    );
    let mut exposed = Vec::new();
    for path in manifests {
        exposed.extend(assert_shipped_catalog(&path));
    }
    let expected: Vec<Exposed> = SHIPPED_PEER_OPERATIONS
        .iter()
        .map(|&(plugin, kind, name, replay)| (plugin.into(), kind, name.into(), replay))
        .collect();
    assert_eq!(exposed, expected, "shipped peer exposure");
}

fn assert_shipped_catalog(path: &Path) -> Vec<Exposed> {
    let manifest = PluginManifest::load_and_validate(path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let plugin_id = manifest.plugin.id.clone().unwrap_or_else(|| {
        PluginId::new(
            path.parent()
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy(),
        )
    });
    let runtime_path = path.with_file_name("qol-runtime.toml");
    let runtime = match std::fs::read_to_string(&runtime_path) {
        Ok(source) => Some(
            qol_config::contract::parse_runtime_spec_str(&source)
                .unwrap_or_else(|error| panic!("{}: {error:?}", runtime_path.display())),
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => panic!("{}: {error}", runtime_path.display()),
    };
    let catalog = OperationCatalog::derive(&plugin_id, &manifest, runtime.as_ref())
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let exposed = catalog
        .iter()
        .filter_map(|operation| {
            let peer = operation.peer.as_ref()?;
            Some((
                plugin_id.to_string(),
                operation.key.kind,
                operation.key.name.clone(),
                peer.replay,
            ))
        })
        .collect();
    let Some(runtime) = runtime else {
        assert!(
            catalog.iter().all(|operation| !operation.agent_tool),
            "{}",
            path.display()
        );
        return exposed;
    };
    assert_agent_tools(path, &plugin_id, &manifest, &catalog, &runtime);
    exposed
}

fn assert_agent_tools(
    path: &Path,
    plugin_id: &PluginId,
    manifest: &PluginManifest,
    catalog: &OperationCatalog,
    runtime: &RuntimeSpec,
) {
    let queries = runtime.queries.iter().map(|(name, spec)| {
        (
            OperationKind::Query,
            name,
            &spec.description,
            &spec.tool_description,
            &spec.input,
            spec.agent_tool,
        )
    });
    let actions = runtime.actions.iter().map(|(name, spec)| {
        (
            OperationKind::Action,
            name,
            &spec.description,
            &spec.tool_description,
            &spec.input,
            spec.agent_tool,
        )
    });
    let mut expected = Vec::new();
    for (kind, name, description, tool_description, input, agent_tool) in queries.chain(actions) {
        if !agent_tool {
            continue;
        }
        let identity = match &manifest.plugin.uid {
            Some(uid) => OperationIdentity::Stable(uid.clone()),
            None => OperationIdentity::Local(plugin_id.clone()),
        };
        let key = OperationKey {
            identity,
            kind,
            name: name.clone(),
        };
        let operation = catalog
            .get(&key)
            .unwrap_or_else(|| panic!("{}: missing {} {name}", path.display(), kind.as_str()));
        assert!(operation.agent_tool, "{}: {key:?}", path.display());
        assert_eq!(&operation.input, input, "{}: {key:?}", path.display());
        assert_eq!(
            &operation.description,
            description,
            "{}: {key:?}",
            path.display()
        );
        assert_eq!(
            &operation.tool_description,
            tool_description,
            "{}: {key:?}",
            path.display()
        );
        expected.push(key);
    }
    let actual: Vec<_> = catalog
        .iter()
        .filter(|operation| operation.agent_tool)
        .map(|operation| operation.key.clone())
        .collect();
    assert_eq!(
        actual,
        expected,
        "{}: agent tool identity/order",
        path.display()
    );
}
