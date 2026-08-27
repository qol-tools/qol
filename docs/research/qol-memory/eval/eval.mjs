#!/usr/bin/env node
import { readdirSync, readFileSync, mkdirSync, writeFileSync, statSync } from "node:fs";
import { join, resolve, basename, dirname } from "node:path";
import { homedir } from "node:os";
import { qolMemoryStore } from "../lib/store-path.js";

const args = process.argv.slice(2);
const pick = (flag, def) => {
  const i = args.indexOf(flag);
  return i >= 0 && args[i + 1] ? args[i + 1] : def;
};
const BASE = dirname(dirname(new URL(import.meta.url).pathname));
const STORE_ROOT = resolve(pick("--store", qolMemoryStore()));
const SNAPSHOT_ROOT = resolve(pick("--snapshot-root", join(STORE_ROOT, "snapshot")));
const PINNED = JSON.parse(readFileSync(join(BASE, "eval", "questions.json"), "utf8")).run_pin || null;
const RUN_ID = pick("--run", null) || PINNED || latestRun(SNAPSHOT_ROOT);
if (PINNED && !pick("--run", null)) {
  try {
    if (!statSync(join(SNAPSHOT_ROOT, PINNED)).isDirectory()) throw new Error("missing");
  } catch {
    throw new Error(`pinned run ${PINNED} not found under ${SNAPSHOT_ROOT} - the pinned snapshot is machine-local (reports/ is gitignored); re-pin questions.json run_pin or pass --run`);
  }
}
const OUT_DIR = resolve(pick("--out", join(STORE_ROOT, "eval", new Date().toISOString().replace(/[:.]/g, "-"))));
const DENSE = pick("--dense", null);
const KINDS = (pick("--kinds", "user") || "user").split(",");
const RRF_K = Number(pick("--rrf-k", "200"));
const NOTES = pick("--notes", null);
const PRF_DOCS = Number(pick("--prf-docs", "10"));
const PRF_TERMS = Number(pick("--prf-terms", "4"));
const PRF_BOOST = Number(pick("--prf-boost", "2"));
const HELDOUT = pick("--heldout", null);

function latestRun(root) {
  const runs = readdirSync(root)
    .filter((n) => /^\d{4}-\d{2}-\d{2}T/.test(n))
    .map((n) => ({ n, d: statSync(join(root, n)).mtimeMs }))
    .sort((a, b) => b.d - a.d);
  if (!runs.length) throw new Error(`no snapshot runs under ${root}`);
  return runs[0].n;
}

import { tokens, normalize } from "../lib/retrieval.js";

export function buildIndex(units) {
  const df = new Map();
  const docs = units.map((u) => {
    const tf = new Map();
    for (const t of tokens(u.text)) tf.set(t, (tf.get(t) || 0) + 1);
    for (const t of tf.keys()) df.set(t, (df.get(t) || 0) + 1);
    return { unit: u, tf, len: u.text.length };
  });
  const N = docs.length;
  const avgdl = docs.reduce((s, d) => s + d.len, 0) / Math.max(1, N);
  const idf = new Map();
  for (const [t, n] of df) idf.set(t, Math.log(1 + (N - n + 0.5) / (n + 0.5)));
  return { docs, idf, N, avgdl };
}

function bm25Ranks(query, idx, weights) {
  const qt = tokens(query);
  if (!qt.length) return [];
  const scored = [];
  for (const d of idx.docs) {
    let s = 0;
    for (const t of qt) {
      const f = d.tf.get(t) || 0;
      if (!f) continue;
      const w = idx.idf.get(t) || 0;
      const boost = weights && weights[t] ? weights[t] : 1;
      s += (w * f * boost * 1.2) / (f + 1.2 * (1 - 0.75 + 0.75 * (d.len / idx.avgdl)));
    }
    scored.push([d.unit.key, s]);
  }
  scored.sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : 1));
  return scored;
}

