use anyhow::{anyhow, bail, Result};
use serde_json::Value;

use crate::commands::sessions::contract::{tool_specs, ToolSpec};

const HEADER: &str = r#"import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { spawnSync } from "node:child_process";
import { Type } from "typebox";

const WAIT_TIMEOUT_MS = 610_000;

function run(args, timeoutMs) {
  const result = spawnSync("qol", ["sessions", ...args], {
    encoding: "utf-8",
    timeout: timeoutMs ?? 60_000,
  });
  if (result.error) {
    throw new Error(`qol sessions failed: ${result.error.message}`);
  }
  if (result.status !== 0) {
    const message = (result.stderr ?? "").trim() || (result.stdout ?? "").trim();
    throw new Error(message || `qol sessions exited with ${result.status}`);
  }
  return (result.stdout ?? "").trim();
}
"#;

const EXECUTE_LIST: &str = r#"    async execute(_toolCallId, _params, _signal, _onUpdate) {
      const stdout = run(["list", "--json"]);
      const rows = JSON.parse(stdout);
      const text = rows
        .map(
          (row) =>
            `${row.session}  ${row.tool ?? "?"}  ${row.display_name ?? ""}${row.cwd ? `  (${row.cwd})` : ""}${row.activity == null ? "" : row.activity ? "  busy" : "  idle"}`,
        )
        .join("\n");
      return { content: [{ type: "text", text: text || "no sessions" }], details: { rows } };
    },
"#;

const EXECUTE_READ: &str = r#"    async execute(_toolCallId, params, _signal, _onUpdate) {
      const text = run(["read", params.session]);
      return { content: [{ type: "text", text }], details: {} };
    },
"#;

const EXECUTE_SEND: &str = r#"    async execute(_toolCallId, params, _signal, _onUpdate) {
      const args = ["send", params.session, params.text];
      if (params.submit) args.push("--submit");
      const text = run(args);
      return { content: [{ type: "text", text }], details: {} };
    },
"#;

const EXECUTE_WAIT: &str = r#"    async execute(_toolCallId, params, _signal, _onUpdate) {
      const args = ["wait", params.session];
      if (params.timeout_ms != null) args.push("--timeout-ms", String(Math.round(params.timeout_ms)));
      if (params.expect) args.push("--expect", params.expect);
      const stdout = run(args, WAIT_TIMEOUT_MS);
      const outcome = JSON.parse(stdout);
      const text = outcome.settled
        ? `settled after ${outcome.elapsed_ms}ms (${outcome.polls} polls)`
        : `timeout after ${outcome.elapsed_ms}ms (${outcome.polls} polls); screen below`;
      return {
        content: [{ type: "text", text: `${text}\n${outcome.screen}` }],
        details: { outcome },
      };
    },
"#;

const EXECUTE_FOCUS: &str = r#"    async execute(_toolCallId, params, _signal, _onUpdate) {
      const text = run(["focus", params.session]);
      return { content: [{ type: "text", text }], details: {} };
    },
"#;

pub(crate) fn pi_extension() -> Result<String> {
    let blocks = tool_specs()
        .iter()
        .map(render_tool_block)
        .collect::<Result<Vec<_>>>()?
        .join("\n\n");
    Ok(format!(
        "{HEADER}\nexport default function sessionsToolsExtension(pi: ExtensionAPI) {{\n{blocks}\n}}\n"
    ))
}

fn render_tool_block(spec: &ToolSpec) -> Result<String> {
    let parameters = typebox_object(&spec.input_schema)?;
    let execute = match spec.name {
        "sessions_list" => EXECUTE_LIST,
        "session_read_screen" => EXECUTE_READ,
        "session_send_text" => EXECUTE_SEND,
        "session_wait_output" => EXECUTE_WAIT,
        "session_focus" => EXECUTE_FOCUS,
        other => bail!("no pi adapter template for contract tool `{other}`"),
    };
    Ok(format!(
        "  pi.registerTool({{\n    name: {:?},\n    label: {:?},\n    description:\n      {:?},\n    parameters: {parameters},\n{execute}  }});",
        spec.name, spec.label, spec.description
    ))
}

fn typebox_object(schema: &Value) -> Result<String> {
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("contract schema must declare an object properties map"))?;
    if properties.is_empty() {
        return Ok("Type.Object({})".to_owned());
    }
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|values| values.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    let mut fields = Vec::new();
    for (name, property) in properties {
        let expr = typebox_expr(property, required.contains(&name.as_str()))?;
        fields.push(format!("      {name}: {expr},"));
    }
    Ok(format!("Type.Object({{\n{}\n    }})", fields.join("\n")))
}

fn typebox_expr(property: &Value, required: bool) -> Result<String> {
    let type_name = property
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("contract property must declare a type"))?;
    let description = property.get("description").and_then(Value::as_str);
    let args = description.map_or_else(String::new, |text| format!("{{ description: {text:?} }}"));
    let inner = match type_name {
        "string" => format!("Type.String({args})"),
        "boolean" => format!("Type.Boolean({args})"),
        "integer" => format!("Type.Integer({args})"),
        "object" => typebox_object(property)?,
        other => bail!("unsupported contract schema type `{other}`"),
    };
    if required {
        Ok(inner)
    } else {
        Ok(format!("Type.Optional({inner})"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pi_adapter_covers_every_contract_tool() {
        let source = pi_extension().expect("pi render must succeed");
        for spec in tool_specs() {
            assert!(
                source.contains(&format!("name: {:?}", spec.name)),
                "rendered extension must contain tool {}",
                spec.name
            );
        }
    }

    #[test]
    fn pi_rendering_is_byte_stable() {
        let first = pi_extension().expect("render");
        let second = pi_extension().expect("render");
        assert_eq!(first, second);
    }

    #[test]
    fn pi_schema_maps_integer_and_optional() {
        let specs = tool_specs();
        let spec = specs
            .iter()
            .find(|spec| spec.name == "session_wait_output")
            .expect("wait_output in contract");
        let source = render_tool_block(spec).expect("render");
        assert!(source.contains("timeout_ms: Type.Optional(Type.Integer({ description: \"Timeout in milliseconds, clamped 1000..600000 (default 30000)\" })),"));
        assert!(source.contains("expect: Type.Optional(Type.String({ description: \"Substring to wait for in the screen; the echo of the last-sent text does not count\" })),"));
        assert!(source.contains(
            "session: Type.String({ description: \"Stable session token from sessions_list\" }),"
        ));
    }
}
