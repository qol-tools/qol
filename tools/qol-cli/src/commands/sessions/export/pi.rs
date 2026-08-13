use anyhow::{anyhow, bail, Result};
use serde_json::Value;

use crate::commands::sessions::contract::{tool_specs, ToolSpec};

const HEADER: &str = r#"import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { spawn } from "node:child_process";
import { Type } from "typebox";

const BRIDGE_TIMEOUT_MS = 86_410_000;
const LOOP_ENTRY = "qol-sessions-feature-loop";
const LOOP_PHASES = new Set(["idle", "waiting", "review", "closing", "paused"]);
const REVIEW_FOLLOW_UP = `The qol-sessions feature loop is still active. Personally inspect the implementation against the user's complete acceptance criteria. If anything remains, call session_bridge for the next bounded correction round and acknowledge the reviewed completion_marker. If the entire feature is accepted, call session_loop_close with the session, completion_marker, outcome accepted, landed, before, now, verification, and remaining. If the user redirected the work or a genuine blocker requires user input, call session_loop_close with the session, completion_marker, outcome paused, and unfinished scope under remaining. Do not stop at a round boundary.`;
const FINAL_REPORT_FOLLOW_UP = `The qol-sessions feature loop is closing. Return the exact canonical final report emitted by session_loop_close. Do not add or remove sections.`;

function run(args, timeoutMs, input, signal) {
  return new Promise((resolve, reject) => {
    const child = spawn("qol", ["sessions", ...args], { stdio: ["pipe", "pipe", "pipe"] });
    let stdout = "";
    let stderr = "";
    let settled = false;
    let timer = null;
    const settle = (fn) => {
      if (settled) return;
      settled = true;
      if (timer !== null) clearTimeout(timer);
      signal?.removeEventListener("abort", onAbort);
      fn();
    };
    const onAbort = () => {
      child.kill("SIGTERM");
      settle(() => reject(new Error("qol sessions aborted by the host")));
    };
    timer = setTimeout(() => {
      child.kill("SIGTERM");
      settle(() => reject(new Error(`qol sessions timed out after ${timeoutMs ?? 60_000}ms`)));
    }, timeoutMs ?? 60_000);
    if (signal?.aborted) {
      onAbort();
      return;
    }
    signal?.addEventListener("abort", onAbort, { once: true });
    child.stdout.on("data", (chunk) => { stdout += chunk; });
    child.stderr.on("data", (chunk) => { stderr += chunk; });
    child.on("error", (error) => settle(() => reject(new Error(`qol sessions failed: ${error.message}`))));
    child.on("close", (code, childSignal) => settle(() => {
      if (childSignal) {
        reject(new Error(`qol sessions exited with ${childSignal}`));
        return;
      }
      if (code !== 0) {
        const message = stderr.trim() || stdout.trim();
        reject(new Error(message || `qol sessions exited with ${code}`));
        return;
      }
      resolve(stdout.trim());
    }));
    child.stdin.end(input ?? "");
  });
}

function assistantText(messages) {
  return messages
    .filter((message) => message?.role === "assistant")
    .flatMap((message) =>
      typeof message.content === "string"
        ? [message.content]
        : Array.isArray(message.content)
          ? message.content
              .filter((block) => block?.type === "text" && typeof block.text === "string")
              .map((block) => block.text)
          : [],
    )
    .join("\n");
}
"#;