function prfRanks(query, idx, opts) {
  const k = opts && opts.feedbackDocs ? opts.feedbackDocs : 10;
  const n = opts && opts.terms ? opts.terms : 4;
  const boost = opts && opts.boost ? opts.boost : 2;
  const qt = tokens(query);
  if (!qt.length) return [];
  const top = bm25Ranks(query, idx).slice(0, k);
  const termScore = new Map();
  for (const [key] of top) {
    const d = idx.docs.find((x) => x.unit.key === key);
    for (const [t, f] of d.tf) {
      if (qt.includes(t)) continue;
      const idf = idx.idf.get(t) || 0;
      if (idf <= 0) continue;
      termScore.set(t, (termScore.get(t) || 0) + (1 + Math.log(f)) * idf);
    }
  }
  const weights = {};
  for (const [t] of [...termScore.entries()].sort((a, b) => b[1] - a[1]).slice(0, n)) {
    weights[t] = boost;
  }
  return bm25Ranks([...qt, ...Object.keys(weights)].join(" "), idx, weights);
}

function rankMap(ranked) {
  return new Map(ranked.map(([k], i) => [k, i]));
}

function rrfRanks(bm25Ranked, denseRanked) {
  const rrf = new Map();
  const add = (ranked) => {
    for (const [k, i] of rankMap(ranked)) {
      rrf.set(k, (rrf.get(k) || 0) + 1 / (RRF_K + i + 1));
    }
  };
  add(bm25Ranked);
  if (denseRanked) add(denseRanked);
  return [...rrf.entries()].sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : 1));
}

const questionsDoc = JSON.parse(readFileSync(join(BASE, "eval", "questions.json"), "utf8"));
const questions = questionsDoc.questions;
const denseDump = DENSE ? JSON.parse(readFileSync(DENSE, "utf8")) : null;
const units = readFileSync(join(SNAPSHOT_ROOT, RUN_ID, "snapshot.jsonl"), "utf8")
  .trim()
  .split("\n")
  .map((l) => JSON.parse(l));
const userUnits = units.filter((u) => KINDS.includes(u.kind));
const idx = buildIndex(userUnits);
const notesUnits = NOTES
  ? readFileSync(NOTES, "utf8")
      .trim()
      .split("\n")
      .map((l) => JSON.parse(l))
  : null;
const notesIdx = notesUnits ? buildIndex(notesUnits) : null;
const notesById = new Map((notesUnits || []).map((u) => [u.key, u.text]));
const notesBySource = new Map();
for (const n of notesUnits || []) {
  if (!notesBySource.has(n.source_key)) notesBySource.set(n.source_key, []);
  notesBySource.get(n.source_key).push(n.key);
}

