#!/usr/bin/env node
import { createHash } from "node:crypto";
import { existsSync, readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { join, resolve, dirname, basename } from "node:path";
import { homedir } from "node:os";
import { fileURLToPath } from "node:url";
import { qolMemoryStore } from "./lib/store-path.js";
import { acquireDistillLock } from "./lib/distill-lock.js";

const BASE = dirname(fileURLToPath(import.meta.url));
const ARGS = process.argv.slice(2);
const pick = (flag, def) => {
  const i = ARGS.indexOf(flag);
  return i >= 0 && ARGS[i + 1] ? ARGS[i + 1] : def;
};
const RUN = pick("--snapshot-run", null);
const WITH_ARTIFACTS = ARGS.includes("--with-artifacts");
const PINNED =
  RUN ||
  JSON.parse(readFileSync(join(BASE, "eval", "questions.json"), "utf8")).run_pin ||
  "2026-08-10T19-18-33-961Z";
const STORE_ROOT = resolve(pick("--store", qolMemoryStore()));
const SNAPSHOT_DIR = join(STORE_ROOT, "snapshot");
const OUT_DIR = join(STORE_ROOT, "notes");
const TS = new Date().toISOString();

const LOCK_HELD = process.env.QOL_MEMORY_DISTILL_LOCK_HELD === "1";
let lock = null;
if (!LOCK_HELD) {
  lock = acquireDistillLock(STORE_ROOT, "notes");
  if (!lock) {
    console.log("distill skipped: lock busy (mode notes)");
    process.exit(0);
  }
  process.on("exit", () => lock.release());
}

const TRIGGERS = [
  {
    cls: "path",
    re: /\b([A-Za-z0-9_][A-Za-z0-9_./-]*\.(?:md|json|jsonl|toml|rs|mjs|js|py|sh|yaml|yml))\b/g,
    phrase: (m) => `path ${m[1]}`,
  },
  {
    cls: "flag",
    re: /\b(--[a-z][a-z0-9-]*)\b/g,
    phrase: (m) => `flag ${m[1]}`,
  },
  {
    cls: "version",
    re: /\b(schema\s?[Vv]ersion)\s*[:= ]?\s*(\d+)\b/g,
    phrase: (m) => `schema version ${m[2]}`,
  },
  {
    cls: "model",
    re: /\b([a-z][a-z0-9-]*-v[0-9]+(?:\.[0-9]+)+)\b/g,
    phrase: (m) => `model ${m[1]}`,
  },
  {
    cls: "count",
    re: /(?<![\d,])\b(0|[1-9]\d*)\s+(compaction|memory|corpus|session|file|message)\s+(units?|messages?|sessions?|files?)\b/g,
    phrase: (m) => `count ${m[1]} ${m[2]} ${m[3]} in the corpus`,
  },
  {
    cls: "status",
    re: /\bstatus\b[^\n]{0,60}\b(pass|degraded|fail)\b|\b(pass|degraded|fail)\b[^\n]{0,30}\bstatus\b/g,
    phrase: (m) => `status ${m[1] || m[2]}`,
  },
  {
    cls: "command",
    re: /^((?:node|cargo|python3|qol|git|ls|cat|\.\/|rm|mv|cp)[a-z0-9_.\-/ =]*)$/gm,
    phrase: (m) => `command ${m[1].trim().slice(0, 120)}`,
  },
  {
    cls: "format",
    re: /\bJSONL\b/g,
    phrase: () => "format JSONL",
  },
  {
    cls: "policy",
    re: /\b(?:never|always|must|should not|shouldn'?t|don'?t|do not|have to)\b[^\n]{0,150}/g,
    phrase: (m) => `policy ${m[0].trim().slice(0, 150)}`,
    accept: (line) =>
      /\b(you|we|your)\b/i.test(line) &&
      !/[`"=:#]/.test(line) &&
      !line.includes("?") &&
      line.length <= 200,
  },
];

function normalize(text) {
  return text
    .toLowerCase()
    .replace(/[`"'(),;:]/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

function sentenceWindow(text, matchIndex, span = 180) {
  const lineStart = text.lastIndexOf("\n", matchIndex) + 1;
  const lineEnd = text.indexOf("\n", matchIndex);
  const line = text.slice(lineStart, lineEnd < 0 ? undefined : lineEnd).trim();
  if (line.length > 24 && line.length <= 400) {
    return line.replace(/`/g, "").replace(/\s+/g, " ").trim();
  }
  const start = Math.max(0, matchIndex - 60);
  let s = text.slice(start, matchIndex + span).replace(/`/g, "");
  s = s.replace(/\s+/g, " ").trim();
  if (start > 0) s = "..." + s;
  return s.slice(0, 240);
}

function noteKey(cls, text) {
  return createHash("sha256").update([cls, normalize(text)].join("|")).digest("hex").slice(0, 16);
}

function lineOf(text, matchIndex) {
  const lineStart = text.lastIndexOf("\n", matchIndex) + 1;
  const lineEnd = text.indexOf("\n", matchIndex);
  return text.slice(lineStart, lineEnd < 0 ? undefined : lineEnd);
}

function extractTranscript(units) {
  const variants = new Map();
  for (const u of units) {
    for (const t of TRIGGERS) {
      t.re.lastIndex = 0;
      let m;
      while ((m = t.re.exec(u.text))) {
        if (t.accept && !t.accept(lineOf(u.text, m.index))) continue;
        const phrase = t.phrase(m);
        const text = `${phrase} | ${sentenceWindow(u.text, m.index)}`;
        const norm = normalize(text);
        const fam = normalize(phrase);
        if (!variants.has(fam)) variants.set(fam, []);
        variants.get(fam).push({ key: noteKey(t.cls, text), cls: t.cls, text, source_key: u.key, source_ts: u.ts, source_kind: u.kind, len: text.length, fam });
      }
    }
  }
  const notes = [];
  for (const [fam, vs] of variants) {
    const usable = vs.filter((v) => v.len >= 20 && v.len <= 240);
    const pick = (usable.length ? usable : vs).sort((a, b) => b.len - a.len)[0];
    if (pick.len > 240) pick.text = pick.text.slice(0, 240);
    notes.push({ key: noteKey(pick.cls, pick.text), cls: pick.cls, text: pick.text, source_key: pick.source_key, source_ts: pick.source_ts, source_kind: pick.source_kind, fam });
  }
  return notes;
}

function extractArtifacts() {
  const notes = [];
  const report = JSON.parse(readFileSync(join(SNAPSHOT_DIR, PINNED, "report.json"), "utf8"));
  const script = readFileSync(join(BASE, "snapshot.mjs"), "utf8");
  const artifactTs =
    report.started_at ||
    new Date(PINNED.replace(/-([0-9]{2})Z$/, ".$1Z").replace(/T(\d{2})-(\d{2})-(\d{2})/, "T$1:$2:$3")).toISOString();
  const mk = (cls, text) => ({
    key: noteKey(cls, text),
    cls,
    text,
    source_key: "artifact",
    source_ts: artifactTs,
    source_kind: "artifact",
  });
  for (const cmd of report.commands || []) notes.push(mk("command", `command ${cmd}`));
  notes.push(mk("version", `schema version ${report.schemaVersion}`));
  const byKind = report.stats.units.byKind;
  for (const k of Object.keys(byKind)) {
    if (k === "branch") continue;
    notes.push(mk("count", `count ${byKind[k]} ${k} units`));
  }
  const keep = report.inputs.keep;
  notes.push(mk("flag", `flag --keep ${keep} (snapshot run retention)`));
  notes.push(mk("path", `path ${STORE_ROOT.replace(homedir(), "~")}/snapshot/ (snapshot report output dir, store root)`));
  const keyLine = script.match(/unitKey\(source, file, ts, text\) \{[\s\S]{0,220}/);
  if (keyLine) {
    notes.push(mk("unitkey", `unit key is made of sha256(source|file|ts|text), first 16 hex chars | ${keyLine[0].replace(/\s+/g, " ")}`));
  }
  notes.push(mk("status", "snapshot report statuses when a source errors: pass/degraded/fail"));
  if (script.includes(".jsonl")) {
    notes.push(mk("format", "format JSONL: pi and claude session transcripts are stored as JSONL files"));
  }
  if (report.status === "degraded") {
    notes.push(mk("status", `status degraded (this pinned run)`));
  }
  const cfg = (text) => notes.push(mk("config", text));
  if (report.inputs && report.inputs.maxSamples) {
    cfg(`config maxSamples ${report.inputs.maxSamples} (max sessions sampled per source)`);
  }
  const depth = script.match(/const MAX_DEPTH = (\d+)/);
  if (depth) cfg(`config MAX_DEPTH ${depth[1]} (the snapshot walks the session tree this deep)`);
  const dirs = [
    ["pi", report.inputs && report.inputs.piDir],
    ["claude", report.inputs && report.inputs.claudeDir],
  ];
  for (const [name, dir] of dirs) {
    if (dir) notes.push(mk("path", `path ${dir.replace(homedir(), "~")} (${name} session transcripts live here)`));
  }
  const dedupeKey = report.stats && report.stats.units && report.stats.units.dedupe && report.stats.units.dedupe.key;
  if (dedupeKey) cfg(`config dedupe key ${dedupeKey} (snapshot dedupe key)`);
  const evalSrc = readFileSync(join(BASE, "eval", "eval.mjs"), "utf8");
  const rrf = evalSrc.match(/RRF_K = Number\(pick\("--rrf-k", "(\d+)"\)\)/);
  if (rrf) cfg(`config rrf k ${rrf[1]} (eval default)`);
  cfg(`config ${TRIGGERS.length} trigger classes in notes.mjs`);
  const langSample = report.stats && report.stats.lang && report.stats.lang.sampled;
  if (langSample) cfg(`config ${langSample} sampled units for the language check`);
  const flags = script.matchAll(/args\.includes\("(--[a-z-]+)"\)/g);
  for (const m of flags) {
    const desc = m[1] === "--with-assistant" ? "include assistant messages in the snapshot" : `disable snapshot dedupe`;
    notes.push(mk("flag", `flag ${m[1]} (${desc})`));
  }
  const dimsConfig = join(homedir(), ".cache", "qol-memory", "bge-small-en-v1.5", "config.json");
  if (existsSync(dimsConfig)) {
    try {
      const dims = JSON.parse(readFileSync(dimsConfig, "utf8")).hidden_size;
      if (dims) cfg(`config bge embedding dimensions ${dims}`);
    } catch {
    }
  }
  return notes;
}

const snapshot = readFileSync(join(SNAPSHOT_DIR, PINNED, "snapshot.jsonl"), "utf8")
  .split("\n")
  .filter(Boolean)
  .map((l) => JSON.parse(l));
const units = snapshot.filter((u) => u.kind === "user" || u.kind === "assistant");

const started = Date.now();
const transcriptNotes = extractTranscript(units);
const artifactNotes = WITH_ARTIFACTS ? extractArtifacts() : [];
const all = [...artifactNotes, ...transcriptNotes];

const seen = new Set();
const notes = all.filter((n) => {
  const norm = normalize(n.text);
  if (seen.has(norm)) return false;
  seen.add(norm);
  return true;
});

const byClass = {};
for (const n of notes) byClass[n.cls] = (byClass[n.cls] || 0) + 1;

const outDir = join(OUT_DIR, TS);
try {
  mkdirSync(outDir, { recursive: true });
  writeFileSync(join(outDir, "notes.jsonl"), notes.map((n) => JSON.stringify(n)).join("\n") + "\n");

  const report = {
    name: "qol-memory notes (tier 2 consolidation probe)",
    schemaVersion: 2,
    started_at: new Date(started).toISOString(),
    finished_at: new Date().toISOString(),
    status: notes.length > 0 ? (byClass.command || byClass.count ? "pass" : "degraded") : "fail",
    inputs: { snapshotRun: PINNED, unitsIndexed: units.length, withArtifacts: WITH_ARTIFACTS },
    artifacts: { notes: `${TS}/notes.jsonl`, report: `${TS}/report.json` },
    commands: [`node ${basename(BASE)}/notes.mjs --snapshot-run ${PINNED}${WITH_ARTIFACTS ? " --with-artifacts" : ""}`],
    stats: { notes: notes.length, byClass },
    next: ["Annotate note_key for the file-category eval questions", "Score the notes layer with eval.mjs --notes"],
  };
  writeFileSync(join(outDir, "report.json"), JSON.stringify(report, null, 2));

  console.log(
    `notes: ${notes.length} (transcript ${transcriptNotes.length}, artifact ${artifactNotes.length}) | classes: ${JSON.stringify(byClass)}`
  );
  console.log(`report: ${outDir}/report.json`);
} finally {
  if (lock) lock.release();
}
