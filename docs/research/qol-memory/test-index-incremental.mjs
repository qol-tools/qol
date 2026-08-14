#!/usr/bin/env node
import { cpSync, mkdirSync, rmSync, readFileSync, writeFileSync, readdirSync, utimesSync, statSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { qolMemoryStore } from "./lib/store-path.js";
import { buildIndex } from "./lib/retrieval.js";
import { buildOrLoad, saveIndex, persistedIndexPath } from "./lib/indexcache.js";

const SRC = join(qolMemoryStore(), "units.jsonl");
const TMP = join(tmpdir(), "qol-memory-idx-incremental");
rmSync(TMP, { recursive: true, force: true });
mkdirSync(join(TMP, "notes"), { recursive: true });
mkdirSync(join(TMP, "snapshot"), { recursive: true });
cpSync(SRC, join(TMP, "units.jsonl"));
const unitsPath = join(TMP, "units.jsonl");

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

function readUnits() {
  const text = readFileSync(unitsPath, "utf8").trim();
  return text ? text.split("\n").filter(Boolean).map((l) => JSON.parse(l)) : [];
}

function writeUnits(items) {
  writeFileSync(unitsPath, items.map((u) => JSON.stringify(u)).join("\n") + (items.length ? "\n" : ""));
}

function poolItems() {
  return dedupeUserUnits(readUnits().filter((u) => u.kind === "user"));
}

function synth(key, ts, text) {
  return { key, source: "test", session: "test-session-incremental", cwd: "/tmp", kind: "user", ts, text };
}

function normIndex(idx) {
  const sorted = (m) => [...m.entries()].sort((a, b) => (a[0] < b[0] ? -1 : 1));
  const df = new Map();
  for (const d of idx.docs) for (const t of d.tf.keys()) df.set(t, (df.get(t) || 0) + 1);
  return {
    N: idx.N,
    avgdl: idx.avgdl,
    totalLength: idx.totalLength ?? idx.docs.reduce((s, d) => s + d.len, 0),
    docs: idx.docs.map((d) => ({ key: d.unit.key, len: d.len, tf: sorted(d.tf) })),
    idf: sorted(idx.idf),
    df: sorted(df),
  };
}

let failed = 0;
function assertDeepEqual(idxA, idxB, label) {
  const a = JSON.stringify(normIndex(idxA));
  const b = JSON.stringify(normIndex(idxB));
  if (a !== b) {
    failed++;
    console.error(`FAIL ${label}`);
    console.error("A", a.slice(0, 500));
    console.error("B", b.slice(0, 500));
  } else {
    console.log(`pass ${label}`);
  }
}

const metaOf = (layer) => JSON.parse(readFileSync(persistedIndexPath(TMP, layer) + ".meta", "utf8"));

const base = poolItems();
const coldIdx = buildOrLoad(TMP, "pool", base, unitsPath);
assertDeepEqual(coldIdx, buildIndex(base), "cold build deep-equals buildIndex");
const coldMeta = metaOf("pool");
if (!(coldMeta.fp && coldMeta.size && coldMeta.count === base.length && coldMeta.firstKey === base[0].key && coldMeta.lastKey === base[base.length - 1].key)) {
  failed++;
  console.error("FAIL cold meta carries O(1) prefix proof", JSON.stringify(coldMeta));
} else {
  console.log("pass cold meta carries O(1) prefix proof (size,count,firstKey,lastKey,fp)");
}

const now = Date.now();
const added = [
  synth("unit-inc-t1", new Date(now).toISOString(), "incremental cache test tail unit one with alpha bravo charlie"),
  synth("unit-inc-t2", new Date(now + 1000).toISOString(), "incremental cache test tail unit two with delta echo foxtrot"),
  synth("unit-inc-t3", new Date(now + 2000).toISOString(), "incremental cache test tail unit three with golf hotel india"),
  synth("unit-inc-t4", new Date(now + 3000).toISOString(), "incremental cache test tail unit four with juliet kilo lima"),
];
const baseLines = readUnits();
writeUnits(baseLines.concat(added));
const items2 = poolItems();
const mergedIdx = buildOrLoad(TMP, "pool", items2, unitsPath);
assertDeepEqual(mergedIdx, buildIndex(items2), "incremental merge deep-equals cold rebuild on appended store");
assertDeepEqual(buildOrLoad(TMP, "pool", items2, unitsPath), buildIndex(items2), "warm read after merge deep-equals cold rebuild");

const systemUnit = { key: "unit-inc-sys", source: "test", session: "test-session-incremental", cwd: "/tmp", kind: "compaction", ts: new Date(now + 4000).toISOString(), text: "compaction summary that must not enter the pool layer" };
writeUnits(readUnits().concat(systemUnit));
const items3 = poolItems();
const refreshedIdx = buildOrLoad(TMP, "pool", items3, unitsPath);
assertDeepEqual(refreshedIdx, buildIndex(items3), "filtered append refreshes meta and stays deep-equal");
if (metaOf("pool").size !== statSync(unitsPath).size) {
  failed++;
  console.error("FAIL meta size refreshed after filtered append");
} else {
  console.log("pass meta size refreshed after filtered append");
}
assertDeepEqual(buildOrLoad(TMP, "pool", items3, unitsPath), buildIndex(items3), "warm read after meta refresh deep-equals cold rebuild");

const tampered = readUnits();
tampered[100] = { ...tampered[100], text: tampered[100].text + " tampered middle line changed byte length" };
writeUnits(tampered);
const items4 = poolItems();
const fallbackIdx = buildOrLoad(TMP, "pool", items4, unitsPath);
assertDeepEqual(fallbackIdx, buildIndex(items4), "middle edit falls back to full cold rebuild, deep-equal");

const truncated = readUnits().slice(0, -1);
writeUnits(truncated);
const items5 = poolItems();
const truncIdx = buildOrLoad(TMP, "pool", items5, unitsPath);
assertDeepEqual(truncIdx, buildIndex(items5), "tail truncation falls back to full cold rebuild, deep-equal");

const pruneBase = poolItems();
const pruneIdx = buildIndex(pruneBase);
const pruneNames = [];
for (let i = 0; i < 7; i++) {
  const layer = `pool-x-${String(i).padStart(8, "0")}`;
  saveIndex(TMP, layer, pruneIdx, pruneBase, unitsPath);
  const p = persistedIndexPath(TMP, layer);
  utimesSync(p, 1_700_000_000 + i, 1_700_000_000 + i);
  pruneNames.push(layer);
}
const survivors = readdirSync(TMP).filter((n) => /^idx-pool-x-.+\.json$/.test(n)).sort();
const expect = pruneNames.slice(2).map((l) => `idx-${l}.json`).sort();
if (JSON.stringify(survivors) !== JSON.stringify(expect)) {
  failed++;
  console.error("FAIL pool-x prune keeps newest 5 by mtime", survivors, "expected", expect);
} else {
  console.log(`pass pool-x prune keeps newest 5 by mtime (${survivors.length} survivors)`);
}

console.log(failed ? `FAILED ${failed}` : "ALL PASS");
process.exit(failed ? 1 : 0);
