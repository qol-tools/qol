use anyhow::{anyhow, bail, Result};
use serde_json::Value;

use crate::commands::sessions::contract::{tool_specs, ToolSpec};

const HEADER: &str = r#"import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { spawn } from "node:child_process";
import * as fs from "node:fs";
import * as fsp from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
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
  let reviewFollowUpSent = false;

  function setLoopPhase(phase, finalReport = "") {
    if (loopPhase === phase && loopFinalReport === finalReport) return;
    if (loopPhase === "review" && phase !== "review") reviewFollowUpSent = false;
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
    await startWatcher(ctx);
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
      if (!reviewFollowUpSent) {
        reviewFollowUpSent = true;
        pi.sendUserMessage(REVIEW_FOLLOW_UP, { deliverAs: "followUp" });
      }
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

const WATCH_GLUE: &str = r#"  function sessionsDir() {
    const base = process.env.XDG_DATA_HOME ?? path.join(os.homedir(), ".local", "share");
    return path.join(base, "qol-tray", "sessions");
  }

  function watchStateFile(sessionId) {
    return path.join(sessionsDir(), `watch-owner-${sessionId}.json`);
  }

  function wakeDebugLog(sessionId, line) {
    try {
      const logPath = path.join(sessionsDir(), `wake-debug-${sessionId}.log`);
      fs.appendFileSync(logPath, `${new Date().toISOString()} ${line}\n`);
    } catch {}
  }

  async function readWatchedTokens(sessionId) {
    try {
      const parsed = JSON.parse(await fsp.readFile(watchStateFile(sessionId), "utf8"));
      return Array.isArray(parsed) ? parsed.filter((token) => typeof token === "string") : [];
    } catch {}
    return [];
  }

  function dropWatchedToken(sessionId, token) {
    let tokens = [];
    try {
      const parsed = JSON.parse(fs.readFileSync(watchStateFile(sessionId), "utf8"));
      tokens = Array.isArray(parsed) ? parsed.filter((candidate) => typeof candidate === "string") : [];
    } catch {}
    const remaining = tokens.filter((candidate) => candidate !== token);
    if (remaining.length === tokens.length) return null;
    try {
      fs.writeFileSync(watchStateFile(sessionId), JSON.stringify(remaining));
    } catch {
      return null;
    }
    return remaining;
  }

  async function recordWatchedToken(sessionId, token) {
    try {
      await fsp.mkdir(sessionsDir(), { recursive: true });
      const tokens = await readWatchedTokens(sessionId);
      if (!tokens.includes(token)) tokens.push(token);
      await fsp.writeFile(watchStateFile(sessionId), JSON.stringify(tokens));
      if (watcherChild !== null && watcherChild.exitCode == null) {
        watcherChild.kill("SIGTERM");
        watcherChild = null;
      }
    } catch {}
  }

  let watcherChild: ReturnType<typeof spawn> | null = null;
  let stdoutBuffer = "";

  async function startWatcher(ctx) {
    if (watcherChild !== null && watcherChild.exitCode == null) return;
    const sessionId = ctx.sessionManager.getSessionId();
    if (!sessionId) return;
    const tokens = await readWatchedTokens(sessionId);
    if (tokens.length === 0) return;
    wakeDebugLog(sessionId, `watch start tokens=${tokens.length}`);
    try {
      const child = spawn("qol", ["sessions", "watch", ...tokens], {
        detached: true,
        stdio: ["ignore", "pipe", "pipe"],
      });
      watcherChild = child;
      wakeDebugLog(sessionId, `watch spawn pid=${child.pid}`);
      child.stdout.on("data", async (chunk) => {
        stdoutBuffer += chunk.toString();
        const lines = stdoutBuffer.split("\n");
        stdoutBuffer = lines.pop() ?? "";
        wakeDebugLog(sessionId, `chunk bytes=${chunk.length} buffer=${stdoutBuffer.length}`);
        for (const line of lines) {
          const trimmed = line.trim();
          if (!trimmed) continue;
          let event;
          try {
            event = JSON.parse(trimmed);
          } catch {}
          if (typeof event?.event !== "string" || typeof event?.session !== "string") continue;
          wakeDebugLog(sessionId, `event=${event.event} session=${event.session} delivered=${event.delivered === true}${typeof event.wake_error === "string" ? ` error=${event.wake_error}` : ""} screen=${typeof event.screen === "string" ? event.screen.length : 0}`);
          const remaining = dropWatchedToken(sessionId, event.session);
          if (remaining === null) {
            wakeDebugLog(sessionId, `delivery skip session=${event.session} reason=already_delivered`);
            continue;
          }
          wakeDebugLog(sessionId, `token removed remaining=${remaining.length}`);
          if (watcherChild !== null && watcherChild.exitCode == null) {
            watcherChild.kill("SIGTERM");
            watcherChild = null;
          }
          if (remaining.length > 0) {
            await startWatcher(ctx);
          }
          if (event.delivered === false) {
            wakeDebugLog(sessionId, `wake undeliverable session=${event.session} event=${event.event} error=${typeof event.wake_error === "string" ? event.wake_error : "unknown"}`);
          }
        }
      });
      child.on("error", (error) => {
        wakeDebugLog(sessionId, `watch child error: ${error?.message ?? error}`);
        if (watcherChild === child) watcherChild = null;
      });
      child.on("exit", (code, signal) => {
        wakeDebugLog(sessionId, `watch child exit code=${code} signal=${signal}`);
        if (watcherChild === child) watcherChild = null;
      });
    } catch {}
  }

  function stopWatcher() {
    try {
      if (watcherChild !== null) {
        watcherChild.kill("SIGTERM");
        watcherChild = null;
      }
    } catch {}
  }

  pi.on("session_shutdown", async () => {
    stopWatcher();
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

const EXECUTE_SPAWN: &str = r#"    async execute(_toolCallId, params, signal, _onUpdate, ctx) {
      const args = ["spawn", "--tool", params.tool, "--cwd", params.cwd, "--key", params.key];
      if (params.surface != null) args.push("--surface", params.surface);
      if (params.model != null) args.push("--model", params.model);
      if (params.title != null) args.push("--title", params.title);
      args.push("--task", params.task, "--background");
      if (params.resume === true) args.push("--resume");
      const stdout = await run(args, 60_000, undefined, signal);
      const outcome = JSON.parse(stdout);
      await recordWatchedToken(ctx.sessionManager.getSessionId(), outcome.session);
      await startWatcher(ctx);
      const text = `spawned session ${outcome.session} in the background (${outcome.tool}, key ${outcome.key}); round queued, you will be woken when it completes`;
      return { content: [{ type: "text", text }], details: { outcome } };
    },
"#;

const EXECUTE_SUBMIT: &str = r#"    async execute(_toolCallId, params, signal, _onUpdate) {
      const args = ["submit", params.session, "--task", params.task];
      if (params.acknowledge_marker != null) args.push("--acknowledge-marker", params.acknowledge_marker);
      const stdout = await run(args, 60_000, undefined, signal);
      const outcome = JSON.parse(stdout);
      reviewFollowUpSent = false;
      const text = `task submitted to session ${outcome.session}; round open, wait with session_bridge (omit task)`;
      return { content: [{ type: "text", text: `${text}\n${outcome.screen}` }], details: { outcome } };
    },
