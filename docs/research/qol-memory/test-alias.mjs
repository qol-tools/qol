#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync, readdirSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { qolMemoryStore } from "./lib/store-path.js";
import { loadAliases, expandTokens, validate, ALIAS_CAP } from "./lib/concept-aliases.js";

const BASE = dirname(fileURLToPath(import.meta.url));
const ASK = join(BASE, "ask.mjs");
const SNAPSHOT_RUN = "2026-08-12T18-46-58-129Z";
const NOTES_RUN = "2026-08-13T16:31:40.844Z";
const FROZEN = join(tmpdir(), "qol-memory-alias-test");
const ALIASES_JSON = join(BASE, "..", "..", "..", "plugins", "qol-memory", "assets", "concept-aliases.json");
const STORE_SRC = qolMemoryStore();
const GOLD_KEYS = { d01: "79046028d14b1cec", d02: "d857979480b72faa", d03: "7570839245601dcc" };
const ABSTENTIONS = ["h08", "h09", "p01", "p02", "p03", "d01", "d02", "d03"];

const heldout = JSON.parse(readFileSync(join(BASE, "eval", "heldout.json"), "utf8"));

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

const norm = (s) => (s || "").toLowerCase().replace(/[^a-z0-9 ]/g, " ").replace(/\s+/g, " ").trim();

function factMatch(answerText, fact) {
  if (!answerText) return false;
  const a = norm(answerText);
  const f = norm(fact);
  if (!f) return true;
  return a.includes(f);
}

function runAsk(query, fact, envExtra) {
  const args = [ASK, query, "--brief", "--no-log", "--log-source", "eval"];
  if (fact) args.push("--log-fact", fact);
  const r = spawnSync("node", args, {
    env: { ...process.env, QOL_MEMORY_STORE: FROZEN, ...envExtra },
    encoding: "utf8",
    timeout: 120000,
    maxBuffer: 64 * 1024 * 1024,
  });
  if (r.status !== 0) throw new Error(`ask.mjs exited ${r.status} for "${query}": ${(r.stderr || "").slice(0, 400)}`);
  return JSON.parse(r.stdout);
}

function runGate(envExtra) {
  const rows = [];
  for (const q of heldout.questions) {
    const d = runAsk(q.query, q.fact, envExtra);
    const matched = factMatch(d.answer ? d.answer.text : null, q.fact);
    const result = d.verdict === "answered" ? (matched ? "correct" : "wrong") : "unanswered";
    rows.push({ id: q.id, verdict: d.verdict, result, answerKey: d.answer ? d.answer.key : null });
  }
  const traps = [];
  for (const t of heldout.traps || []) {
    const d = runAsk(t.query, null, envExtra);
    traps.push({ id: t.id, verdict: d.verdict, expected: t.expected, safe: d.verdict !== "answered", drift: d.verdict !== t.expected });
  }
  const correct = rows.filter((r) => r.result === "correct").length;
  const wrong = rows.filter((r) => r.result === "wrong").length;
  const unanswered = rows.filter((r) => r.result === "unanswered").length;
  const trapSafe = traps.filter((t) => t.safe).length;
  return { rows, traps, correct, wrong, unanswered, trapSafe, pass: wrong === 0 && correct >= 11 && trapSafe === traps.length };
}

freeze();
const baseline = runGate({ QOL_MEMORY_ALIASES_DISABLE: "1" });
const seed = runGate({});
const seedMap = loadAliases(ALIASES_JSON);

check(validate(ALIASES_JSON).ok, "C1 committed seed file passes validate");

check(seed.pass, "C1 seed gate PASS (wrong 0, correct >= 11, traps safe)");
check(seed.correct === 25 && seed.wrong === 0 && seed.unanswered === 5 && seed.trapSafe === 8, "C1 seed gate line 25/25/0/5 traps 8/8");
for (const id of ["d01", "d02", "d03"]) {
  const r = seed.rows.find((x) => x.id === id);
  check(r.verdict === "answered" && r.result === "correct" && r.answerKey === GOLD_KEYS[id], `C1 ${id} answered-correct with gold key ${GOLD_KEYS[id]}`);
}

check(baseline.pass, "C2 baseline gate PASS (wrong 0, correct >= 11, traps safe)");
check(baseline.correct === 22 && baseline.wrong === 0 && baseline.unanswered === 8 && baseline.trapSafe === 8, "C2 empty map reproduces 22/22/0/8 traps 8/8");
const un = baseline.rows.filter((r) => r.result === "unanswered").map((r) => r.id).sort();
check(JSON.stringify(un) === JSON.stringify([...ABSTENTIONS].sort()), "C2 unanswered set exactly h08 h09 p01 p02 p03 d01 d02 d03");

let c3ok = true;
for (const b of baseline.rows) {
  if (b.result !== "correct") continue;
  const s = seed.rows.find((x) => x.id === b.id);
  if (!(s.verdict === b.verdict && s.result === "correct" && s.answerKey === b.answerKey)) c3ok = false;
}
check(c3ok, "C3 every already-correct row keeps verdict + answer key under the seed");