const LOOP_SETUP: &str = r#"  let loopPhase = "idle";
  let loopFinalReport = "";
  let closingFollowUpSent = false;

  function setLoopPhase(phase, finalReport = "") {
    if (loopPhase === phase && loopFinalReport === finalReport) return;
    loopPhase = phase;
    loopFinalReport = finalReport;
    pi.appendEntry(LOOP_ENTRY, { phase, final_report: finalReport });
  }

  function normalized(text) {
    return text.replace(/[^a-z0-9]/gi, "").toLowerCase();
  }

  function restoreLoopPhase(ctx) {
    const entry = [...ctx.sessionManager.getBranch()]
      .reverse()
      .find((candidate) => candidate?.type === "custom" && candidate.customType === LOOP_ENTRY);
    const restored = entry?.data?.phase;
    loopPhase = LOOP_PHASES.has(restored) ? restored : "idle";
    loopFinalReport = typeof entry?.data?.final_report === "string" ? entry.data.final_report : "";
    if (loopPhase === "waiting") setLoopPhase("paused");
    if (loopPhase === "closing") setLoopPhase("idle");
  }

  pi.on("session_start", async (_event, ctx) => {
    restoreLoopPhase(ctx);
  });

  pi.on("session_tree", async (_event, ctx) => {
    restoreLoopPhase(ctx);
  });

  pi.on("agent_end", async (event, _ctx) => {
    const text = assistantText(event.messages);
    if (loopPhase === "closing" && loopFinalReport && normalized(text).includes(normalized(loopFinalReport))) {
      setLoopPhase("idle");
    }
  });

  pi.on("agent_settled", async (_event, _ctx) => {
    if (loopPhase === "review") {
      pi.sendUserMessage(REVIEW_FOLLOW_UP, { deliverAs: "followUp" });
    }
    if (loopPhase === "closing") {
      if (!closingFollowUpSent) {
        closingFollowUpSent = true;
        pi.sendUserMessage(FINAL_REPORT_FOLLOW_UP, { deliverAs: "followUp" });
      } else {
        setLoopPhase("idle");
      }
    }
  });
"#;

const EXECUTE_LIST: &str = r#"    async execute(_toolCallId, _params, signal, _onUpdate) {
      const stdout = await run(["list", "--json"], 60_000, undefined, signal);
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

const EXECUTE_SPAWN: &str = r#"    async execute(_toolCallId, params, signal, _onUpdate) {
      const args = ["spawn", "--tool", params.tool, "--cwd", params.cwd, "--key", params.key];
      if (params.surface != null) args.push("--surface", params.surface);
      if (params.model != null) args.push("--model", params.model);
      if (params.title != null) args.push("--title", params.title);
      if (params.task != null) args.push("--task", params.task);
      const stdout = await run(args, 60_000, undefined, signal);
      const outcome = JSON.parse(stdout);
      const text = outcome.reused
        ? `reused session ${outcome.session} (${outcome.tool}, key ${outcome.key}, ${outcome.cwd})`
        : `spawned session ${outcome.session} (${outcome.tool}, key ${outcome.key}, ${outcome.cwd}, ${outcome.surface})`
          + (outcome.task_submitted ? "; first round delivered, wait with session_bridge (omit task)" : "");
      return { content: [{ type: "text", text }], details: { outcome } };
    },
"#;

const EXECUTE_SUBMIT: &str = r#"    async execute(_toolCallId, params, signal, _onUpdate) {
      const args = ["submit", params.session, "--task", params.task];
      if (params.acknowledge_marker != null) args.push("--acknowledge-marker", params.acknowledge_marker);
      const stdout = await run(args, 60_000, undefined, signal);
      const outcome = JSON.parse(stdout);
      const text = `task submitted to session ${outcome.session}; round open, wait with session_bridge (omit task)`;
      return { content: [{ type: "text", text: `${text}\n${outcome.screen}` }], details: { outcome } };
    },
"#;

