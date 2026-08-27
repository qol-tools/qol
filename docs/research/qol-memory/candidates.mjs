#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { appendFileSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { qolMemoryStore } from "./lib/store-path.js";
import { parseUnitsText } from "./lib/seal.js";
import { normalizeQuery, isMiss, candidateKey, discriminatorCount, countPendingCandidates, CANDIDATE_COOLDOWN_MS } from "./lib/retrieval-log.js";

const BASE = dirname(fileURLToPath(import.meta.url));
const VERDICT_EVAL = join(BASE, "eval", "verdict-eval.mjs");
const DEFAULT_HELDOUT = join(BASE, "eval", "heldout.json");
const PINNED_SNAPSHOT = "2026-08-12T18-46-58-129Z";
const PINNED_NOTES = "2026-08-13T16:31:40.844Z";

const args = process.argv.slice(2);
const pick = (flag, def) => {
  const i = args.indexOf(flag);
  return i >= 0 && args[i + 1] ? args[i + 1] : def;
};
const STORE = resolve(pick("--store", qolMemoryStore()));
const HELDOUT = resolve(pick("--heldout", DEFAULT_HELDOUT));

function readJsonLines(path) {
  try {
    return parseUnitsText(readFileSync(path, "utf8"));
  } catch {
    return [];
  }
}

function readHeldout() {
  return JSON.parse(readFileSync(HELDOUT, "utf8"));
}

function readCandidates() {
  return readJsonLines(join(STORE, "candidates.jsonl"));
}

function writeTmpRename(path, data) {
  mkdirSync(dirname(path), { recursive: true });
  const tmp = path + ".tmp";
  writeFileSync(tmp, data);
  renameSync(tmp, path);
}

function writeCandidates(items) {
  writeTmpRename(join(STORE, "candidates.jsonl"), items.map((c) => JSON.stringify(c)).join("\n") + (items.length ? "\n" : ""));
}

function noteOf(event) {
  const keys = event.recalled_keys || [];
  const run = event.signals && event.signals.notes_run_ts;
  if (!keys.length || !run) return null;
  try {
    const notes = parseUnitsText(readFileSync(join(STORE, "notes", run, "notes.jsonl"), "utf8"));
    const hit = notes.find((n) => n.key === keys[0]);
    return hit && typeof hit.text === "string" ? { key: hit.key, text: hit.text } : null;
  } catch {
    return null;
  }
}

function buildCandidate(event, norm) {
  const note = noteOf(event);
  return {
    key: candidateKey(norm),
    query: event.query,
    norm_query: norm,
    fact: note ? note.text : null,
    fact_norm: note ? normalizeQuery(note.text) : null,
    source_unit_key: note ? note.key : null,
    source_event_ts: event.ts || null,
    source: event.source || "ask-cli",
    session: event.session || null,
    cwd: event.cwd || null,
    verdict: event.verdict,
    created_ts: new Date().toISOString(),
    status: "candidate",
    promoted_ts: null,
    heldout_id: null,
    rejected_ts: null,
    reject_reason: null,
  };
}

function harvest() {
  const events = readJsonLines(join(STORE, "retrievals.jsonl"));
  const misses = events
    .filter((e) => isMiss(e) && (e.source === "ask-cli" || e.source === "tool"))
    .sort((a, b) => (a.ts < b.ts ? -1 : a.ts > b.ts ? 1 : 0));
  const heldoutNorms = new Set((readHeldout().questions || []).map((q) => normalizeQuery(q.query)));
  const latestCapture = new Map();
  for (const c of readCandidates()) {
    const t = Date.parse(c.created_ts || "");
    if (Number.isNaN(t)) continue;
    latestCapture.set(c.norm_query, Math.max(latestCapture.get(c.norm_query) || 0, t));
  }
  const captured = new Set();
  const added = [];
  const skipped = { heldout: 0, cooldown: 0, duplicate: 0 };
  for (const e of misses) {
    const norm = normalizeQuery(e.query);
    if (!norm || captured.has(norm)) {
      if (norm) skipped.duplicate++;
      continue;
    }
    if (heldoutNorms.has(norm)) {
      skipped.heldout++;
      continue;
    }
    const ts = Date.parse(e.ts || "");
    const last = latestCapture.get(norm) || 0;
    if (!Number.isNaN(ts) && last && ts - last < CANDIDATE_COOLDOWN_MS) {
      skipped.cooldown++;
      continue;
    }
    captured.add(norm);
    const candidate = buildCandidate(e, norm);
    latestCapture.set(norm, Date.parse(candidate.created_ts));
    added.push(candidate);
  }
  if (added.length) {
    mkdirSync(STORE, { recursive: true });
    appendFileSync(join(STORE, "candidates.jsonl"), added.map((c) => JSON.stringify(c)).join("\n") + "\n");
  }
  const pending = countPendingCandidates(STORE);
  const report = {
    name: "qol-memory candidates harvest",
    schemaVersion: 1,
    ran_at: new Date().toISOString(),
    store: STORE,
    harvest: {
      misses: misses.length,
      candidates_added: added.length,
      skipped,
    },
    candidates: added,
    pending,
  };
  const outDir = join(STORE, "ingest");
  mkdirSync(outDir, { recursive: true });
  writeFileSync(join(outDir, "report.json"), JSON.stringify(report, null, 2));
  console.log(`candidates harvest | misses ${misses.length} | added ${added.length} | skipped ${skipped.heldout} heldout ${skipped.cooldown} cooldown ${skipped.duplicate} duplicate | pending ${pending}`);
  return added;
}

