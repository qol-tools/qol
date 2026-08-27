#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { readdirSync, writeFileSync, mkdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { qolMemoryStore } from "./lib/store-path.js";
import { mergeStep } from "./lib/merge.js";
import { countPendingCandidates } from "./lib/retrieval-log.js";

const BASE = dirname(fileURLToPath(import.meta.url));
const STORE_ROOT = qolMemoryStore();
const ARGS = process.argv.slice(2);
const NO_LLM = ARGS.includes("--no-llm");

const step = (name, cmd, args) => stepEnv(name, cmd, args, process.env);
const stepEnv = (name, cmd, args, env) => {
  const t = Date.now();
  const r = spawnSync(cmd, args, { encoding: "utf8", timeout: 600000, maxBuffer: 64 * 1024 * 1024, env });
  const ms = Date.now() - t;
  if (r.status !== 0) {
    console.error(`[ingest] FAIL ${name} (exit ${r.status}, ${ms}ms)\n${(r.stderr || "").slice(0, 500)}`);
    process.exit(1);
  }
  console.log(`[ingest] ${name} done (${ms}ms)`);
  return r.stdout;
};

const snapOut = step("snapshot", "node", [join(BASE, "snapshot.mjs")]);
const snapRun = (snapOut.match(/snapshot\/([0-9TZ.-]+)\/report\.json/) || [])[1];
if (!snapRun) {
  console.error(`[ingest] could not locate snapshot run: ${snapOut.slice(0, 300)}`);
  process.exit(1);
}

mergeStep(STORE_ROOT, snapRun);

const decArgs = [join(BASE, "decisions.mjs"), "--snapshot-run", snapRun];
const decEnv = NO_LLM ? { ...process.env, QOL_MEMORY_MODEL_DISABLE: "1" } : process.env;
const decOut = stepEnv("decisions", "node", decArgs, decEnv);

const notesRuns = readdirSync(join(STORE_ROOT, "notes")).filter((n) => /^\d{4}-\d{2}-\d{2}T/.test(n)).sort().reverse();
const notesRun = notesRuns[0];

const evalOut = step("eval units (frozen)", "node", [join(BASE, "eval", "eval.mjs")]);
const notesEvalOut = step("eval notes (latest run)", "node", [
  join(BASE, "eval", "eval.mjs"),
  "--notes",
  join(STORE_ROOT, "notes", notesRun, "notes.jsonl"),
]);
const skillsOut = step("eval skills", "node", [join(BASE, "eval", "skills-eval.mjs")]);

const stepInfo = (name, cmd, args) => {
  const t = Date.now();
  const r = spawnSync(cmd, args, { encoding: "utf8", timeout: 600000, maxBuffer: 64 * 1024 * 1024 });
  const ms = Date.now() - t;
  const verdict = (r.stdout || "").match(/verdict-eval \| ([^\n]+)/);
  console.log(`[ingest] ${name} done (${ms}ms)${verdict ? " - " + verdict[1] : ""}${r.status !== 0 ? ` (informational, exit ${r.status} ignored)` : ""}`);
  return r.stdout;
};
const verdictOut = stepInfo("verdict eval (informational)", "node", [join(BASE, "eval", "verdict-eval.mjs")]);

const grab = (out, re) => (out.match(re) || [])[1] || "?";
const report = {
  name: "qol-memory ingest",
  schemaVersion: 1,
  started_at: new Date().toISOString(),
  commands: [`node ${join(BASE, "ingest.mjs")}${NO_LLM ? " --no-llm" : ""}`],
  candidates_pending: countPendingCandidates(STORE_ROOT),
  snapshot_run: snapRun,
  notes_run: notesRun,
  decisions: grab(decOut, /decisions added: ([^\n]+)/),
  evals: {
    units: grab(evalOut, /bm25\s+hit@1 (\d+\/\d+) \(\d+%\)\s+hit@5 (\d+\/\d+)/) ? {
      hit1: grab(evalOut, /hit@1 (\d+\/\d+)/),
      hit5: grab(evalOut, /hit@5 (\d+\/\d+)/),
      mrr: grab(evalOut, /mrr ([\d.]+)/),
    } : "see eval report",
    notes: {
      hit1: grab(notesEvalOut, /\[notes\] bm25\s+hit@1 (\d+\/\d+)/),
      hit5: grab(notesEvalOut, /\[notes\] bm25\s+hit@1 \d+\/\d+ \(\d+%\)\s+hit@5 (\d+\/\d+)/),
      mrr: grab(notesEvalOut, /\[notes\] bm25[^\n]*mrr ([\d.]+)/),
    },
    skills: grab(skillsOut, /skills eval: ([^\n]+)/),
    verdict: grab(verdictOut, /verdict-eval \| ([^\n]+)/),
  },
};
const outDir = join(STORE_ROOT, "ingest");
mkdirSync(outDir, { recursive: true });
const outPath = join(outDir, `report-${new Date().toISOString().replace(/[:.]/g, "-")}.json`);
writeFileSync(outPath, JSON.stringify(report, null, 2));
console.log(`[ingest] report: ${outPath}`);
