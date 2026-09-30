use crate::plugins::PluginManager;
use qol_config::contract::IndexMap;
use qol_plugin_api::operations::{Invocation, Operation, OperationCatalog};
use std::sync::{Arc, Mutex};

pub(super) struct PluginToolHost {
    plugin_manager: Arc<Mutex<PluginManager>>,
}

impl PluginToolHost {
    pub(super) fn new(plugin_manager: Arc<Mutex<PluginManager>>) -> Self {
        Self { plugin_manager }
    }
}

impl qol_mcp::ToolHost for PluginToolHost {
    fn server_info(&self) -> qol_mcp::ServerInfo {
        qol_mcp::ServerInfo {
            name: "qol-tray".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    fn list(&self) -> Vec<qol_mcp::ToolSpec> {
        let specs: Vec<qol_mcp::ToolSpec> = bindings(&self.plugin_manager)
            .into_iter()
            .map(|binding| binding.spec)
            .collect();
        qol_runtime::probe!("TRAY_MCP", "event=tools_listed count={}", specs.len());
        specs
    }

    fn call(
        &self,
        name: &str,
        arguments: serde_json::Value,
        caller: &qol_mcp::Caller,
    ) -> qol_mcp::ToolResult {
        let binding = match bindings(&self.plugin_manager)
            .into_iter()
            .find(|binding| binding.spec.name == name)
        {
            Some(binding) => binding,
            None => return qol_mcp::ToolResult::error(format!("unknown tool: {name}")),
        };
        let arguments =
            match with_agent_home_argument(arguments, caller, binding.accepts_agent_home, || {
                qol_agent_homes::Registry::load().is_partitioned()
            }) {
                Ok(arguments) => arguments,
                Err(message) => return qol_mcp::ToolResult::error(message),
            };
        qol_runtime::probe!(
            "TRAY_MCP",
            "event=tool_called plugin={} runable={} kind={}",
            binding.plugin_id,
            binding.runable,
            kind_label(binding.kind)
        );
        match binding.kind {
            Invocation::Query => {
                match crate::plugins::action_executor::dispatch_query_with_input(
                    &self.plugin_manager,
                    &binding.plugin_id,
                    &binding.runable,
                    arguments,
                    crate::plugins::action_executor::MCP_DISPATCH_TIMEOUT,
                ) {
                    Ok(value) => qol_mcp::ToolResult::structured(value),
                    Err(error) => tool_failed(&binding, &error.to_string()),
                }
            }
            Invocation::Action => {
                match crate::plugins::action_executor::try_execute_action_with_input_result(
                    &self.plugin_manager,
                    &binding.plugin_id,
                    &binding.runable,
                    arguments,
                ) {
                    Ok(Some(value)) => qol_mcp::ToolResult::structured(value),
                    Ok(None) => {
                        qol_mcp::ToolResult::structured(serde_json::json!({"status": "ok"}))
                    }
                    Err(error) => tool_failed(&binding, &error.to_string()),
                }
            }
        }
    }
}

const MISSING_AGENT_HOME_ERROR: &str =
    "caller identity missing: no x-qol-agent-home header; run qol mcp configure <harness>, restart the harness, and update the qol CLI if qol mcp headers prints no x-qol-agent-home";

fn with_agent_home_argument(
    arguments: serde_json::Value,
    caller: &qol_mcp::Caller,
    accepts_agent_home: bool,
    partitioned: impl FnOnce() -> bool,
) -> Result<serde_json::Value, String> {
    let mut arguments = if arguments.is_null() {
        serde_json::json!({})
    } else {
        arguments
    };
    if !accepts_agent_home {
        return Ok(arguments);
    }
    match caller.agent_home.as_deref() {
        Some(agent_home) => {
            let Some(map) = arguments.as_object_mut() else {
                return Err("arguments must be a JSON object".to_owned());
            };
            map.insert(
                "agent_home".to_owned(),
                serde_json::Value::String(agent_home.to_owned()),
            );
            Ok(arguments)
        }
        None => {
            if partitioned() {
                Err(MISSING_AGENT_HOME_ERROR.to_owned())
            } else {
                Ok(arguments)
            }
        }
    }
}

fn tool_failed(binding: &ToolBinding, error: &str) -> qol_mcp::ToolResult {
    qol_runtime::probe!(
        "TRAY_MCP",
        "event=tool_failed plugin={} runable={} error={}",
        binding.plugin_id,
        binding.runable,
        error
    );
    qol_mcp::ToolResult::error(error)
}

fn kind_label(invocation: Invocation) -> &'static str {
    match invocation {
        Invocation::Query => "query",
        Invocation::Action => "action",
    }
}

struct ToolBinding {
    plugin_id: String,
    runable: String,
    kind: Invocation,
    accepts_agent_home: bool,
    spec: qol_mcp::ToolSpec,
}

fn tool_name(plugin_id: &str, runable: &str) -> String {
    format!("{plugin_id}__{runable}")
}

fn bindings(plugin_manager: &Arc<Mutex<PluginManager>>) -> Vec<ToolBinding> {
    let Ok(mut manager) = plugin_manager.lock() else {
        return Vec::new();
    };
    if manager.reconcile_profile_generation().is_err() {
        qol_runtime::probe!(
            "TRAY_MCP",
            "event=catalog_skipped reason=profile_reconciliation_failed"
        );
        return Vec::new();
    }
    let catalog = crate::plugins::operation_catalog::installed_catalog(&manager);
    catalog_bindings(&catalog)
}

fn catalog_bindings(catalog: &OperationCatalog) -> Vec<ToolBinding> {
    catalog
        .iter()
        .filter(|operation| operation.agent_tool)
        .filter_map(tool_binding)
        .collect()
}

fn tool_binding(operation: &Operation) -> Option<ToolBinding> {
    let kind = operation.invocation()?;
    Some(ToolBinding {
        plugin_id: operation.plugin_id.as_str().to_string(),
        runable: operation.key.name.clone(),
        kind,
        accepts_agent_home: operation
            .input
            .as_ref()
            .is_some_and(|map| map.contains_key("agent_home")),
        spec: tool_spec(operation),
    })
}

fn tool_spec(operation: &Operation) -> qol_mcp::ToolSpec {
    let published = operation.input.as_ref().map(|map| {
        let mut published = map.clone();
        published.shift_remove("agent_home");
        published
    });
    qol_mcp::ToolSpec {
        name: tool_name(operation.plugin_id.as_str(), &operation.key.name),
        description: operation.tool_description().to_string(),
        input_schema: qol_mcp::input_schema(published.as_ref().unwrap_or(&IndexMap::new())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::manifest::{
        BuildInfo, Capabilities, MenuConfig, PluginId, PluginInfo, PluginManifest, PluginUid,
        CURRENT_MANIFEST_VERSION,
    };
    use qol_config::contract::RuntimeSpec;

    fn manifest_for(plugin_id: &str) -> PluginManifest {
        PluginManifest {
            manifest_version: CURRENT_MANIFEST_VERSION,
            plugin: PluginInfo {
                id: Some(PluginId::new(plugin_id)),
                uid: Some(PluginUid::new(format!("uid-{plugin_id}"))),
                name: "Test".to_string(),
                description: String::new(),
                version: "1.0.0".to_string(),
                author: None,
                platforms: None,
                icon: None,
            },
            menu: MenuConfig {
                label: "Test".to_string(),
                items: Vec::new(),
            },
            daemon: None,
            dependencies: None,
            runtime: None,
            actions: Default::default(),
            capabilities: Capabilities::default(),
            build: BuildInfo::default(),
            traits: None,
            shortcuts: Vec::new(),
            config: Default::default(),
            launcher: None,
        }
    }

    fn catalog_for(plugin_id: &str, runtime: &RuntimeSpec) -> OperationCatalog {
        let manifest = manifest_for(plugin_id);
        OperationCatalog::derive(&PluginId::new(plugin_id), &manifest, Some(runtime))
            .expect("derive catalog")
    }

    fn bindings_for_contract(plugin_id: &str, runtime: &RuntimeSpec) -> Vec<ToolBinding> {
        catalog_bindings(&catalog_for(plugin_id, runtime))
    }

    #[test]
    fn tool_name_joins_plugin_id_and_runable_with_double_underscore() {
        assert_eq!(tool_name("lights", "status"), "lights__status");
    }

    #[test]
    fn shipped_memory_activation_preserves_query_tools_and_typed_capture() {
        let dir = tempfile::TempDir::new().unwrap();
        let _paths = crate::paths::push_test_path_root(dir.path());
        let mut manifest: PluginManifest =
            toml::from_str(include_str!("../../../../../plugins/memory/plugin.toml")).unwrap();
        manifest.plugin.platforms = None;
        let runtime_source = include_str!("../../../../../plugins/memory/qol-runtime.toml");
        let runtime = qol_config::contract::parse_runtime_spec_str(runtime_source).unwrap();
        std::fs::write(dir.path().join("qol-runtime.toml"), runtime_source).unwrap();
        std::fs::write(
            dir.path().join(&manifest.runtime.as_ref().unwrap().command),
            "fixture",
        )
        .unwrap();
        let mut manager = PluginManager::new();
        manager.insert_plugin_for_test(crate::plugins::Plugin::new(
            PluginId::new("qol-memory"),
            manifest,
            dir.path().to_path_buf(),
        ));
        let catalog = crate::plugins::operation_catalog::installed_catalog(&manager);
        let activation = catalog
            .iter()
            .find(|operation| {
                operation.key.name == "status" && operation.invocation() == Some(Invocation::Action)
            })
            .unwrap();
        assert!(!activation.agent_tool);
        assert!(activation.input.is_none());
        let bindings = catalog_bindings(&catalog);
        let names: Vec<_> = bindings
            .iter()
            .map(|binding| binding.spec.name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "qol-memory__ask",
                "qol-memory__status",
                "qol-memory__capture"
            ]
        );
        assert_eq!(bindings[0].kind, Invocation::Query);
        assert_eq!(bindings[1].kind, Invocation::Query);
        assert_eq!(bindings[2].kind, Invocation::Action);
        assert!(bindings[0].accepts_agent_home);
        assert!(!bindings[1].accepts_agent_home);
        assert!(bindings[2].accepts_agent_home);
        let ask = &runtime.queries["ask"];
        let mut published = ask.input.clone().unwrap();
        published.shift_remove("agent_home");
        assert_eq!(
            bindings[0].spec.input_schema,
            qol_mcp::input_schema(&published)
        );
        assert_eq!(
            bindings[0].spec.description,
            *ask.tool_description.as_ref().unwrap()
        );
        assert_eq!(
            bindings[1].spec.description,
            *runtime.queries["status"].tool_description.as_ref().unwrap()
        );
        assert!(catalog.iter().all(|operation| operation.peer.is_none()));
    }

    #[test]
    fn contract_bindings_expose_only_flagged_runables_with_override_description_and_schema() {
        let runtime = qol_config::contract::parse_runtime_spec_str(
            r#"
schema_version = 1

[query.status]
description = "light status query"
poll_interval_ms = 1000
agent_tool = true
tool_description = "Light status"
input = { zone = "Zone to query" }

[query.internal]
description = "internal query"
poll_interval_ms = 1000

[action.blink]
description = "blink action"
agent_tool = true
"#,
        )
        .unwrap();
        let bindings = bindings_for_contract("lights", &runtime);
        assert_eq!(bindings.len(), 2);
        assert_eq!(bindings[0].spec.name, "lights__status");
        assert_eq!(bindings[0].spec.description, "Light status");
        assert_eq!(
            bindings[0].spec.input_schema,
            serde_json::json!({
                "type": "object",
                "properties": {
                    "zone": {
                        "type": "string",
                        "description": "Zone to query"
                    }
                },
                "required": ["zone"]
            })
        );
        assert_eq!(bindings[0].kind, Invocation::Query);
        assert_eq!(bindings[1].spec.name, "lights__blink");
        assert_eq!(bindings[1].spec.description, "blink action");
        assert_eq!(
            bindings[1].spec.input_schema,
            serde_json::json!({"type": "object", "properties": {}, "required": []})
        );
        assert_eq!(bindings[1].kind, Invocation::Action);
    }

    #[test]
    fn peer_exposure_alone_publishes_no_tool() {
        let runtime = qol_config::contract::parse_runtime_spec_str(
            r#"
schema_version = 1

[action.reconnect]
description = "Reconnect devices"
peer = { replay = "idempotent" }

[query.devices]
description = "Device inventory"
poll_interval_ms = 1000
peer = { replay = "idempotent" }
"#,
        )
        .unwrap();
        let catalog = catalog_for("lights", &runtime);
        assert_eq!(catalog.len(), 2);
        assert!(catalog.iter().all(|operation| operation.peer.is_some()));
        assert!(catalog_bindings(&catalog).is_empty());
    }

    #[test]
    fn agent_tools_without_peer_stay_exposed() {
        let runtime = qol_config::contract::parse_runtime_spec_str(
            r#"
schema_version = 1

[action.reconnect]
description = "Reconnect devices"
agent_tool = true
"#,
        )
        .unwrap();
        let catalog = catalog_for("lights", &runtime);
        let entry = catalog
            .iter()
            .find(|operation| operation.key.name == "reconnect")
            .expect("reconnect");
        assert!(entry.peer.is_none());
        let bindings = catalog_bindings(&catalog);
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].spec.name, "lights__reconnect");
    }