function promote(key) {
  const candidates = readCandidates();
  const candidate = candidates.find((c) => c.key === key);
  if (!candidate) {
    console.error(`promote ${key}: no candidate with this key`);
    process.exit(1);
  }
  if (candidate.status !== "candidate") {
    console.error(`promote ${key}: status is ${candidate.status}, only candidate can promote`);
    process.exit(1);
  }
  const heldout = readHeldout();
  const tempHeldout = {
    ...heldout,
    questions: [...(heldout.questions || []), { id: key, query: candidate.query, fact: candidate.fact || "" }],
  };
  const tempPath = join(tmpdir(), `qol-memory-promote-${key}.json`);
  writeFileSync(tempPath, JSON.stringify(tempHeldout, null, 2));
  const r = spawnSync("node", [VERDICT_EVAL, "--store", STORE, "--heldout", tempPath], { encoding: "utf8", timeout: 600000, maxBuffer: 64 * 1024 * 1024 });
  rmSync(tempPath, { force: true });
  const stdout = r.stdout || "";
  const gatePass = r.status === 0;
  const rowMatch = stdout.match(new RegExp(`^\\s+${key}\\s+\\S+\\s+(correct|wrong|unanswered)\\s`, "m"));
  const rowResult = rowMatch ? rowMatch[1] : null;
  let noteTexts = [];
  try {
    const frozenNotes = join(tmpdir(), "qol-memory-verdict-eval", `${PINNED_SNAPSHOT}__${PINNED_NOTES}`, "notes", PINNED_NOTES, "notes.jsonl");
    noteTexts = parseUnitsText(readFileSync(frozenNotes, "utf8")).map((n) => n.text);
  } catch {}
  const disc = discriminatorCount(candidate.fact, noteTexts);
  if (!(gatePass && rowResult === "correct" && disc === 1)) {
    console.log(stdout);
    console.error(`promote ${key}: FAIL gate=${gatePass ? "PASS" : "FAIL"} row=${rowResult} discriminator=${disc}`);
    process.exit(1);
  }
  const rowKeyMatch = stdout.match(new RegExp(`^\\s+${key}\\s+\\S+\\s+correct\\s.*?::([0-9a-f]{16})\\s*$`, "m"));
  const answerKey = rowKeyMatch ? rowKeyMatch[1] : null;
  if (answerKey !== candidate.source_unit_key) {
    console.log(stdout);
    console.error(`promote ${key}: FAIL answer_key=${answerKey || "none"} source_unit_key=${candidate.source_unit_key}`);
    process.exit(1);
  }
  heldout.questions.push({ id: key, query: candidate.query, fact: candidate.fact });
  writeTmpRename(HELDOUT, JSON.stringify(heldout, null, 2) + "\n");
  writeCandidates(candidates.map((c) => (c.key === key ? { ...c, status: "promoted", promoted_ts: new Date().toISOString(), heldout_id: key } : c)));
  console.log(`promote ${key}: PASS gate=${gatePass ? "PASS" : "FAIL"} row=${rowResult} discriminator=${disc} heldout=${HELDOUT} status promoted`);
}

function reject(key, reason) {
  if (!reason) {
    console.error(`reject ${key}: --reason is required`);
    process.exit(1);
  }
  const candidates = readCandidates();
  const candidate = candidates.find((c) => c.key === key);
  if (!candidate) {
    console.error(`reject ${key}: no candidate with this key`);
    process.exit(1);
  }
  if (candidate.status !== "candidate") {
    console.error(`reject ${key}: status is ${candidate.status}, only candidate can be rejected`);
    process.exit(1);
  }
  writeCandidates(candidates.map((c) => (c.key === key ? { ...c, status: "rejected", rejected_ts: new Date().toISOString(), reject_reason: reason } : c)));
  console.log(`reject ${key}: status rejected (${reason})`);
}

const command = args.find((a) => a === "harvest" || a === "count");
if (args.includes("--promote")) {
  promote(pick("--promote", ""));
} else if (args.includes("--reject")) {
  reject(pick("--reject", ""), pick("--reason", ""));
} else if (command === "count") {
  console.log(String(countPendingCandidates(STORE)));
} else if (command === "harvest") {
  harvest();
} else {
  console.error("usage: node candidates.mjs --store PATH harvest|count|--promote KEY [--heldout PATH]|--reject KEY --reason R");
  process.exit(1);
}
