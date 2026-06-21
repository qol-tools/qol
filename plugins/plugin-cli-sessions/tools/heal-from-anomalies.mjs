#!/usr/bin/env node
// Triage captured anomalies into reviewable candidate fixtures.
//
//   input:  an anomalies dir written by the daemon recorder
//           (CLI_SESSIONS_RECORD_ANOMALIES=1; path is logged when it fires)
//   output: tests/fixtures/candidates/<name>.{txt,meta.json} for each frame
//           that classifies NeedsYou, plus a report.json summary
//
// The label stays human/agent-gated: a real prompt answered within the flap
// window also flaps, so we surface candidates - we do not auto-assert them.
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync, mkdirSync, readdirSync, existsSync, rmSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve, join } from "node:path";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, "..", "..", "..");
const PLUGIN = resolve(REPO, "plugins", "plugin-cli-sessions");
const BIN = resolve(REPO, "target", "debug", "examples", "classify");
const CANDIDATES = join(PLUGIN, "tests", "fixtures", "candidates");

const anomDir = process.argv[2] || process.env.CLI_SESSIONS_ANOMALY_DIR;
if (!anomDir || !existsSync(anomDir)) {
  console.log(`no anomalies dir (pass one as arg, or set CLI_SESSIONS_ANOMALY_DIR). got: ${anomDir}`);
  process.exit(0);
}

function classify(title, screen) {
  const payload = JSON.stringify({ title, at_prompt: false, foreground_basenames: ["claude"], screen });
  return execFileSync(BIN, [], { input: payload, encoding: "utf8" }).trim();
}

rmSync(CANDIDATES, { recursive: true, force: true });
mkdirSync(CANDIDATES, { recursive: true });

const summary = [];
for (const name of readdirSync(anomDir)) {
  const dir = join(anomDir, name);
  const reportPath = join(dir, "report.json");
  if (!existsSync(reportPath)) continue;
  const report = JSON.parse(readFileSync(reportPath, "utf8"));
  for (const f of report.frames) {
    const screenPath = join(dir, f.file);
    if (!existsSync(screenPath)) continue;
    const screen = readFileSync(screenPath, "utf8");
    const result = classify(f.title, screen);
    const status = /status=(\w+)/.exec(result)?.[1];
    if (status !== "NeedsYou") continue;
    const label = `flap_${name}_${f.file.replace(/\.txt$/, "")}`;
    writeFileSync(join(CANDIDATES, `${label}.txt`), screen);
    writeFileSync(
      join(CANDIDATES, `${label}.meta.json`),
      JSON.stringify(
        {
          title: f.title,
          at_prompt: false,
          foreground_basenames: ["claude"],
          expect: null,
          note: `captured from ${report.kind} on win${report.window_id} (dwell ${report.dwell_secs}s); confirm false-positive then set expect or move to corpus`,
        },
        null,
        2
      )
    );
    summary.push({ anomaly: name, frame: f.file, dwell_secs: report.dwell_secs, classified: status, candidate: label });
  }
}

writeFileSync(join(CANDIDATES, "report.json"), JSON.stringify({ count: summary.length, candidates: summary }, null, 2));
console.log(`triaged ${summary.length} NeedsYou frame(s) -> ${CANDIDATES}`);
for (const s of summary) console.log(`  ${s.candidate} (dwell ${s.dwell_secs}s)`);
if (summary.length === 0) console.log("  no NeedsYou frames in the captured anomalies");