    #[test]
    fn manifest_actions_without_a_runtime_contract_publish_no_tool() {
        let mut manifest = manifest_for("lights");
        manifest.actions.insert(
            "blink".to_string(),
            crate::plugins::manifest::ActionDeclaration {
                label: "Blink".to_string(),
                kind: crate::plugins::manifest::ActionType::Run,
                continuous: false,
                args: Some(vec!["blink".to_string()]),
                config_key: None,
                checked: false,
                picture: None,
                hotkey: true,
                peer: None,
            },
        );
        let catalog =
            OperationCatalog::derive(&PluginId::new("lights"), &manifest, None).expect("derive");
        assert_eq!(catalog.len(), 1);
        assert!(catalog_bindings(&catalog).is_empty());
    }

    #[test]
    fn streams_never_publish_a_tool() {
        let runtime = qol_config::contract::parse_runtime_spec_str(
            r#"
schema_version = 1

[stream.live]
description = "Live state"
throttle_ms = 100
"#,
        )
        .unwrap();
        let catalog = catalog_for("lights", &runtime);
        assert_eq!(catalog.len(), 1);
        assert!(catalog_bindings(&catalog).is_empty());
    }

    #[test]
    fn published_schema_omits_the_reserved_agent_home_input() {
        let runtime = qol_config::contract::parse_runtime_spec_str(
            r#"
schema_version = 1

[query.ask]
description = "ask"
poll_interval_ms = 1000
agent_tool = true
tool_description = "Ask memory"
input = { question = "Question to ask", agent_home = "Agent home id" }
"#,
        )
        .unwrap();
        let bindings = bindings_for_contract("memory", &runtime);
        assert_eq!(bindings.len(), 1);
        assert!(bindings[0].accepts_agent_home);
        let properties = bindings[0].spec.input_schema["properties"]
            .as_object()
            .unwrap();
        assert!(properties.contains_key("question"));
        assert!(!properties.contains_key("agent_home"));
        assert_eq!(
            bindings[0].spec.input_schema["required"],
            serde_json::json!(["question"])
        );
    }

