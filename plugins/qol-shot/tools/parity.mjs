#!/usr/bin/env node
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, "..", "..", "..");
const DEFAULT_BINARY = resolve(REPO, "target", "debug", "qol-shot");
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

const [verb = "init", ...args] = process.argv.slice(2);
if (verb === "init") init(args);
else if (verb === "mark") mark(args);
else if (verb === "summary") summary(args);
else throw new Error(`unknown verb: ${verb}`);
