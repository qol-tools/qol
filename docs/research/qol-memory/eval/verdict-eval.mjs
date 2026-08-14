#!/usr/bin/env node
import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { qolMemoryStore } from "../lib/store-path.js";
import { countPendingCandidates } from "../lib/retrieval-log.js";

const BASE = dirname(fileURLToPath(import.meta.url));
const ASK = resolve(join(BASE, "..", "ask.mjs"));
const args = process.argv.slice(2);
const pick = (flag, def) => {
  const i = args.indexOf(flag);
  return i >= 0 && args[i + 1] ? args[i + 1] : def;
};
const STORE_SRC = resolve(pick("--store", qolMemoryStore()));
const SNAPSHOT_RUN = pick("--snapshot-run", "2026-08-12T18-46-58-129Z");
const NOTES_RUN = pick("--notes-run", "2026-08-13T16:31:40.844Z");
const REBUILD = args.includes("--rebuild");
const FLOOR = Number(pick("--floor", "11"));

const heldout = JSON.parse(readFileSync(resolve(pick("--heldout", join(BASE, "heldout.json"))), "utf8"));
const questions = heldout.questions;
const traps = heldout.traps || [];

const TEMP_ROOT = join(tmpdir(), "qol-memory-verdict-eval");
const STORE = join(TEMP_ROOT, `${SNAPSHOT_RUN}__${NOTES_RUN}`);

function freeze() {
  if (existsSync(STORE) && !REBUILD) return;
  rmSync(STORE, { recursive: true, force: true });
  mkdirSync(join(STORE, "snapshot"), { recursive: true });
  mkdirSync(join(STORE, "notes"), { recursive: true });
  cpSync(join(STORE_SRC, "snapshot", SNAPSHOT_RUN), join(STORE, "snapshot", SNAPSHOT_RUN), { recursive: true });
  cpSync(join(STORE_SRC, "notes", NOTES_RUN), join(STORE, "notes", NOTES_RUN), { recursive: true });
}
freeze();
if (existsSync(join(STORE, "units.jsonl"))) {
  throw new Error(`frozen store ${STORE} must never carry a live units.jsonl`);
}

const norm = (s) =>
  (s || "")
    .toLowerCase()
    .replace(/[^a-z0-9 ]/g, " ")
    .replace(/\s+/g, " ")
    .trim();

function factMatch(answerText, fact) {
  if (!answerText) return false;
  const a = norm(answerText);
  const f = norm(fact);
  if (!f) return true;
  return a.includes(f);
}

function runAsk(query, fact) {
  const args = [ASK, query, "--brief", "--log-source", "eval"];
  if (fact) args.push("--log-fact", fact);
  const out = execFileSync("node", args, {
    env: { ...process.env, QOL_MEMORY_STORE: STORE },
    encoding: "utf8",
    timeout: 120000,
  });
  return JSON.parse(out);
}

const rows = [];
for (const q of questions) {
  const d = runAsk(q.query, q.fact);
  const matched = factMatch(d.answer ? d.answer.text : null, q.fact);
  const result = d.verdict === "answered" ? (matched ? "correct" : "wrong") : "unanswered";
  rows.push({ id: q.id, verdict: d.verdict, result, answer: d.answer ? d.answer.text : null, answer_key: d.answer ? d.answer.key : null, reason: d.reason });
}

const wrong = rows.filter((r) => r.result === "wrong");
const correct = rows.filter((r) => r.result === "correct");
const answered = rows.filter((r) => r.result !== "unanswered");
const unanswered = rows.filter((r) => r.result === "unanswered");

const trapRows = [];
for (const t of traps) {
  const d = runAsk(t.query, "trap:" + t.expected);
  trapRows.push({ id: t.id, verdict: d.verdict, expected: t.expected, ok: d.verdict !== "answered", drift: d.verdict !== t.expected });
}
const trapFails = trapRows.filter((r) => !r.ok);

const gatePass = wrong.length === 0 && correct.length >= FLOOR && trapFails.length === 0;
const pending = countPendingCandidates(qolMemoryStore());

console.log(`frozen store ${STORE}`);
console.log(`snapshot pin ${SNAPSHOT_RUN} | notes run ${NOTES_RUN} (newest kept snapshot + recovered notes run at harness build 2026-08-12)`);
console.log(`floor: correct >= ${FLOOR} (correct count observed on this frozen store at harness build; wrong==0 pins precision, floor pins recall)`);
console.log("");
console.log("heldout verdicts:");
for (const r of rows) {
  const detail = r.result === "correct" ? r.answer : r.result === "wrong" ? r.answer : r.reason;
  console.log(`  ${r.id.padEnd(4)} ${r.verdict.padEnd(10)} ${r.result.padEnd(10)} ${(detail || "").slice(0, 90)}${r.answer_key ? ` ::${r.answer_key}` : ""}`);
}
console.log("");
if (wrong.length) {
  console.log("WRONG (answered without fact match):");
  for (const r of wrong) console.log(`  ${r.id} :: ${questions.find((q) => q.id === r.id).query} -> "${r.answer}"`);
  console.log("");
}
console.log("traps:");
for (const r of trapRows) {
  const drift = r.drift ? " (drift from expected " + r.expected + ")" : "";
  console.log(`  ${r.id.padEnd(4)} ${r.verdict.padEnd(10)} ${r.ok ? "safe" : "FAIL answered"}${drift}`);
}
console.log("");
console.log(
  `verdict-eval | heldout ${rows.length} | answered ${answered.length} | correct ${correct.length} | wrong ${wrong.length} | unanswered ${unanswered.length} | traps ${trapRows.length - trapFails.length}/${trapRows.length} safe | gate ${gatePass ? "PASS" : "FAIL"} | candidates pending ${pending}`
);
process.exit(gatePass ? 0 : 1);