    #[test]
    fn agent_home_is_injected_and_overwrites_a_caller_supplied_value() {
        let caller = qol_mcp::Caller {
            agent_home: Some("/home/k/.claude-work".to_owned()),
        };
        let arguments = with_agent_home_argument(
            serde_json::json!({"question": "q", "agent_home": "caller-chosen"}),
            &caller,
            true,
            || false,
        )
        .unwrap();
        assert_eq!(
            arguments,
            serde_json::json!({"question": "q", "agent_home": "/home/k/.claude-work"})
        );
        let untouched = with_agent_home_argument(
            serde_json::json!({"agent_home": "caller-chosen"}),
            &caller,
            false,
            || false,
        )
        .unwrap();
        assert_eq!(
            untouched,
            serde_json::json!({"agent_home": "caller-chosen"})
        );
    }

    #[test]
    fn null_arguments_become_an_object_when_agent_home_is_injected() {
        let caller = qol_mcp::Caller {
            agent_home: Some("/home/k/.claude-work".to_owned()),
        };
        let arguments =
            with_agent_home_argument(serde_json::Value::Null, &caller, true, || false).unwrap();
        assert_eq!(
            arguments,
            serde_json::json!({"agent_home": "/home/k/.claude-work"})
        );
        let untouched =
            with_agent_home_argument(serde_json::Value::Null, &caller, false, || false).unwrap();
        assert_eq!(untouched, serde_json::json!({}));
    }

