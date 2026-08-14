#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { qolMemoryStore } from "./lib/store-path.js";

const BASE = dirname(fileURLToPath(import.meta.url));
const ASK = join(BASE, "ask.mjs");
const SNAPSHOT_RUN = "2026-08-12T18-46-58-129Z";
const NOTES_RUN = "2026-08-13T16:31:40.844Z";
const FROZEN = join(tmpdir(), "qol-memory-units-replace-test");
const STORE_SRC = qolMemoryStore();

const QUERIES = [
  "m4a1",
  "m4a1 anchoring",
  "how did we fix the m4a1 rifle anchoring",
  "what does the m4a1 weapon cost",
];

let failed = 0;
function check(cond, label) {
  if (!cond) {
    failed++;
    console.error(`FAIL ${label}`);
  } else {
    console.log(`pass ${label}`);
  }
}

function freeze() {
  rmSync(FROZEN, { recursive: true, force: true });
  mkdirSync(join(FROZEN, "snapshot"), { recursive: true });
  mkdirSync(join(FROZEN, "notes"), { recursive: true });
  cpSync(join(STORE_SRC, "snapshot", SNAPSHOT_RUN), join(FROZEN, "snapshot", SNAPSHOT_RUN), { recursive: true });
  cpSync(join(STORE_SRC, "notes", NOTES_RUN), join(FROZEN, "notes", NOTES_RUN), { recursive: true });
  if (existsSync(join(FROZEN, "units.jsonl"))) throw new Error(`frozen store ${FROZEN} must never carry a live units.jsonl`);
}

function runAsk(query, envExtra) {
  const r = spawnSync("node", [ASK, query, "--k", "5", "--no-log"], {
    env: { ...process.env, QOL_MEMORY_STORE: FROZEN, ...envExtra },
    encoding: "utf8",
    timeout: 120000,
    maxBuffer: 64 * 1024 * 1024,
  });
  if (r.status !== 0) throw new Error(`ask.mjs exited ${r.status} for "${query}": ${(r.stderr || "").slice(0, 400)}`);
  return JSON.parse(r.stdout);
}

freeze();
const rows = [];
for (const q of QUERIES) {
  const off = runAsk(q, { QOL_MEMORY_ALIASES_DISABLE: "1" });
  const on = runAsk(q, {});
  rows.push({
    query: q,
    off: {
      m4: (off.units || []).slice(0, 5).filter((u) => (u.text || "").toLowerCase().includes("m4a1")).length,
      top: off.signals.top_unit_score,
      verdict: off.verdict,
    },
    on: {
      m4: (on.units || []).slice(0, 5).filter((u) => (u.text || "").toLowerCase().includes("m4a1")).length,
      top: on.signals.top_unit_score,
      verdict: on.verdict,
    },
  });
}

console.log("m4a1 units-layer reachability on the pinned frozen store (top-5 of the units layer):");
console.log("query | aliases-off m4-top5 / top_score / verdict | aliases-on m4-top5 / top_score / verdict");
for (const r of rows) {
  console.log(
    `  ${r.query.padEnd(38)} | ${r.off.m4}/5 ${String(r.off.top).padStart(6)} ${r.off.verdict.padEnd(10)} | ${r.on.m4}/5 ${String(r.on.top).padStart(6)} ${r.on.verdict}`
  );
}

for (const r of rows) {
  check(r.off.m4 === 5, `U1 aliases-off units top-5 all m4a1 units for "${r.query}" (raw query semantics)`);
  check(r.on.m4 === 5, `U2 aliases-on units top-5 all m4a1 units for "${r.query}" (keep-original units query)`);
}
check(rows[2].on.verdict === "answered" && rows[2].off.verdict === "candidates", "U3 d01-class query: honest abstention without the bridge, answered via decision notes with aliases");
check(rows[3].on.verdict === "candidates" && rows[3].off.verdict === "candidates", "U4 t04 trap stays candidates in both modes");

console.log(failed ? `test-units-replace FAILED ${failed}` : "test-units-replace ALL PASS");
process.exit(failed ? 1 : 0);