const EXECUTE_BRIDGE: &str = r#"    async execute(_toolCallId, params, signal, _onUpdate) {
      const args = ["bridge", params.session];
      if (params.acknowledge_marker != null) args.push("--acknowledge-marker", params.acknowledge_marker);
      if (params.task != null) args.push("--", params.task);
      setLoopPhase("waiting");
      try {
        const stdout = await run(args, BRIDGE_TIMEOUT_MS, undefined, signal);
        const outcome = JSON.parse(stdout);
        setLoopPhase(outcome.completed ? "review" : "paused");
        const text = outcome.completed
          ? outcome.submitted
            ? `implementation completed after ${outcome.elapsed_ms}ms (${outcome.reads} screen reads)`
            : `recovered the previous implementation response before submitting new work after ${outcome.elapsed_ms}ms (${outcome.reads} screen reads)`
          : `bridge timed out after ${outcome.elapsed_ms}ms; do not resend the task`;
        return {
          content: [{ type: "text", text: `${text}\n${outcome.screen}` }],
          details: { outcome },
        };
      } catch (error) {
        setLoopPhase("paused");
        throw error;
      }
    },
"#;

const EXECUTE_LOOP_CLOSE: &str = r#"    async execute(_toolCallId, params, signal, _onUpdate) {
      const request = { jsonrpc: "2.0", id: 1, method: "tools/call", params: { name: "session_loop_close", arguments: params } };
      const response = JSON.parse(await run(["mcp"], 10_000, `${JSON.stringify(request)}\n`, signal));
      const result = response?.result;
      const text = result?.content?.[0]?.text;
      if (result?.isError || typeof text !== "string") throw new Error(text || "session_loop_close failed");
      const receipt = JSON.parse(text);
      setLoopPhase("closing", receipt.final_report);
      return {
        content: [{ type: "text", text: JSON.stringify(receipt) }],
        details: { receipt },
      };
    },
"#;

const EXECUTE_CLOSE: &str = r#"    async execute(_toolCallId, params, signal, _onUpdate) {
      const stdout = await run(["close", params.session], 30_000, undefined, signal);
      const outcome = JSON.parse(stdout);
      return {
        content: [{ type: "text", text: `closed session ${outcome.session} (${outcome.tool}, key ${outcome.key})` }],
        details: { outcome },
      };
    },
"#;

pub(crate) fn pi_extension() -> Result<String> {
    let blocks = tool_specs()
        .iter()
        .map(render_tool_block)
        .collect::<Result<Vec<_>>>()?
        .join("\n\n");
    Ok(format!(
        "{HEADER}\nexport default function sessionsToolsExtension(pi: ExtensionAPI) {{\n{LOOP_SETUP}\n{blocks}\n}}\n"
    ))
}