    #[test]
    fn missing_caller_fails_closed_only_on_a_partitioned_registry() {
        let caller = qol_mcp::Caller::default();
        let error =
            with_agent_home_argument(serde_json::json!({"question": "q"}), &caller, true, || true)
                .unwrap_err();
        assert_eq!(
            error,
            "caller identity missing: no x-qol-agent-home header; run qol mcp configure <harness>, restart the harness, and update the qol CLI if qol mcp headers prints no x-qol-agent-home"
        );
        let forwarded =
            with_agent_home_argument(serde_json::json!({"question": "q"}), &caller, true, || {
                false
            })
            .unwrap();
        assert_eq!(forwarded, serde_json::json!({"question": "q"}));
    }

    #[test]
    fn non_object_arguments_fail_when_injection_is_expected() {
        let caller = qol_mcp::Caller {
            agent_home: Some("/home/k/.claude-work".to_owned()),
        };
        let first = serde_json::json!(["entry"]);
        let error = with_agent_home_argument(first, &caller, true, || false).unwrap_err();
        assert_eq!(error, "arguments must be a JSON object");
        let second = serde_json::json!(["entry"]);
        let forwarded = with_agent_home_argument(second, &caller, false, || false).unwrap();
        assert_eq!(forwarded, serde_json::json!(["entry"]));
    }

