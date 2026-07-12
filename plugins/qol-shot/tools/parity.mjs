#!/usr/bin/env node
import { execFileSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { createConnection } from "node:net";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, "..", "..", "..");
const DEFAULT_BINARY = resolve(REPO, "target", "debug", "qol-shot");
const TRACE_LOG = "/tmp/qol-altmon.log";
const VERDICTS = new Set(["pass", "fail", "blocked"]);

const MANUAL_SCENARIOS = [
  ["selection-cancel", "selection", "Cancelling selection exits cleanly without creating output."],
  ["selection-manual", "selection", "A manually dragged region captures the selected pixels."],
  ["selection-window-target", "selection", "A detected window can be selected without drawing a region."],
  ["screenshot-preview", "screenshot", "A completed screenshot opens the preview for the captured image."],
  ["preview-actions", "preview", "Preview copy, copy-path, pin, and dismiss actions complete correctly."],
  ["pin-interaction", "pin", "Pinned images move and resize predictably by drag and wheel."],
  ["clipboard-image", "clipboard", "Copy places the latest screenshot image on the native clipboard."],
  ["clipboard-path", "clipboard", "Copy Path places the latest screenshot path on the native clipboard."],
  ["recording-video", "recording", "Recording starts, stops, finalizes, and reveals a playable file."],
  ["recording-microphone", "recording", "Microphone-enabled recording contains audible microphone input."],
  ["recording-system-audio", "recording", "System-audio recording contains audible desktop output."],
  ["recording-formats", "recording", "Every exposed output format produces a playable file or a visible prerequisite error."],
  ["multi-monitor", "capture", "Selection and capture remain aligned across monitor boundaries and offsets."],
];

function platformName() {
  if (process.platform === "darwin") return "macos";
  if (process.platform === "win32") return "windows";
  return process.platform;
}

function displayBackend() {
  if (process.platform !== "linux") return "native";
  const session = (process.env.XDG_SESSION_TYPE || "").toLowerCase();
  if (process.env.WAYLAND_DISPLAY || session === "wayland") return "wayland";
  if (process.env.DISPLAY || session === "x11") return "x11";
  return "unknown";
}