const methodKeys = ["bm25", "prf", ...(notesIdx ? ["twostage"] : []), ...(denseDump ? ["dense", "hybrid"] : [])];
const agg = Object.fromEntries(methodKeys.map((m) => [m, { hit1: 0, hit5: 0, hit10: 0, mrr: 0 }]));
let covered = 0;
const results = [];
for (const q of questions) {
  const useNotes = !!(q.note_key && notesIdx);
  const scorableKey = useNotes ? q.note_key : q.target_key;
  if (useNotes && !q.target_key && !q.covered) {
    q.covered = true;
  }
  const bm25Ranked = bm25Ranks(q.query, useNotes ? notesIdx : idx);
  const prfRanked = useNotes ? null : prfRanks(q.query, idx, { feedbackDocs: PRF_DOCS, terms: PRF_TERMS, boost: PRF_BOOST });
  const denseRanked = !useNotes && denseDump && denseDump[q.id] ? denseDump[q.id] : null;
  const perMethod = {
    bm25: bm25Ranked,
    ...(prfRanked ? { prf: prfRanked } : {}),
    ...(denseRanked ? { dense: denseRanked } : {}),
    ...(denseRanked ? { hybrid: rrfRanks(bm25Ranked, denseRanked) } : {}),
  };
  const scorable = !!scorableKey;
  if (scorable) covered++;
  const row = { id: q.id, category: q.category, covered: scorable, layer: useNotes ? "notes" : "units", ranks: {} };
  for (const m of Object.keys(perMethod)) {
    const rank = scorableKey ? perMethod[m].findIndex(([k]) => k === scorableKey) : -1;
    row.ranks[m] = rank;
    if (scorable && rank === 0) agg[m].hit1++;
    if (scorable && rank >= 0 && rank < 5) agg[m].hit5++;
    if (scorable && rank >= 0 && rank < 10) agg[m].hit10++;
    if (scorable && rank >= 0) agg[m].mrr += 1 / (rank + 1);
  }
  if (!useNotes && notesIdx && q.target_key) {
    let ts = -1;
    const bm25Rank = row.ranks.bm25;
    if (bm25Rank >= 0 && bm25Rank < 5) {
      ts = bm25Rank;
    } else {
      const srcHits = notesBySource.get(q.target_key) || [];
      if (srcHits.length) {
        const notesTop = bm25Ranks(q.query, notesIdx).slice(0, 5).map(([k]) => k);
        if (notesTop.some((k) => srcHits.includes(k))) ts = 5;
      }
    }
    row.ranks.twostage = ts;
    if (scorable && ts === 0) agg.twostage.hit1++;
    if (scorable && ts >= 0) agg.twostage.hit5++;
    if (scorable && ts >= 0) agg.twostage.hit10++;
    if (scorable && ts >= 0) agg.twostage.mrr += 1 / (ts + 1);
  }
  row.top5 = perMethod.hybrid ? perMethod.hybrid.slice(0, 5).map(([k, s]) => `${k}:${s.toFixed(3)}`) : perMethod.bm25.slice(0, 5).map(([k, s]) => `${k}:${s.toFixed(2)}`);
  results.push(row);
}

const stats = {
  questions: questions.length,
  covered,
  coverage_share: questions.length ? covered / questions.length : 0,
  units_indexed: userUnits.length,
  notes_indexed: notesUnits ? notesUnits.length : 0,
  rrf_k: RRF_K,
  prf: { feedbackDocs: PRF_DOCS, terms: PRF_TERMS, boost: PRF_BOOST },
  methods: {},
};
for (const m of methodKeys) {
  stats.methods[m] = {
    hit1: agg[m].hit1,
    hit5: agg[m].hit5,
    hit10: agg[m].hit10,
    mrr: covered ? agg[m].mrr / covered : 0,
    hit1_share: covered ? agg[m].hit1 / covered : 0,
    hit5_share: covered ? agg[m].hit5 / covered : 0,
    hit10_share: covered ? agg[m].hit10 / covered : 0,
  };
}
const layerAgg = { units: {}, notes: {} };
for (const row of results) {
  const la = layerAgg[row.layer];
  for (const m of methodKeys) {
    if (row.ranks[m] === undefined) continue;
    la[m] = la[m] || { n: 0, hit1: 0, hit5: 0, hit10: 0, mrr: 0 };
    la[m].n++;
    const r = row.ranks[m];
    const hit5 = m === "twostage" ? r >= 0 : r >= 0 && r < 5;
    const hit10 = m === "twostage" ? r >= 0 : r >= 0 && r < 10;
    if (r === 0) la[m].hit1++;
    if (hit5) la[m].hit5++;
    if (hit10) la[m].hit10++;
    if (r >= 0) la[m].mrr += 1 / (r + 1);
  }
}
stats.layers = {};
for (const layer of Object.keys(layerAgg)) {
  stats.layers[layer] = {};
  for (const [m, a] of Object.entries(layerAgg[layer])) {
    stats.layers[layer][m] = {
      n: a.n,
      hit1: a.hit1,
      hit5: a.hit5,
      hit10: a.hit10,
      mrr: a.n ? a.mrr / a.n : 0,
      hit1_share: a.n ? a.hit1 / a.n : 0,
      hit5_share: a.n ? a.hit5 / a.n : 0,
      hit10_share: a.n ? a.hit10 / a.n : 0,
    };
  }
}

