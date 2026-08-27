use crate::plugins::PluginManager;
use qol_config::contract::{IndexMap, RuntimeSpec};
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

    fn call(&self, name: &str, arguments: serde_json::Value) -> qol_mcp::ToolResult {
        let binding = match bindings(&self.plugin_manager)
            .into_iter()
            .find(|binding| binding.spec.name == name)
        {
            Some(binding) => binding,
            None => return qol_mcp::ToolResult::error(format!("unknown tool: {name}")),
        };
        qol_runtime::probe!(
            "TRAY_MCP",
            "event=tool_called plugin={} runable={} kind={}",
            binding.plugin_id,
            binding.runable,
            kind_label(&binding.kind)
        );
        match &binding.kind {
            RunableKind::Query => {
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
            RunableKind::Action => {
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

fn kind_label(kind: &RunableKind) -> &'static str {
    match kind {
        RunableKind::Query => "query",
        RunableKind::Action => "action",
    }
}

enum RunableKind {
    Query,
    Action,
}

struct ToolBinding {
    plugin_id: String,
    runable: String,
    kind: RunableKind,
    spec: qol_mcp::ToolSpec,
}

fn tool_name(plugin_id: &str, runable: &str) -> String {
    format!("{plugin_id}__{runable}")
}

fn bindings(plugin_manager: &Arc<Mutex<PluginManager>>) -> Vec<ToolBinding> {
    let Ok(manager) = plugin_manager.lock() else {
        return Vec::new();
    };
    manager
        .plugins()
        .filter_map(|plugin| {
            let plugin_id = plugin.id.as_str().to_string();
            match crate::plugins::config::load_runable_contract_from_root(&plugin.path) {
                Ok(Some(runtime)) => Some(bindings_for_contract(&plugin_id, &runtime)),
                Ok(None) => {
                    qol_runtime::probe!("TRAY_MCP", "event=contract_skipped plugin={}", plugin_id);
                    None
                }
                Err(error) => {
                    qol_runtime::probe!(
                        "TRAY_MCP",
                        "event=contract_skipped plugin={} error={}",
                        plugin_id,
                        error
                    );
                    None
                }
            }
        })
        .flatten()
        .collect()
}

fn bindings_for_contract(plugin_id: &str, runtime: &RuntimeSpec) -> Vec<ToolBinding> {
    let mut result = Vec::new();
    for (name, entry) in &runtime.queries {
        if !entry.agent_tool {
            continue;
        }
        result.push(ToolBinding {
            plugin_id: plugin_id.to_string(),
            runable: name.clone(),
            kind: RunableKind::Query,
            spec: tool_spec(
                plugin_id,
                name,
                entry.tool_description(),
                entry.input.as_ref(),
            ),
        });
    }
    for (name, entry) in &runtime.actions {
        if !entry.agent_tool {
            continue;
        }
        result.push(ToolBinding {
            plugin_id: plugin_id.to_string(),
            runable: name.clone(),
            kind: RunableKind::Action,
            spec: tool_spec(
                plugin_id,
                name,
                entry.tool_description(),
                entry.input.as_ref(),
            ),
        });
    }
    result
}

fn tool_spec(
    plugin_id: &str,
    runable: &str,
    description: &str,
    input: Option<&IndexMap<String, String>>,
) -> qol_mcp::ToolSpec {
    qol_mcp::ToolSpec {
        name: tool_name(plugin_id, runable),
        description: description.to_string(),
        input_schema: qol_mcp::input_schema(input.unwrap_or(&IndexMap::new())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_name_joins_plugin_id_and_runable_with_double_underscore() {
        assert_eq!(tool_name("lights", "status"), "lights__status");
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
        assert!(matches!(&bindings[0].kind, RunableKind::Query));
        assert_eq!(bindings[1].spec.name, "lights__blink");
        assert_eq!(bindings[1].spec.description, "blink action");
        assert_eq!(
            bindings[1].spec.input_schema,
            serde_json::json!({"type": "object", "properties": {}, "required": []})
        );
        assert!(matches!(&bindings[1].kind, RunableKind::Action));
    }
}