    #[test]
    fn calls_revalidate_exposure_after_listing_without_invoking_a_stale_tool() {
        use qol_mcp::ToolHost;

        let dir = tempfile::TempDir::new().unwrap();
        let _paths = crate::paths::push_test_path_root(dir.path());
        let mut manifest = manifest_for("fixture");
        manifest.runtime = Some(crate::plugins::manifest::RuntimeConfig {
            command: "plugin-bin".into(),
            actions: None,
        });
        std::fs::write(dir.path().join("plugin-bin"), "fixture").unwrap();
        let runtime_path = dir.path().join("qol-runtime.toml");
        std::fs::write(
            &runtime_path,
            "schema_version = 1\n[action.blink]\ndescription = \"Blink\"\nagent_tool = true\n",
        )
        .unwrap();
        let mut manager = PluginManager::new();
        manager.insert_plugin_for_test(crate::plugins::Plugin::new(
            PluginId::new("fixture"),
            manifest,
            dir.path().to_path_buf(),
        ));
        let host = PluginToolHost::new(Arc::new(Mutex::new(manager)));
        assert_eq!(host.list()[0].name, "fixture__blink");
        std::fs::write(&runtime_path,
            "schema_version = 1\n[action.blink]\ndescription = \"Blink\"\npeer = { replay = \"never\" }\n"
        ).unwrap();
        let result = host.call(
            "fixture__blink",
            serde_json::json!({}),
            &qol_mcp::Caller::default(),
        );
        assert_eq!(
            result,
            qol_mcp::ToolResult::error("unknown tool: fixture__blink")
        );
        assert!(host.list().is_empty());
    }
}