if (HELDOUT) {
  if (!notesIdx) throw new Error("--heldout requires --notes");
  const heldout = JSON.parse(readFileSync(HELDOUT, "utf8")).questions || JSON.parse(readFileSync(HELDOUT, "utf8"));
  const hoRows = [];
  for (const q of heldout) {
    const ranked = bm25Ranks(q.query, notesIdx);
    const clean = (t) => t.toLowerCase().replace(/[^\p{L}\p{N}]+/gu, " ");
    const hit5 = ranked
      .slice(0, 5)
      .some(([k]) => clean(notesById.get(k) || "").includes(clean(q.fact)));
    hoRows.push({ id: q.id, query: q.query, fact: q.fact, hit5 });
  }
  stats.heldout = { n: hoRows.length, hit5: hoRows.filter((r) => r.hit5).length, rows: hoRows };
}

mkdirSync(OUT_DIR, { recursive: true });
const reportPath = join(OUT_DIR, "report.json");
writeFileSync(
  reportPath,
  JSON.stringify(
    {
      name: "qol-memory eval (BM25 / dense / hybrid)",
      schemaVersion: 2,
      started_at: new Date().toISOString(),
      finished_at: new Date().toISOString(),
      status: covered === 0 ? "fail" : agg.bm25.hit5 === 0 ? "fail" : agg.bm25.hit5 / covered >= 0.5 ? "pass" : "degraded",
      inputs: { snapshotRun: RUN_ID, questions: join(BASE, "eval", "questions.json"), denseDump: DENSE, kinds: KINDS, notes: NOTES, heldout: HELDOUT },
      artifacts: { report: reportPath },
      commands: ["node docs/research/qol-memory/eval/eval.mjs " + process.argv.slice(2).join(" ")],
      stats,
      results,
      next: [
        "Dense-embed the notes layer (probe extension); the notes index is BM25-only today",
        "Remaining units-layer misses are paraphrases (q05@5, q12@6 borderline, q09/q18/q19 deep): query rewriting",
      ],
    },
    null,
    2
  )
);
console.log(`run ${RUN_ID} | units indexed ${userUnits.length}${notesUnits ? ", notes " + notesUnits.length : ""} | covered ${covered}/${questions.length}`);
for (const layer of Object.keys(stats.layers)) {
  for (const [m, a] of Object.entries(stats.layers[layer])) {
    console.log(`  [${layer}] ${m.padEnd(7)} hit@1 ${a.hit1}/${a.n} (${(a.hit1_share * 100).toFixed(0)}%)  hit@5 ${a.hit5}/${a.n} (${(a.hit5_share * 100).toFixed(0)}%)  mrr ${a.mrr.toFixed(3)}`);
  }
}
for (const m of methodKeys) {
  const s = stats.methods[m];
  console.log(`  ${m.padEnd(7)} hit@1 ${s.hit1}/${covered} (${(s.hit1_share * 100).toFixed(0)}%)  hit@5 ${s.hit5}/${covered} (${(s.hit5_share * 100).toFixed(0)}%)  hit@10 ${s.hit10}/${covered} (${(s.hit10_share * 100).toFixed(0)}%)  mrr ${s.mrr.toFixed(3)}`);
}
for (const r of results) {
  if (r.covered && r.ranks.hybrid !== undefined && !(r.ranks.hybrid >= 0 && r.ranks.hybrid < 5)) {
    const q = questions.find((x) => x.id === r.id);
    console.log(`  MISS ${r.id} hybrid_rank=${r.ranks.hybrid} q="${q.query.slice(0, 55)}"`);
  }
}
if (stats.heldout) {
  console.log(`  [heldout] hit@5 ${stats.heldout.hit5}/${stats.heldout.n}`);
  for (const r of stats.heldout.rows) {
    if (!r.hit5) console.log(`    MISS ${r.id} :: ${r.query} (fact: ${r.fact})`);
  }
}
console.log(`report: ${reportPath}`);