function run(command, args) {
  try {
    const stdout = execFileSync(command, args, {
      cwd: REPO,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
    return { argv: [command, ...args], exit_code: 0, stdout };
  } catch (error) {
    return {
      argv: [command, ...args],
      exit_code: error.status ?? 1,
      stdout: error.stdout?.toString() || "",
      stderr: error.stderr?.toString() || error.message,
    };
  }
}

function automatedScenarios(binary, commands) {
  const doctor = run(binary, ["--json", "doctor"]);
  commands.push(doctor);
  let doctorReport = null;
  try {
    doctorReport = JSON.parse(doctor.stdout);
  } catch {
    doctorReport = null;
  }
  const doctorPass = doctor.exit_code === 0 && doctorReport?.status !== "fail";

  const help = run(binary, ["help"]);
  commands.push(help);
  const expectedCommands = ["copy", "copy-path", "doctor", "preview", "record", "screenshot", "settings"];
  const missingCommands = expectedCommands.filter((command) => !new RegExp(`^  ${command}\\s`, "m").test(help.stdout));

  return [
    {
      id: "doctor",
      capability: "health",
      contract: "Native doctor completes without a failing platform or dependency check.",
      evidence_type: "automated-native",
      status: doctorPass ? "pass" : "fail",
      evidence: doctorReport || doctor.stderr || "doctor returned invalid JSON",
    },
    {
      id: "command-surface",
      capability: "cli",
      contract: "Every supported platform exposes the same public command surface.",
      evidence_type: "automated-native",
      status: help.exit_code === 0 && missingCommands.length === 0 ? "pass" : "fail",
      evidence: missingCommands.length === 0 ? expectedCommands : { missing_commands: missingCommands },
    },
  ];
}

function overallStatus(scenarios) {
  if (scenarios.some((scenario) => scenario.status === "fail")) return "failed";
  if (scenarios.some((scenario) => scenario.status === "pending")) return "pending";
  if (scenarios.some((scenario) => scenario.status === "blocked")) return "blocked";
  return "pass";
}

function save(reportPath, report) {
  report.updated_at = new Date().toISOString();
  report.status = overallStatus(report.scenarios);
  mkdirSync(dirname(reportPath), { recursive: true });
  writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
}

function readReport(reportPath) {
  if (!existsSync(reportPath)) throw new Error(`report not found: ${reportPath}`);
  return JSON.parse(readFileSync(reportPath, "utf8"));
}

function previousReport(reportPath) {
  if (!existsSync(reportPath)) return null;
  try {
    const report = readReport(reportPath);
    const compatible =
      report.inputs?.platform === platformName() && report.inputs?.arch === process.arch;
    return compatible ? report : null;
  } catch {
    return null;
  }
}

function printSummary(reportPath, report) {
  console.log(`${report.name}: ${report.status}`);
  console.log(`platform=${report.inputs.platform} arch=${report.inputs.arch} display=${report.inputs.display_backend}`);
  for (const scenario of report.scenarios) {
    console.log(`${scenario.status.padEnd(7)} ${scenario.id.padEnd(25)} ${scenario.contract}`);
  }
  console.log(`report=${reportPath}`);
}

function parseInitArgs(args) {
  let binary = DEFAULT_BINARY;
  let reportPath = resolve(REPO, "target", "qol-shot-parity", platformName(), "report.json");
  for (let index = 0; index < args.length; index += 1) {
    if (args[index] === "--binary") binary = resolve(args[++index]);
    else if (args[index] === "--out") reportPath = resolve(args[++index]);
    else throw new Error(`unknown argument: ${args[index]}`);
  }
  return { binary, reportPath };
}

function init(args) {
  const { binary, reportPath } = parseInitArgs(args);
  if (!existsSync(binary)) throw new Error(`binary not found: ${binary}`);
  const previous = previousReport(reportPath);
  const commands = [];
  const commit = run("git", ["rev-parse", "HEAD"]);
  commands.push(commit);
  const scenarios = automatedScenarios(binary, commands);
  scenarios.push(
    ...MANUAL_SCENARIOS.map(([id, capability, contract]) => {
      const prior = previous?.scenarios.find((scenario) => scenario.id === id);
      return {
        id,
        capability,
        contract,
        evidence_type: "native-manual",
        status: prior?.status || "pending",
        evidence: prior?.evidence || null,
        ...(prior?.verified_at ? { verified_at: prior.verified_at } : {}),
      };
    }),
  );
  const now = new Date().toISOString();
  const report = {
    name: "qol-shot-platform-parity",
    started_at: previous?.started_at || now,
    updated_at: now,
    status: "pending",
    inputs: {
      platform: platformName(),
      arch: process.arch,
      display_backend: displayBackend(),
      binary,
      commit: commit.exit_code === 0 ? commit.stdout.trim() : null,
    },
    artifacts: { report: reportPath },
    commands: commands.map(({ stdout, stderr, ...command }) => command),
    scenarios,
    next: [
      `node ${fileURLToPath(import.meta.url)} mark ${reportPath} <scenario> <pass|fail|blocked> [evidence]`,
      `node ${fileURLToPath(import.meta.url)} summary ${reportPath}`,
    ],
  };
  save(reportPath, report);
  printSummary(reportPath, report);
}

function mark([reportArg, scenarioId, verdict, ...evidenceParts]) {
  if (!reportArg || !scenarioId || !VERDICTS.has(verdict)) {
    throw new Error("usage: parity.mjs mark <report.json> <scenario> <pass|fail|blocked> [evidence]");
  }
  const reportPath = resolve(reportArg);
  const report = readReport(reportPath);
  const scenario = report.scenarios.find((candidate) => candidate.id === scenarioId);
  if (!scenario) throw new Error(`unknown scenario: ${scenarioId}`);
  scenario.status = verdict;
  scenario.evidence = evidenceParts.join(" ") || null;
  scenario.verified_at = new Date().toISOString();
  save(reportPath, report);
  printSummary(reportPath, report);
}

function summary([reportArg]) {
  if (!reportArg) throw new Error("usage: parity.mjs summary <report.json>");
  const reportPath = resolve(reportArg);
  printSummary(reportPath, readReport(reportPath));
}

function parseSmokeArgs(args) {
  let binary = DEFAULT_BINARY;
  let reportPath = resolve(REPO, "target", "qol-shot-selector-smoke", "linux", "report.json");
  let timeoutMs = 4000;
  let beforeMs = null;
  let beforeGrabMs = null;
  let isolatedDaemon = true;
  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index];
    if (argument === "--binary") {
      binary = resolve(args[++index]);
      continue;
    }
    if (argument === "--out") {
      reportPath = resolve(args[++index]);
      continue;
    }
    if (argument === "--timeout-ms") {
      timeoutMs = Number(args[++index]);
      continue;
    }
    if (argument === "--before-ms") {
      beforeMs = Number(args[++index]);
      continue;
    }
    if (argument === "--before-grab-ms") {
      beforeGrabMs = Number(args[++index]);
      continue;
    }
    if (argument === "--standalone") {
      isolatedDaemon = false;
      continue;
    }
    throw new Error(`unknown argument: ${argument}`);
  }
  return { binary, reportPath, timeoutMs, beforeMs, beforeGrabMs, isolatedDaemon };
}