"#;

const EXECUTE_BRIDGE: &str = r#"    async execute(_toolCallId, params, signal, _onUpdate) {
      const args = ["bridge", params.session];
      setLoopPhase("waiting");
      try {
        const stdout = await run(args, BRIDGE_TIMEOUT_MS, undefined, signal);
        const outcome = JSON.parse(stdout);
        setLoopPhase(outcome.completed ? "review" : "paused");
        const text = outcome.completed
          ? `implementation completed after ${outcome.elapsed_ms}ms (${outcome.reads} screen reads)`
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
        content: [{ type: "text", text: typeof receipt.final_report === "string" ? receipt.final_report : JSON.stringify(receipt) }],
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
        "{HEADER}\nexport default function sessionsToolsExtension(pi: ExtensionAPI) {{\n{LOOP_SETUP}\n{WATCH_GLUE}\n{blocks}\n}}\n"
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
    fn pi_schema_maps_the_bridge_as_a_collect_only_tool_without_a_deadline() {
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
        assert!(
            !source.contains("task:"),
            "bridge takes no task property: delivery belongs to spawn and submit"
        );
        assert!(!source.contains("acknowledge_marker:"));
        assert!(!EXECUTE_BRIDGE.contains("--acknowledge-marker"));
        assert!(!EXECUTE_BRIDGE.contains("params.task"));
        assert!(EXECUTE_BRIDGE.contains("const args = [\"bridge\", params.session];"));
    }

    #[test]
    fn pi_adapter_restarts_the_watcher_when_a_new_round_is_recorded() {
        let record = WATCH_GLUE
            .split("async function recordWatchedToken")
            .nth(1)
            .expect("recordWatchedToken present")
            .split("async function reportSnippet")
            .next()
            .expect("recordWatchedToken body");
        assert!(record.contains("watcherChild.kill(\"SIGTERM\")"));
        assert!(record.contains("watcherChild = null"));
        assert!(WATCH_GLUE
            .contains("if (watcherChild !== null && watcherChild.exitCode == null) return;"));
        assert!(WATCH_GLUE.contains("const tokens = await readWatchedTokens(sessionId);"));
    }

    #[test]
    fn pi_adapter_prunes_delivered_tokens_and_respawns_the_watcher() {
        assert!(WATCH_GLUE.contains("function dropWatchedToken(sessionId, token)"));
        assert!(WATCH_GLUE.contains("fs.readFileSync(watchStateFile(sessionId), \"utf8\")"));
        assert!(WATCH_GLUE
            .contains("fs.writeFileSync(watchStateFile(sessionId), JSON.stringify(remaining))"));
        assert!(WATCH_GLUE.contains("if (remaining === null)"));
        assert!(WATCH_GLUE.contains("reason=already_delivered"));
        assert!(WATCH_GLUE.contains("token removed remaining=${remaining.length}"));
        assert!(WATCH_GLUE.contains("if (remaining.length > 0)"));
        assert!(WATCH_GLUE.contains("await startWatcher(ctx)"));
        assert!(WATCH_GLUE.contains("pi.on(\"session_shutdown\", async () => {"));
    }

    #[test]
    fn pi_adapter_spawns_the_watcher_and_lets_it_deliver_into_the_initiator_terminal() {
        let source = pi_extension().expect("render");
        assert!(source.contains("pi.on(\"session_start\""));
        assert!(source.contains("await startWatcher(ctx)"));
        assert!(
            !WATCH_GLUE.contains("sendUserMessage"),
            "the pi glue must not send wakes itself; the watcher owns delivery"
        );
        assert!(source.contains("delivered=${event.delivered === true}"));
        assert!(source.contains("event.wake_error"));
        assert!(source.contains("pi.on(\"session_shutdown\""));
        assert!(WATCH_GLUE.contains("watcherChild.kill(\"SIGTERM\")"));
        assert!(WATCH_GLUE.contains("detached: true"));
        assert!(WATCH_GLUE.contains("stdio: [\"ignore\", \"pipe\", \"pipe\"]"));
    }

    #[test]
    fn pi_adapter_buffers_fragmented_watcher_stdout_lines() {
        let source = pi_extension().expect("render");
        assert!(source.contains("let stdoutBuffer = \"\";"));
        assert!(source.contains("stdoutBuffer += chunk.toString();"));
        assert!(source.contains("const lines = stdoutBuffer.split(\"\\n\");"));
        assert!(source.contains("stdoutBuffer = lines.pop() ?? \"\";"));
        assert!(WATCH_GLUE.contains("for (const line of lines)"));
        assert!(WATCH_GLUE.contains("const trimmed = line.trim();"));
        assert!(WATCH_GLUE.contains("JSON.parse(trimmed)"));
        assert!(WATCH_GLUE.contains("typeof event?.event !== \"string\""));
        assert!(
            !WATCH_GLUE.contains("sendUserMessage"),
            "delivery moved into the watcher; the glue only prunes tokens"
        );
    }

    #[test]
    fn pi_adapter_wake_debug_log_covers_every_delivery_step() {
        let source = pi_extension().expect("render");
        assert!(WATCH_GLUE.contains("function wakeDebugLog(sessionId, line)"));
        assert!(WATCH_GLUE.contains("wake-debug-${sessionId}.log"));
        assert!(WATCH_GLUE.contains("fs.appendFileSync(logPath"));
        assert!(WATCH_GLUE.contains("watch start tokens=${tokens.length}"));
        assert!(WATCH_GLUE.contains("watch spawn pid=${child.pid}"));
        assert!(WATCH_GLUE.contains("watch child error: ${error?.message ?? error}"));
        assert!(WATCH_GLUE.contains("watch child exit code=${code} signal=${signal}"));
        assert!(WATCH_GLUE.contains("chunk bytes=${chunk.length} buffer=${stdoutBuffer.length}"));
        assert!(WATCH_GLUE.contains("event=${event.event} session=${event.session} delivered="));
        assert!(WATCH_GLUE.contains("event.wake_error"));
        assert!(WATCH_GLUE.contains("wake undeliverable session=${event.session}"));
        assert!(source.contains("import * as fs from \"node:fs\";"));
        assert!(source.contains("import * as fsp from \"node:fs/promises\";"));
    }

    #[test]
    fn pi_adapter_always_spawns_in_background_and_records_the_round_before_the_watcher() {
        let specs = tool_specs();
        let spec = specs
            .iter()
            .find(|spec| spec.name == "session_spawn")
            .expect("spawn in contract");
        let source = render_tool_block(spec).expect("render");
        let extension = pi_extension().expect("render");
        assert!(
            !source.contains("background:"),
            "the schema no longer accepts a background flag"
        );
        assert!(source.contains(
            "task: Type.String({ description: \"Required bounded first-round task embedded in the launch"
        ));
        assert!(
            EXECUTE_SPAWN.contains("args.push(\"--task\", params.task, \"--background\")"),
            "every spawn embeds the task and runs in the background"
        );
        assert!(EXECUTE_SPAWN.contains(
            "await recordWatchedToken(ctx.sessionManager.getSessionId(), outcome.session)"
        ));
        assert!(EXECUTE_SPAWN.contains("await startWatcher(ctx)"));
        assert!(EXECUTE_SPAWN.contains("round queued, you will be woken when it completes"));
        assert!(extension.contains("watch-owner-${sessionId}.json"));
    }

    #[test]
    fn pi_adapter_never_passes_an_autoclose_flag_and_lets_the_watcher_announce_the_closed_lane() {
        assert!(
            !EXECUTE_SPAWN.contains("--auto-close"),
            "the removed autoclose knob must not reach the CLI"
        );
        assert!(
            !EXECUTE_SPAWN.contains("params.autoclose"),
            "the removed autoclose knob must not be read from params"
        );
        assert!(EXECUTE_SPAWN.contains("args.push(\"--task\", params.task, \"--background\")"));
        assert!(
            !WATCH_GLUE.contains("lane auto-closed"),
            "the closed-lane note now lives in the watcher's wake message"
        );
    }

    #[test]
    fn pi_adapter_passes_spawn_resume_through_as_an_opt_in_flag() {
        assert!(
            EXECUTE_SPAWN.contains("if (params.resume === true) args.push(\"--resume\")"),
            "spawn resume is opt-in, so only an explicit request pushes the flag"
        );
        assert!(
            !EXECUTE_SPAWN.contains("params.autoclose"),
            "the resume passthrough must not reintroduce the autoclose knob"
        );
    }

    #[test]
    fn pi_adapter_watches_every_spawn_round_without_a_foreground_split() {
        let source = pi_extension().expect("render");
        assert!(!EXECUTE_SPAWN.contains("outcome.task_submitted"));
        assert!(EXECUTE_SPAWN.contains(
            "await recordWatchedToken(ctx.sessionManager.getSessionId(), outcome.session);\n      await startWatcher(ctx);"
        ));
        assert!(EXECUTE_SPAWN.contains(
            "`spawned session ${outcome.session} in the background (${outcome.tool}, key ${outcome.key}); round queued, you will be woken when it completes`"
        ));
        assert!(source.contains("await startWatcher(ctx)"));
    }

    #[test]
    fn pi_adapter_keeps_the_wake_composition_in_the_watcher() {
        assert!(
            !WATCH_GLUE.contains("function reportSnippet(screen)"),
            "the report snippet now lives in the watcher (watch.rs report_snippet)"
        );
        assert!(!WATCH_GLUE.contains("Collect with session_bridge."));
        assert!(
            !WATCH_GLUE.contains("start a fresh lane if the work still matters."),
            "wake copy moved into the watcher's wake_message"
        );
        assert!(
            !WATCH_GLUE.contains("nudge it with qol sessions resume --kickstart"),
            "wake copy moved into the watcher's wake_message"
        );
    }

    #[test]
    fn pi_adapter_watcher_glue_is_defensive_and_keeps_the_closing_flow_untouched() {
        let source = pi_extension().expect("render");
        assert!(WATCH_GLUE.contains("try {"));
        assert!(WATCH_GLUE.contains("catch {}"));
        assert!(LOOP_SETUP.contains("restoreLoopPhase(ctx);"));
        assert!(LOOP_SETUP.contains("} else {\n        setLoopPhase(\"idle\")"));
        assert!(LOOP_SETUP
            .contains("pi.sendUserMessage(FINAL_REPORT_FOLLOW_UP, { deliverAs: \"followUp\" })"));
        assert!(source.contains("let closingFollowUpSent = false;"));
    }

    #[test]
    fn pi_adapter_keeps_the_feature_loop_armed_through_review() {
        let source = pi_extension().expect("render");
        assert!(source.contains("setLoopPhase(\"waiting\")"));
        assert!(source.contains("setLoopPhase(outcome.completed ? \"review\" : \"paused\")"));
        assert!(
            !source.contains("recovered the previous implementation response"),
            "bridge no longer submits, so there is no recovered-submit copy"
        );
        assert!(source.contains("pi.on(\"agent_settled\""));
        assert!(source.contains("pi.sendUserMessage(REVIEW_FOLLOW_UP"));
    }

    #[test]
    fn pi_adapter_fires_the_review_reminder_once_per_round() {
        let source = pi_extension().expect("render");
        assert!(source.contains("let reviewFollowUpSent = false;"));
        assert!(source.contains("if (!reviewFollowUpSent) {"));
        let set_at = source
            .find("reviewFollowUpSent = true;")
            .expect("flag set before send");
        let sent_at = source
            .find("pi.sendUserMessage(REVIEW_FOLLOW_UP")
            .expect("review follow-up send");
        assert!(set_at < sent_at);
        assert!(EXECUTE_SUBMIT.contains("reviewFollowUpSent = false;"));
        assert!(LOOP_SETUP.contains(
            "if (loopPhase === \"review\" && phase !== \"review\") reviewFollowUpSent = false;"
        ));
        assert!(source.contains("if (!closingFollowUpSent) {"));
        assert!(source.contains("closingFollowUpSent = true;"));
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
        assert!(EXECUTE_SPAWN.contains("args.push(\"--task\", params.task, \"--background\")"));
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
        assert!(
            source.contains(
                "text: typeof receipt.final_report === \"string\" ? receipt.final_report : JSON.stringify(receipt)"
            ),
            "the loop-close result text must be the human-readable final report, with the full receipt as fallback"
        );
        assert!(
            !EXECUTE_LOOP_CLOSE
                .contains("content: [{ type: \"text\", text: JSON.stringify(receipt) }]"),
            "the raw JSON receipt must not be rendered into the chat"
        );
        assert!(EXECUTE_LOOP_CLOSE.contains("details: { receipt },"));
    }

    #[test]
    fn pi_adapter_never_passes_submit_autoclose_through() {
        assert!(
            !EXECUTE_SUBMIT.contains("params.autoclose"),
            "the removed autoclose knob must not be read from params"
        );
        assert!(
            !EXECUTE_SUBMIT.contains("--no-auto-close") && !EXECUTE_SUBMIT.contains("--auto-close"),
            "the removed autoclose knob must not reach the CLI"
        );
        assert!(EXECUTE_SUBMIT.contains(
            "if (params.acknowledge_marker != null) args.push(\"--acknowledge-marker\", params.acknowledge_marker)"
        ));
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