const expanded = expandTokens(["m4a1", "alpha", "july", "bravo", "kept"], seedMap);
check(
  JSON.stringify(expanded) === JSON.stringify(["bspace", "clip", "caf", "dba", "alpha", "idle", "bravo", "keep"]),
  "C4 unknown tokens pass through, aliased terms replaced in place (keeps normalized to keep via tokens())"
);
const p1 = runAsk("what was the root cause of the collapsed arms regression", null, {});
const p2 = runAsk("what was the root cause of the collapsed arms regression", null, { QOL_MEMORY_ALIASES_DISABLE: "1" });
check(JSON.stringify(p1) === JSON.stringify(p2), "C4 non-aliased query byte-identical with map present vs absent");

const capPath = join(tmpdir(), "qol-memory-alias-cap.json");
writeFileSync(capPath, JSON.stringify({ schema: 1, aliases: { x: ["one", "two", "three", "four", "five"] } }));
const capMap = loadAliases(capPath);
check(Array.isArray(capMap.get("x")) && capMap.get("x").length === 4 && JSON.stringify(capMap.get("x")) === JSON.stringify(["one", "two", "three", "four"]), "C5 alias with 5 expansions loads only 4");
check(seedMap.get("m4a1").length === 4, "C5 m4a1 4-term entry loads fully");
check(ALIAS_CAP === 4, "C5 CAP is the constant 4");

const diffs = [];
for (const b of baseline.rows) {
  const s = seed.rows.find((x) => x.id === b.id);
  if (s.verdict !== b.verdict || s.result !== b.result || s.answerKey !== b.answerKey) diffs.push(b.id);
}
check(JSON.stringify(diffs) === JSON.stringify(["d01", "d02", "d03"]), "C6 per-row gate diff exactly d01/d02/d03, nothing else");

const idxFiles = readdirSync(FROZEN).filter((n) => n.startsWith("idx-")).sort();
const idxBefore = idxFiles.map((n) => `${n}:${statSync(join(FROZEN, n)).mtimeMs}`).join("|");
runAsk("what was the root cause of the m4a1 arms collapsing", null, {});
const idxAfter = idxFiles.map((n) => `${n}:${statSync(join(FROZEN, n)).mtimeMs}`).join("|");
check(idxBefore === idxAfter, "C7 aliased ask leaves all idx-*.json + .meta mtimes untouched");
const w1 = runAsk("what was the root cause of the collapsed arms regression", null, {});
const w2 = runAsk("what was the root cause of the collapsed arms regression", null, { QOL_MEMORY_ALIASES_DISABLE: "1" });
check(JSON.stringify(w1) === JSON.stringify(w2), "C7 warm aliased vs unaliased ask byte-identical for a non-aliased query");

const det1 = runAsk("what was the root cause of the m4a1 arms collapsing", null, {});
const det2 = runAsk("what was the root cause of the m4a1 arms collapsing", null, {});
check(JSON.stringify(det1) === JSON.stringify(det2), "C8 same query twice byte-identical stdout");

const bd01 = baseline.rows.find((x) => x.id === "d01");
const sd01 = seed.rows.find((x) => x.id === "d01");
check(bd01.result === "unanswered" && sd01.result === "correct", "C9 kill switch ablates the seed (d01 unanswered disabled, answered-correct enabled)");

const normPath = join(tmpdir(), "qol-memory-alias-norm.json");
writeFileSync(normPath, JSON.stringify({ schema: 1, aliases: { caf: ["i_caf", "per key"] } }));
const normMap = loadAliases(normPath);
check(JSON.stringify(normMap.get("caf")) === JSON.stringify(["caf", "per", "key"]), "C10 i_caf contributes caf, per key contributes per and key");

const missing = loadAliases(join(tmpdir(), "qol-memory-alias-does-not-exist.json"));
check(missing.size === 0, "C11 missing file loads as empty map");
const corruptPath = join(tmpdir(), "qol-memory-alias-corrupt.json");
writeFileSync(corruptPath, "{not json");
check(loadAliases(corruptPath).size === 0, "C11 corrupt file loads as empty map");
const schemaPath = join(tmpdir(), "qol-memory-alias-schema.json");
writeFileSync(schemaPath, JSON.stringify({ schema: 2, aliases: { a: ["b"] } }));
check(loadAliases(schemaPath).size === 0, "C11 wrong schema loads as empty map");
check(!validate(corruptPath).ok && !validate(schemaPath).ok, "C11 validate rejects corrupt and wrong-schema files");

check(seed.traps.every((t) => t.safe), "C12 all 8 traps safe under the seed set");

console.log(failed ? `test-alias FAILED ${failed}` : "test-alias ALL PASS");
process.exit(failed ? 1 : 0);