fn render_tool_block(spec: &ToolSpec) -> Result<String> {
    let parameters = typebox_object(&spec.input_schema)?;
    let execute = match spec.name {
        "sessions_list" => EXECUTE_LIST,
        "session_spawn" => EXECUTE_SPAWN,
        "session_submit" => EXECUTE_SUBMIT,
        "session_bridge" => EXECUTE_BRIDGE,
        "session_loop_close" => EXECUTE_LOOP_CLOSE,
        "session_close" => EXECUTE_CLOSE,
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
    fn pi_schema_maps_the_bridge_task_without_a_round_deadline() {
        let specs = tool_specs();
        let spec = specs
            .iter()
            .find(|spec| spec.name == "session_bridge")
            .expect("bridge in contract");
        let source = render_tool_block(spec).expect("render");
        assert!(!source.contains("timeout_ms"));
        assert!(!EXECUTE_BRIDGE.contains("timeout_ms"));
        assert!(source.contains(
            "session: Type.String({ description: \"Stable session token from sessions_list\" }),"
        ));
        assert!(source.contains(
            "task: Type.Optional(Type.String({ description: \"Bounded implementation task to submit exactly once after any pending response is acknowledged; omit to wait for the round a prior session_submit or spawn task left open\" })),"
        ));
        assert!(source.contains("acknowledge_marker: Type.Optional(Type.String"));
        assert!(source.contains("args.push(\"--acknowledge-marker\""));
        assert!(source.contains("if (params.task != null) args.push(\"--\", params.task)"));
    }

    #[test]
    fn pi_adapter_keeps_the_feature_loop_armed_through_review() {
        let source = pi_extension().expect("render");
        assert!(source.contains("setLoopPhase(\"waiting\")"));
        assert!(source.contains("setLoopPhase(outcome.completed ? \"review\" : \"paused\")"));
        assert!(source.contains("recovered the previous implementation response"));
        assert!(source.contains("pi.on(\"agent_settled\""));
        assert!(source.contains("pi.sendUserMessage(REVIEW_FOLLOW_UP"));
    }

    #[test]
    fn pi_schema_maps_the_spawn_tool_with_required_key() {
        let specs = tool_specs();
        let spec = specs
            .iter()
            .find(|spec| spec.name == "session_spawn")
            .expect("spawn in contract");
        let source = render_tool_block(spec).expect("render");
        assert!(source.contains(
            "tool: Type.String({ description: \"Registered CLI tool to spawn (codex, claude, pi, kimi)\" }),"
        ));
        assert!(source.contains(
            "cwd: Type.String({ description: \"Working directory for the spawned session\" }),"
        ));
        assert!(source.contains(
            "key: Type.String({ description: \"Stable spawn key; required so retries are idempotent\" }),"
        ));
        assert!(source.contains("surface: Type.Optional(Type.String"));
        assert!(EXECUTE_SPAWN.contains("args.push(\"--surface\", params.surface)"));
        assert!(EXECUTE_SPAWN.contains("outcome.reused"));
        assert!(EXECUTE_SPAWN.contains("spawned session ${outcome.session}"));
    }

    #[test]
    fn pi_adapter_closes_the_loop_through_a_typed_receipt() {
        let specs = tool_specs();
        let spec = specs
            .iter()
            .find(|spec| spec.name == "session_loop_close")
            .expect("loop close in contract");
        let source = render_tool_block(spec).expect("render");
        assert!(source.contains("name: \"session_loop_close\""));
        assert!(source.contains("run([\"mcp\"], 10_000"));
        assert!(source.contains("setLoopPhase(\"closing\", receipt.final_report)"));
    }

    #[test]
    fn pi_adapter_closes_the_loop_after_a_single_report_echo() {
        let source = pi_extension().expect("render");
        assert!(source.contains("function normalized(text)"));
        assert!(source.contains("normalized(text).includes(normalized(loopFinalReport))"));
        assert!(source.contains("let closingFollowUpSent = false;"));
        assert!(source.contains("if (!closingFollowUpSent) {"));
        assert!(source.contains("closingFollowUpSent = true;"));
        assert!(source.contains("if (loopPhase === \"closing\") setLoopPhase(\"idle\")"));
        assert!(LOOP_SETUP.contains("} else {\n        setLoopPhase(\"idle\")"));
        assert!(LOOP_SETUP
            .contains("pi.sendUserMessage(FINAL_REPORT_FOLLOW_UP, { deliverAs: \"followUp\" })"));
        assert!(!source.contains("FINAL_REPORT_FOLLOW_UP}\n\n${loopFinalReport}"));
    }

    #[test]
    fn pi_adapter_never_blocks_the_host_event_loop() {
        let source = pi_extension().expect("render");
        assert!(!source.contains("spawnSync"));
        assert!(source.contains("import { spawn } from \"node:child_process\";"));
        assert!(source.contains("signal?.addEventListener(\"abort\", onAbort"));
        assert!(source.contains("await run(args, BRIDGE_TIMEOUT_MS, undefined, signal)"));
        assert!(EXECUTE_BRIDGE.contains("setLoopPhase(\"waiting\")"));
        for template in [
            EXECUTE_LIST,
            EXECUTE_SPAWN,
            EXECUTE_SUBMIT,
            EXECUTE_BRIDGE,
            EXECUTE_LOOP_CLOSE,
            EXECUTE_CLOSE,
        ] {
            assert!(
                template.contains("signal") && template.contains("await run("),
                "every pi tool must await the async child run with the host signal"
            );
        }
    }
}
