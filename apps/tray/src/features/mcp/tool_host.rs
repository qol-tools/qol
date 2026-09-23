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
    accepts_agent_home: bool,
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
            accepts_agent_home: entry
                .input
                .as_ref()
                .is_some_and(|map| map.contains_key("agent_home")),
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
            accepts_agent_home: entry
                .input
                .as_ref()
                .is_some_and(|map| map.contains_key("agent_home")),
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
    let published = input.map(|map| {
        let mut published = map.clone();
        published.shift_remove("agent_home");
        published
    });
    qol_mcp::ToolSpec {
        name: tool_name(plugin_id, runable),
        description: description.to_string(),
        input_schema: qol_mcp::input_schema(published.as_ref().unwrap_or(&IndexMap::new())),
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
}
