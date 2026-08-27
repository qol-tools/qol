#!/usr/bin/env node
import { readdirSync, readFileSync, writeFileSync, mkdirSync, existsSync } from "node:fs";
import { join, resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { qolMemoryStore } from "./lib/store-path.js";
import { bm25Ranks, snippet, tokens, buildIndex } from "./lib/retrieval.js";
import { buildOrLoad } from "./lib/indexcache.js";
import { trySealedText, parseUnitsText } from "./lib/seal.js";
import { appendRetrieval } from "./lib/retrieval-log.js";
import { loadAliases, expandTokens, expandTokensKeep } from "./lib/concept-aliases.js";
import { walkSkills, buildMetaDoc, probeFresh, serveSection, poolTokens, bestSection } from "./lib/skills-pool.js";

const ASK_BASE = dirname(fileURLToPath(import.meta.url));
const T0 = Date.now();
const DEFAULT_SKILLS_ROOT = resolve(join(ASK_BASE, "..", "..", "..", "..", "..", "..", "qol-skills"));

const args = process.argv.slice(2);
const pick = (flag, def) => {
  const i = args.indexOf(flag);
  return i >= 0 && args[i + 1] ? args[i + 1] : def;
};
const STORE_ROOT = resolve(pick("--store", qolMemoryStore()));
const K = Number(pick("--k", "5"));
const BRIEF = args.includes("--brief");
const LOG_SOURCE = pick("--log-source", "ask-cli");
const LOG_CWD = pick("--log-cwd", null);
const LOG_FACT = pick("--log-fact", null);
const NO_LOG = args.includes("--no-log");

try {
  mkdirSync(STORE_ROOT, { recursive: true });
  writeFileSync(join(STORE_ROOT, "manifest.json"), JSON.stringify({ schema: 1, ask_mjs: join(ASK_BASE, "ask.mjs"), retrievals: "qol-memory-retrieval-v1", candidates: "qol-memory-candidates-v1", concept_aliases: "qol-memory-concept-aliases-v1" }));
} catch {}

const STOPWORDS = new Set(["what", "when", "where", "which", "who", "how", "do", "does", "did", "is", "are", "the", "a", "an", "to", "for", "of", "in", "on", "with", "and", "or", "me", "you", "my", "we", "i", "it", "have", "has", "be", "been", "was", "were", "many", "much", "exist", "really", "want", "should", "could", "would", "can", "work", "fix", "this", "that", "these", "those", "there", "about", "get", "make", "use", "tell", "explain"]);

const ALIASES = process.env.QOL_MEMORY_ALIASES_DISABLE === "1" ? new Map() : loadAliases(join(ASK_BASE, "..", "..", "..", "plugins", "qol-memory", "assets", "concept-aliases.json"));

const BOILERPLATE_MARKERS = [
  "[qol session bridge]",
  "Base directory for this skill:",
  "continued from a previous conversation",
  "Review this change for security vulnerabilities",
];

const RECENCY_CLS = new Set(["count", "status", "version", "flag", "config", "decision", "decision-deter"]);
const STALE_CLS = new Set(["count", "status", "version"]);
const CURATED_KINDS = new Set(["artifact", "decision", "decision-deter"]);
const KIND_RANK = { "decision-deter": 3, "artifact": 2, "decision": 1, "user": 0 };

function latestRun(root) {
  const runs = readdirSync(root)
    .filter((n) => /^\d{4}-\d{2}-\d{2}T/.test(n))
    .sort()
    .reverse();
  if (!runs.length) throw new Error(`no runs under ${root}`);
  return runs[0];
}

function runTime(run) {
  return new Date(run.replace(/T(\d{2})-(\d{2})-(\d{2})-(\d{3})Z$/, "T$1:$2:$3.$4Z"));
}

function readUnits(snapshotRoot) {
  const livePath = join(STORE_ROOT, "units.jsonl");
  if (existsSync(livePath)) {
    const raw = readFileSync(livePath);
    const text = trySealedText(STORE_ROOT, raw) || raw.toString("utf8");
    return { run: "live", path: livePath, items: parseUnitsText(text) };
  }
  const run = latestRun(snapshotRoot);
  const runPath = join(snapshotRoot, run, "snapshot.jsonl");
  return { run, path: runPath, items: parseUnitsText(readFileSync(runPath, "utf8")) };
}

function dedupeUserUnits(units) {
  const seen = new Set();
  return [...units]
    .sort((a, b) => new Date(a.ts || 0).getTime() - new Date(b.ts || 0).getTime())
    .filter((u) => {
      const norm = (u.text || "").toLowerCase().replace(/\s+/g, " ").trim();
      if (seen.has(norm)) return false;
      seen.add(norm);
      return true;
    });
}

function readNotes(notesRoot) {
  const runs = readdirSync(notesRoot).filter((n) => /^\d{4}-\d{2}-\d{2}T/.test(n)).sort().reverse();
  if (!runs.length) return { run: null, items: [] };
  return { run: runs[0], items: readFileSync(join(notesRoot, runs[0], "notes.jsonl"), "utf8").trim().split("\n").filter(Boolean).map((l) => JSON.parse(l)) };
}

function isBoilerplateUnit(u) {
  return BOILERPLATE_MARKERS.some((m) => u.text.includes(m));
}

function familyKey(note) {
  const head = (note.text || "").split(" | ")[0].replace(/\d+/g, "#").toLowerCase();
  const core = head.replace(/ in the corpus$/, "").replace(/\(.*\)$/, "").trim();
  return (note.cls || "") + ":" + core.slice(0, 60);
}

const query = args.find((a) => !a.startsWith("--"));
if (!query) {
  console.error('usage: node ask.mjs "<query>" [--store PATH] [--k N] [--exclude-session ID] [--brief] [--log-source ask-cli|tool|eval] [--log-cwd PATH] [--log-fact FACT] [--no-log]   quote the query, --brief recommended, QOL_MEMORY_STORE overrides the store');
  process.exit(1);
}

const snapshot = join(STORE_ROOT, "snapshot");
const notesRoot = join(STORE_ROOT, "notes");
const { run: snapshotRun, path: unitsPath, items: allUnits } = readUnits(snapshot);
const userUnits = dedupeUserUnits(allUnits.filter((u) => u.kind === "user" || u.kind === "capture"));
const { run: notesRun, items: notes } = readNotes(notesRoot);

const qtokens0 = tokens(query).filter((t) => !STOPWORDS.has(t));
const qtokens = expandTokens(qtokens0, ALIASES);

function distinctScore(qt, text) {
  const lower = text.toLowerCase();
  let matched = 0;
  for (const t of qt) if (lower.includes(t)) matched++;
  return { matched, total: qt.length };
}

const EXCLUDE_SESSION = pick("--exclude-session", "");
const answerPool = userUnits.filter((u) => !isBoilerplateUnit(u) && (!EXCLUDE_SESSION || u.session !== EXCLUDE_SESSION));
const answerIdx = buildOrLoad(STORE_ROOT, EXCLUDE_SESSION ? `pool-x-${EXCLUDE_SESSION.slice(0, 8)}` : "pool", answerPool, unitsPath);
const unitsQuery = expandTokensKeep(tokens(query), ALIASES).join(" ");
const answerRanked = bm25Ranks(unitsQuery, answerIdx, null, K).map(({ key, score }) => {
  const u = answerPool.find((x) => x.key === key);
  return { key, score, kind: u.kind, source: u.source, session: u.session, cwd: u.cwd, ts: u.ts, text: u.text };
});

const allIdx = buildOrLoad(STORE_ROOT, "user", userUnits, unitsPath);
const rankedAll = bm25Ranks(unitsQuery, allIdx, null, K).map(({ key, score }) => {
  const u = userUnits.find((x) => x.key === key);
  return { key, score, kind: u.kind, text: u.text, source: u.source, session: u.session, cwd: u.cwd, ts: u.ts };
});
const topUnits = rankedAll.map((u) => ({ ...u, snippet: snippet(u.text, qtokens) }));

const notesIdx = notes.length ? buildOrLoad(STORE_ROOT, "notes", notes) : null;
const topNotes = notesIdx ? bm25Ranks(expandTokens(qtokens0, ALIASES).join(" "), notesIdx, null, 5).map(({ key, score }) => {
  const n = notes.find((x) => x.key === key);
  return { key, cls: n.cls, text: n.text, source_key: n.source_key, source_ts: n.source_ts, source_kind: n.source_kind, score };
}) : [];

const skillsIndexPath = join(STORE_ROOT, "skills", "index.json");
let skillsIndex = null;
if (existsSync(skillsIndexPath)) {
  try {
    skillsIndex = JSON.parse(readFileSync(skillsIndexPath, "utf8"));
  } catch {
    skillsIndex = null;
  }
}
const skillsRoot = (skillsIndex && skillsIndex.root) || process.env.QOL_MEMORY_SKILLS_ROOT || DEFAULT_SKILLS_ROOT;
const skillsFreshness = skillsIndex ? probeFresh(skillsIndex, skillsRoot) : "not-indexed";
let skillsHits = [];
if (skillsIndex && skillsIndex.skills && skillsIndex.skills.length) {
  const metaDocs = skillsIndex.skills.map((s) => ({ key: s.id, text: buildMetaDoc(s) }));
  const skillsIdx = buildIndex(metaDocs);
  const qt = poolTokens(query).filter((t) => !STOPWORDS.has(t));
  const ranked = bm25Ranks(query, skillsIdx, null, 5);
  const seen = new Set();
  for (const { key, score } of ranked) {
    if (seen.has(key)) continue;
    seen.add(key);
    const s = skillsIndex.skills.find((x) => x.id === key);
    if (!s) continue;
    const best = bestSection(s, skillsRoot, qt, skillsIdx.idf);
    const served = best ? serveSection(s, skillsRoot, best.h, 2048) : serveSection(s, skillsRoot, null, 2048);
    skillsHits.push({
      id: s.id,
      name: s.name,
      score: Number(score.toFixed(2)),
      section: served.ok ? served.section : best ? best.h : null,
      content: served.ok ? served.content : null,
      truncated: served.ok ? served.truncated : false,
      hash_match: served.ok ? served.hash_match : false,
      status: served.ok ? "served" : served.reason,
      head: skillsIndex.repo ? skillsIndex.repo.head : null,
      dirty: skillsIndex.repo ? !!skillsIndex.repo.dirty : null,
    });
  }
}

function phrasedCoverage(q, results) {
  if (!results.length) return 0;
  const { matched, total } = distinctScore(q, results[0].text || "");
  return total ? matched / total : 0;
}

function weightedNoteCov(q, noteText, idfMap) {
  let num = 0;
  let den = 0;
  const lower = (noteText || "").toLowerCase();
  for (const t of q) {
    const w = idfMap.get(t) || 0;
    den += w;
    if (lower.includes(t)) num += w;
  }
  return den ? num / den : 0;
}

let verdict = "no-memory";
let confidence = "none";
let answer = null;
let related = [];
let reason = "no memory above the answer threshold";

const noteTop = topNotes[0];
const unitTop = answerRanked[0];
const unitMajorRank = answerRanked[1];
const unitMargin = unitTop && unitMajorRank ? unitTop.score / unitMajorRank.score : unitTop ? Infinity : 0;
const hasMultiIntent = topNotes.length >= 2 && topNotes[1] && distinctScore(qtokens, topNotes[1].text).matched >= 2 && familyKey(topNotes[1]) !== familyKey(topNotes[0]);

let noteResolved = topNotes[0] || null;
let noteSuperseded = null;
let scoreResolved = true;
if (noteResolved && RECENCY_CLS.has(noteResolved.cls)) {
  const sameFamily = topNotes.filter((r) => familyKey(r) === familyKey(noteResolved) && r.source_ts !== noteResolved.source_ts);
  if (sameFamily.length) {
    const byTs = [noteResolved, ...sameFamily].sort((a, b) => new Date(b.source_ts || 0) - new Date(a.source_ts || 0));
    noteResolved = byTs[0];
    noteSuperseded = byTs.filter((r) => r.key !== noteResolved.key);
    scoreResolved = false;
  }
}
const noteCov = phrasedCoverage(qtokens, noteResolved ? [noteResolved] : []);
const unitCov = unitTop ? distinctScore(qtokens, unitTop.text).matched / Math.max(1, qtokens.length) : 0;
const g = (k, d) => (process.env[k] !== undefined ? Number(process.env[k]) : d);
const NO_MEMORY_COV = g("MEM_NO_COV", 0.5);
const FLOOR = g("MEM_FLOOR", 6.0);
const NOTE_COV_MIN = g("MEM_NOTE_COV", 0.5);
const NOTE_SCORE_MIN = g("MEM_NOTE_SCORE", 6.0);
const UNIT_COV_MIN = g("MEM_UNIT_COV", 1.0);
const UNIT_SCORE_MIN = g("MEM_UNIT_SCORE", 8.0);
const UNIT_MARGIN_MIN = g("MEM_UNIT_MARGIN", 1.5);
const HIGH_MARGIN = g("MEM_HIGH_MARGIN", 1.8);
const gates = {
  NO_MEMORY_COV,
  FLOOR,
  NOTE_COV: NOTE_COV_MIN,
  NOTE_SCORE: NOTE_SCORE_MIN,
  UNIT_COV: UNIT_COV_MIN,
  UNIT_SCORE: UNIT_SCORE_MIN,
  UNIT_MARGIN: UNIT_MARGIN_MIN,
  HIGH_MARGIN,
};
const gateDefaults = { NO_MEMORY_COV: 0.5, FLOOR: 6.0, NOTE_COV: 0.5, NOTE_SCORE: 6.0, UNIT_COV: 1.0, UNIT_SCORE: 8.0, UNIT_MARGIN: 1.5, HIGH_MARGIN: 1.8 };
if (noteResolved && notesIdx && weightedNoteCov(qtokens, noteResolved.text, notesIdx.idf) < NOTE_COV_MIN) {
  const alt = topNotes.find((n) => n.key !== noteResolved.key && weightedNoteCov(qtokens, n.text, notesIdx.idf) >= NOTE_COV_MIN && n.score >= NOTE_SCORE_MIN);
  if (alt) {
    noteResolved = alt;
    noteSuperseded = null;
    scoreResolved = true;
  }
}
const nextFamilyNote = topNotes.find((r) => r.key !== noteResolved.key && familyKey(r) !== familyKey(noteResolved));
let noteDecisive = true;
if (nextFamilyNote && noteResolved.score === nextFamilyNote.score) {
  const tied = topNotes.filter((r) => r.score === noteResolved.score);
  const newestTs = Math.max(...tied.map((r) => new Date(r.source_ts || 0).getTime()));
  const newestTied = tied.filter((r) => new Date(r.source_ts || 0).getTime() === newestTs);
  const bestKind = Math.max(...newestTied.map((r) => KIND_RANK[r.source_kind] ?? 0));
  const kindTied = newestTied.filter((r) => (KIND_RANK[r.source_kind] ?? 0) === bestKind);
  if (kindTied.length === 1) {
    noteResolved = kindTied[0];
  } else {
    noteDecisive = false;
  }
}
const noteCovR = noteResolved && notesIdx ? weightedNoteCov(qtokens, noteResolved.text, notesIdx.idf) : 0;
const maxCov = Math.max(noteCovR, unitCov);
const nonDefaultGates = Object.keys(gateDefaults).some((k) => gates[k] !== gateDefaults[k]);
const famRelevant = noteResolved && qtokens.length
  ? qtokens.filter((t) => (noteResolved.text || "").toLowerCase().includes(t)).length >= Math.max(2, Math.ceil(qtokens.length / 2))
  : false;
const hasRecencyAnswer = noteSuperseded && noteSuperseded.length > 0 && famRelevant && noteResolved.score >= NOTE_SCORE_MIN;

if ((maxCov < NO_MEMORY_COV && !hasRecencyAnswer) || (noteTop ? noteTop.score : 0) < FLOOR && (unitTop ? unitTop.score : 0) < FLOOR && !hasRecencyAnswer) {
  verdict = "no-memory";
  confidence = "none";
  reason = `no memory above the answer threshold (max_cov=${maxCov.toFixed(2)}, floor=${FLOOR})`;
} else {
  const noteWinner = noteResolved && noteDecisive && CURATED_KINDS.has(noteResolved.source_kind) && (noteCovR >= NOTE_COV_MIN || (famRelevant && noteSuperseded)) && noteResolved.score >= NOTE_SCORE_MIN;
  const unitWinner = unitTop && unitCov >= UNIT_COV_MIN && unitTop.score >= UNIT_SCORE_MIN && !isBoilerplateUnit(unitTop) && unitMargin >= UNIT_MARGIN_MIN;
  if (noteWinner) {
    const nextFamily = topNotes.find((r) => r.key !== noteResolved.key && familyKey(r) !== familyKey(noteResolved));
    const margin = nextFamily ? noteResolved.score / nextFamily.score : Infinity;
    const high = margin >= HIGH_MARGIN && !noteSuperseded;
    answer = {
      text: noteResolved.text,
      layer: "note",
      key: noteResolved.key,
      cls: noteResolved.cls,
      source_kind: noteResolved.source_kind,
      source_ts: noteResolved.source_ts,
      score: Number(noteResolved.score.toFixed(2)),
      margin: Number(Math.min(margin, 99).toFixed(2)),
      superseded: noteSuperseded && noteSuperseded.length ? noteSuperseded.map((s) => ({ text: s.text, source_ts: s.source_ts })) : null,
    };
    verdict = "answered";
    confidence = high ? "high" : "medium";
    reason = `notes layer ${noteResolved.cls} answer, margin ${Number(Math.min(margin, 99).toFixed(2))}x${noteSuperseded ? ", recency-resolved (superseded a stale fact)" : ""}`;
    if (hasMultiIntent) {
      const second = topNotes.find((r) => r.key !== noteResolved.key && familyKey(r) !== familyKey(noteResolved) && distinctScore(qtokens, r.text).matched >= 2);
      if (second) related.push({ text: second.text, cls: second.cls, source_ts: second.source_ts });
    }
    scoreResolved = !noteSuperseded;
  } else if (unitWinner) {
    answer = {
      text: snippet(unitTop.text, qtokens),
      layer: "unit",
      key: unitTop.key,
      cls: null,
      source_kind: unitTop.kind,
      source_ts: unitTop.ts,
      session: unitTop.session,
      score: Number(unitTop.score.toFixed(2)),
      margin: null,
    };
    verdict = "answered";
    confidence = "medium";
    reason = unitTop.kind === "capture" ? "units layer answer (agent capture), confidence capped medium" : "units layer answer (user's own words), confidence capped medium";
  } else {
    verdict = "candidates";
    confidence = "low";
    reason = `no decisive answer: note_cov=${noteCovR.toFixed(2)} unit_cov=${unitCov.toFixed(2)}`;
  }
}

const LIVE_UNITS = snapshotRun === "live";
const staleLayer = LIVE_UNITS ? false : notesRun && snapshotRun ? runTime(notesRun) < runTime(snapshotRun) : false;
const recalled = topNotes.map((n) => ({
  key: n.key,
  cls: n.cls,
  score: Number(n.score.toFixed(2)),
  ...(n.source_kind !== undefined ? { source_kind: n.source_kind } : {}),
  ...(n.source_ts !== undefined ? { source_ts: n.source_ts } : {}),
}));

const out = {
  query,
  verdict,
  confidence,
  reason,
  gates,
  non_default_gates: nonDefaultGates,
  answer,
  recalled,
  related,
  signals: {
    top_note_score: noteTop ? Number(noteTop.score.toFixed(2)) : null,
    top_unit_score: unitTop ? Number(unitTop.score.toFixed(2)) : null,
    unit_margin: unitTop ? Number((unitMargin || 0).toFixed(2)) : null,
    note_token_coverage: Number(noteCov.toFixed(2)),
    unit_token_coverage: Number(unitCov.toFixed(2)),
    max_token_coverage: Number(Math.max(noteCov, unitCov).toFixed(2)),
    notes_run_ts: notesRun,
    snapshot_run_ts: snapshotRun,
    live_units: LIVE_UNITS,
    stale_layer: staleLayer,
    recency_resolved: noteSuperseded && noteSuperseded.length > 0,
  },
  counts: { units: userUnits.length, notes: notes.length },
  skills: {
    status: skillsIndex ? (skillsFreshness === "fresh" ? "served" : skillsFreshness) : "not-indexed",
    root: skillsIndex ? skillsIndex.root : skillsRoot,
    head: skillsIndex && skillsIndex.repo ? skillsIndex.repo.head : null,
    dirty: skillsIndex && skillsIndex.repo ? !!skillsIndex.repo.dirty : null,
    hits: BRIEF
      ? skillsHits.map((h) => ({ id: h.id, score: h.score, section: h.section, status: h.status }))
      : skillsHits,
  },
  units: BRIEF ? undefined : topUnits,
  notes: BRIEF ? topNotes.map((n) => ({ key: n.key, cls: n.cls, score: Number(n.score.toFixed(2)), ...(verdict === "answered" ? { text: n.text } : {}) })) : topNotes,
};
for (const k of Object.keys(out)) if (out[k] === undefined) delete out[k];
appendRetrieval(out, {
  storeRoot: STORE_ROOT,
  source: LOG_SOURCE,
  session: EXCLUDE_SESSION || null,
  cwd: LOG_CWD,
  fact: LOG_FACT,
  latencyMs: Date.now() - T0,
  k: K,
  noLog: NO_LOG,
});
console.log(JSON.stringify(out, null, 2));