function selectorWindow() {
  const result = run("xdotool", ["search", "--onlyvisible", "--name", "^qol-shot-selector-"]);
  return result.exit_code === 0 ? result.stdout.trim().split("\n").filter(Boolean).at(-1) : null;
}

function rootWindow() {
  const result = run("xwininfo", ["-root", "-int"]);
  const id = result.stdout.match(/Window id:\s+(\d+)/)?.[1];
  if (!id) throw new Error("could not resolve the X11 root window");
  return id;
}

function windowGeometry(windowId) {
  const result = run("xdotool", ["getwindowgeometry", "--shell", windowId]);
  if (result.exit_code !== 0) return null;
  const values = Object.fromEntries(
    result.stdout
      .trim()
      .split("\n")
      .map((line) => line.split("=")),
  );
  return {
    x: Number(values.X),
    y: Number(values.Y),
    width: Number(values.WIDTH),
    height: Number(values.HEIGHT),
  };
}

function rootGeometry() {
  const result = run("xwininfo", ["-root", "-int"]);
  const width = Number(result.stdout.match(/Width:\s+(\d+)/)?.[1]);
  const height = Number(result.stdout.match(/Height:\s+(\d+)/)?.[1]);
  return { x: 0, y: 0, width, height };
}

function delay(ms) {
  return new Promise((resolveDelay) => setTimeout(resolveDelay, ms));
}

async function waitFor(check, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const value = check();
    if (value) return value;
    await delay(20);
  }
  return null;
}

function traceSlice(offset, pid) {
  if (!existsSync(TRACE_LOG)) return "";
  return readFileSync(TRACE_LOG).subarray(offset).toString().split("\n").filter((line) => line.includes(`pid=${pid} `)).join("\n");
}

function traceTimestamp(trace, tag) {
  const line = trace.split("\n").find((candidate) => candidate.includes(` ${tag} `));
  return Number(line?.match(/^(\d+)/)?.[1]) || null;
}

function traceMetric(trace, tag, field) {
  const line = trace.split("\n").find((candidate) => candidate.includes(` ${tag} `));
  return Number(line?.match(new RegExp(`${field}=(\\d+)`))?.[1]) || null;
}

async function stopChild(child) {
  if (child.exitCode !== null) return;
  child.kill("SIGTERM");
  await Promise.race([new Promise((resolveExit) => child.once("exit", resolveExit)), delay(500)]);
  if (child.exitCode !== null) return;
  child.kill("SIGKILL");
}

function sendDaemonAction(socketPath, action) {
  return new Promise((resolveAction, rejectAction) => {
    const socket = createConnection(socketPath);
    const timeout = setTimeout(() => {
      socket.destroy();
      rejectAction(new Error(`daemon action timed out: ${action}`));
    }, 1000);
    socket.once("error", (error) => {
      clearTimeout(timeout);
      rejectAction(error);
    });
    socket.once("data", () => {
      clearTimeout(timeout);
      socket.destroy();
      resolveAction();
    });
    socket.once("connect", () => {
      socket.end(`${JSON.stringify({ action })}\n`);
    });
  });
}

function metricDelta(before, after) {
  if (!Number.isFinite(before) || !Number.isFinite(after) || before === 0) return "N/A";
  const amount = after - before;
  const percent = ((amount / before) * 100).toFixed(1);
  return `${amount} ms (${percent}%)`;
}

function smokeMetrics(context, reportPath, beforeMs, afterMs, beforeGrabMs, afterGrabMs, escaped) {
  return [
    {
      improvement_vector: "frozen selector responsiveness",
      scenario: "screenshot command to exact selector",
      context,
      metric: "startup latency",
      before: Number.isFinite(beforeMs) ? `${beforeMs} ms` : "not measured by this run",
      after: Number.isFinite(afterMs) ? `${afterMs} ms` : "not measured",
      delta: metricDelta(beforeMs, afterMs),
      correctness: Number.isFinite(afterMs) ? "passed" : "failed",
      evidence: reportPath,
    },
    {
      improvement_vector: "frozen selector responsiveness",
      scenario: "full virtual desktop freeze",
      context,
      metric: "X11 frame transfer",
      before: Number.isFinite(beforeGrabMs) ? `${beforeGrabMs} ms` : "not measured by this run",
      after: Number.isFinite(afterGrabMs) ? `${afterGrabMs} ms` : "not measured",
      delta: metricDelta(beforeGrabMs, afterGrabMs),
      correctness: Number.isFinite(afterGrabMs) ? "passed" : "failed",
      evidence: reportPath,
    },
    {
      improvement_vector: "reliable selector cancellation",
      scenario: "Escape after focus moves to the X11 root",
      context,
      metric: "global cancellation",
      before: "failed",
      after: escaped ? "passed" : "failed",
      delta: "N/A",
      correctness: escaped ? "passed" : "failed",
      evidence: reportPath,
    },
  ];
}

async function smokeSelector(args) {
  const { binary, reportPath, timeoutMs, beforeMs, beforeGrabMs, isolatedDaemon } =
    parseSmokeArgs(args);
  if (process.platform !== "linux" || displayBackend() !== "x11") {
    throw new Error("selector smoke requires a live Linux X11 desktop");
  }
  if (!existsSync(binary)) throw new Error(`binary not found: ${binary}`);
  if (selectorWindow()) throw new Error("a qol-shot selector is already visible");

  const startedAt = new Date().toISOString();
  const traceOffset = existsSync(TRACE_LOG) ? statSync(TRACE_LOG).size : 0;
  const socketPath = resolve(dirname(reportPath), "daemon.sock");
  rmSync(socketPath, { force: true });
  const env = { ...process.env };
  if (isolatedDaemon) {
    env.QOL_TRAY_DAEMON_SOCKET = socketPath;
    env.QOL_TRAY_DAEMON_REPLACE_EXISTING = "1";
  }
  const child = spawn(binary, isolatedDaemon ? [] : ["screenshot"], {
    cwd: REPO,
    stdio: "ignore",
    env,
  });
  let escapeHeld = false;
  let geometry = null;
  let root = null;
  let selectorGone = false;
  let visibleMs = null;
  let wallStarted = Date.now();

  try {
    if (isolatedDaemon) {
      const ready = await waitFor(
        () => traceSlice(traceOffset, child.pid).includes(" SHOT_DAEMON_APP state=ready"),
        timeoutMs,
      );
      if (!ready) throw new Error("isolated daemon did not become ready before timeout");
      wallStarted = Date.now();
      await sendDaemonAction(socketPath, "screenshot");
    }
    const windowId = await waitFor(selectorWindow, timeoutMs);
    if (!windowId) throw new Error("selector did not appear before timeout");
    visibleMs = Date.now() - wallStarted;
    const aligned = await waitFor(
      () => {
        const trace = traceSlice(traceOffset, child.pid);
        return trace.includes(" SHOT_SELECT_VIEWPORT ") && trace.includes("aligned=true");
      },
      timeoutMs,
    );
    if (!aligned) throw new Error("selector did not reach exact viewport bounds before timeout");
    geometry = windowGeometry(windowId);
    root = rootGeometry();
    run("xdotool", ["windowfocus", rootWindow()]);
    await delay(60);
    run("xdotool", ["keydown", "Escape"]);
    escapeHeld = true;
    await delay(150);
    run("xdotool", ["keyup", "Escape"]);
    escapeHeld = false;
    selectorGone = Boolean(await waitFor(() => !selectorWindow(), timeoutMs));
    await delay(100);
  } finally {
    if (escapeHeld) run("xdotool", ["keyup", "Escape"]);
    if (isolatedDaemon && existsSync(socketPath)) {
      await sendDaemonAction(socketPath, "kill").catch(() => {});
    }
    await stopChild(child);
    rmSync(socketPath, { force: true });
  }

  const trace = traceSlice(traceOffset, child.pid);
  const entryAt = isolatedDaemon
    ? traceTimestamp(trace, "SHOT_CMD")
    : traceTimestamp(trace, "SHOT_ENTRY");
  const viewportAt = traceTimestamp(trace, "SHOT_SELECT_VIEWPORT");
  const startupMs = entryAt && viewportAt ? viewportAt - entryAt : null;
  const grabMs = traceMetric(trace, "SHOT_X11_GRAB", "ms");
  const exactGeometry = JSON.stringify(geometry) === JSON.stringify(root);
  const globalEscape = trace.includes(" SHOT_SELECT_CANCEL_INPUT source=global-escape");
  const escaped = globalEscape && selectorGone;
  const scenarios = [
    {
      id: "selector-alignment",
      capability: "selection",
      contract: "The frozen selector exactly matches the X11 root viewport.",
      evidence_type: "automated-native",
      status: exactGeometry ? "pass" : "fail",
      evidence: { selector: geometry, root },
    },
    {
      id: "selection-cancel-without-focus",
      capability: "selection",
      contract: "Escape cancels after input focus is moved away from the selector.",
      evidence_type: "automated-native",
      status: escaped ? "pass" : "fail",
      evidence: { global_escape_trace: globalEscape, selector_gone: selectorGone },
    },
    {
      id: "selector-startup",
      capability: "performance",
      contract: "The selector emits complete startup timing through the native trace.",
      evidence_type: "automated-native",
      status: Number.isFinite(startupMs) && Number.isFinite(grabMs) ? "pass" : "fail",
      evidence: { visible_ms: visibleMs, trace_startup_ms: startupMs, x11_grab_ms: grabMs },
    },
  ];
  const entrypoint = isolatedDaemon ? "isolated daemon action" : "standalone screenshot";
  const context = `Linux X11, ${entrypoint}, one run, ${root?.width}x${root?.height}`;
  const report = {
    name: "qol-shot-selector-smoke",
    started_at: startedAt,
    finished_at: new Date().toISOString(),
    status: "pending",
    inputs: {
      platform: "linux",
      arch: process.arch,
      display_backend: "x11",
      entrypoint,
      binary,
      timeout_ms: timeoutMs,
    },
    artifacts: { report: reportPath },
    commands: [
      isolatedDaemon ? [binary, "<isolated-daemon>"] : [binary, "screenshot"],
      isolatedDaemon ? ["daemon-action", "screenshot"] : [binary, "screenshot"],
      ["xdotool", "windowfocus", "<root>"],
      ["xdotool", "keydown", "Escape"],
    ],
    scenarios,
    metrics: smokeMetrics(context, reportPath, beforeMs, startupMs, beforeGrabMs, grabMs, escaped),
    next: [`node ${fileURLToPath(import.meta.url)} smoke-selector --binary ${binary} --out ${reportPath}`],
  };
  save(reportPath, report);
  printSummary(reportPath, report);
  if (report.status !== "pass") process.exitCode = 1;
}

const [verb = "init", ...args] = process.argv.slice(2);

async function main() {
  if (verb === "init") return init(args);
  if (verb === "mark") return mark(args);
  if (verb === "summary") return summary(args);
  if (verb === "smoke-selector") return smokeSelector(args);
  throw new Error(`unknown verb: ${verb}`);
}

main().catch((error) => {
  console.error(error.message);
  process.exitCode = 1;
});
