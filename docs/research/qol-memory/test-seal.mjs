#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { cpSync, mkdirSync, rmSync, readFileSync, writeFileSync, appendFileSync, statSync, unlinkSync } from "node:fs";
import { join, dirname } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";
import { gunzipSync } from "node:zlib";
import { qolMemoryStore } from "./lib/store-path.js";
import { seal, trySealedText, parseUnitsText, SEAL_SCHEMA } from "./lib/seal.js";
import { mergeUnits } from "./lib/merge.js";

const BASE = dirname(fileURLToPath(import.meta.url));
const SRC = join(qolMemoryStore(), "units.jsonl");
const ROOT = join(tmpdir(), "qol-memory-seal");
rmSync(ROOT, { recursive: true, force: true });

let failed = 0;

function sandbox(name) {
  const root = join(ROOT, name);
  rmSync(root, { recursive: true, force: true });
  mkdirSync(join(root, "notes"), { recursive: true });
  cpSync(SRC, join(root, "units.jsonl"));
  return root;
}

function synth(key, ts, text) {
  return { key, source: "test", session: "seal-test-session", cwd: "/tmp", kind: "user", ts: new Date(ts).toISOString(), text };
}

function rawReadItems(root) {
  return parseUnitsText(readFileSync(join(root, "units.jsonl"), "utf8"));
}

function sealedReadItems(root) {
  const raw = readFileSync(join(root, "units.jsonl"));
  return parseUnitsText(trySealedText(root, raw) || raw.toString("utf8"));
}

function poolItems(items) {
  const seen = new Set();
  return [...items]
    .sort((a, b) => new Date(a.ts || 0).getTime() - new Date(b.ts || 0).getTime())
    .filter((u) => {
      const norm = (u.text || "").toLowerCase().replace(/\s+/g, " ").trim();
      if (u.kind !== "user" || seen.has(norm)) return false;
      seen.add(norm);
      return true;
    });
}

function assert(cond, label) {
  if (!cond) {
    failed++;
    console.error(`FAIL ${label}`);
  } else {
    console.log(`pass ${label}`);
  }
}

function assertBytes(a, b, label) {
  assert(a.equals(b), label);
}

function assertSamePool(a, b, label) {
  assert(JSON.stringify(a) === JSON.stringify(b) && JSON.stringify(poolItems(a)) === JSON.stringify(poolItems(b)), label);
}

{
  const root = sandbox("t1");
  const raw = readFileSync(join(root, "units.jsonl"));
  const marker = seal(root);
  const blob = readFileSync(join(root, "units.seal.gz"));
  assert(marker.schema === SEAL_SCHEMA, "T1 marker carries the seal schema");
  assert(marker.prefix_len > 0 && marker.prefix_len < raw.length, "T1 seal cuts a real prefix with a tail left over");
  assert(raw[marker.prefix_len - 1] === 10, "T1 cut lands on a newline boundary");
  assertBytes(gunzipSync(blob), raw.subarray(0, marker.prefix_len), "T1 round-trip: gunzip(blob) byte-equals raw[0:prefix_len)");
  assert(marker.blob_len === blob.length, "T1 marker blob_len matches the on-disk blob size");
  let newlines = 0;
  for (let i = 0; i < marker.prefix_len; i++) if (raw[i] === 10) newlines++;
  assert(marker.sealed_units === newlines, "T1 sealed_units counts the prefix lines");
}

{
  const root = sandbox("t2");
  const marker = seal(root);
  const raw = readFileSync(join(root, "units.jsonl"));
  assert(marker.prefix_len < raw.length, "T2 a tail is present after sealing");
  assertSamePool(sealedReadItems(root), rawReadItems(root), "T2 sealed read deep-equals raw read with tail present");
}

{
  const root = sandbox("t3a");
  seal(root);
  unlinkSync(join(root, "units.seal.json"));
  assertSamePool(sealedReadItems(root), rawReadItems(root), "T3a blob present, marker missing -> full read, identical pool");
}

{
  const root = sandbox("t3b");
  seal(root);
  unlinkSync(join(root, "units.seal.gz"));
  assertSamePool(sealedReadItems(root), rawReadItems(root), "T3b marker present, blob missing -> full read, identical pool");
}

{
  const root = sandbox("t3c");
  seal(root);
  const markerPath = join(root, "units.seal.json");
  const marker = JSON.parse(readFileSync(markerPath, "utf8"));
  writeFileSync(markerPath, JSON.stringify({ ...marker, blob_len: marker.blob_len + 1 }));
  assertSamePool(sealedReadItems(root), rawReadItems(root), "T3c blob_len mismatch -> full read, identical pool");
}

{
  const root = sandbox("t3d");
  seal(root);
  const lines = rawReadItems(root);
  writeFileSync(join(root, "units.jsonl"), lines.slice(0, Math.floor(lines.length / 2)).map((u) => JSON.stringify(u)).join("\n") + "\n");
  const marker = JSON.parse(readFileSync(join(root, "units.seal.json"), "utf8"));
  assert(marker.prefix_len > statSync(join(root, "units.jsonl")).size, "T3d truncated file is smaller than the sealed prefix_len");
  assertSamePool(sealedReadItems(root), rawReadItems(root), "T3d prefix_len > file size -> full read, identical pool");
}

{
  const root = sandbox("t3e");
  seal(root);
  const now = Date.now();
  appendFileSync(
    join(root, "units.jsonl"),
    [synth("seal-t3e-1", now, "stale marker tail unit one alpha bravo"), synth("seal-t3e-2", now + 1000, "stale marker tail unit two charlie delta"), synth("seal-t3e-3", now + 2000, "stale marker tail unit three echo foxtrot")].join("\n") + "\n"
  );
  assertSamePool(sealedReadItems(root), rawReadItems(root), "T3e stale marker covering fewer units + grown tail -> identical pool");
}

{
  const root = sandbox("t4");
  seal(root);
  const blob1 = readFileSync(join(root, "units.seal.gz"));
  const marker1 = readFileSync(join(root, "units.seal.json"));
  seal(root);
  const blob2 = readFileSync(join(root, "units.seal.gz"));
  const marker2 = readFileSync(join(root, "units.seal.json"));
  assertBytes(blob1, blob2, "T4 re-seal produces a byte-identical gzip blob");
  assertBytes(marker1, marker2, "T4 re-seal produces a byte-identical marker");
}

{
  const root = sandbox("t5");
  seal(root);
  const before = sealedReadItems(root);
  appendFileSync(join(root, "units.jsonl"), '{"key":"seal-t5-partial","source":"test","session":"seal-test-session","cwd":"/tmp","kind":"user","ts":"2026-08-13T00:00:00.000Z","text":"mid-append write in flight, line not finished');
  assertSamePool(sealedReadItems(root), rawReadItems(root), "T5 partial tail line dropped by the parse rule, sealed read equals raw read");
  assert(JSON.stringify(sealedReadItems(root)) === JSON.stringify(before), "T5 sealed prefix unaffected, partial line dropped from both reads");
}

{
  const root = sandbox("t6");
  const ask = () =>
    spawnSync("node", [join(BASE, "ask.mjs"), "what did we decide about the qol-memory surface this week", "--store", root, "--brief"], { encoding: "utf8" });
  const cold = ask();
  assert(cold.status === 0, "T6 cold ask.mjs runs on the sandbox");
  const metaPath = join(root, "idx-pool.json.meta");
  const idxPath = join(root, "idx-pool.json");
  const metaMtime = statSync(metaPath).mtimeMs;
  const idxMtime = statSync(idxPath).mtimeMs;
  const metaFp = JSON.parse(readFileSync(metaPath, "utf8")).fp;
  seal(root);
  const warm = ask();
  assert(warm.status === 0, "T6 warm ask.mjs runs after sealing");
  assert(statSync(metaPath).mtimeMs === metaMtime && statSync(idxPath).mtimeMs === idxMtime, "T6 warm ask.mjs still hits the M0 cache (idx files untouched)");
  assert(cold.stdout === warm.stdout, "T6 sealed-store ask.mjs output byte-identical to raw-store output");
  assert(JSON.parse(readFileSync(metaPath, "utf8")).fp === metaFp, "T6 M0 fingerprint unchanged after sealing");
}

{
  const root = sandbox("t7");
  seal(root);
  const preCount = rawReadItems(root).length;
  writeFileSync(join(root, "units.seal.gz"), Buffer.alloc(16));
  const unitsPath = join(root, "units.jsonl");
  const now = Date.now();
  const snapUnits = [
    synth("seal-t7-1", now + 4000, "merge reseal unit one alpha bravo charlie delta"),
    synth("seal-t7-2", now + 5000, "merge reseal unit two echo foxtrot golf hotel"),
    synth("seal-t7-3", now + 6000, "merge reseal unit three india juliet kilo lima"),
  ];
  const { merged, added } = mergeUnits(root, snapUnits, { tail: 4096 });
  assert(added.length === 3 && merged.length === preCount + 3, "T7 merge appends the snapshot units");
  const raw = readFileSync(unitsPath);
  const marker = JSON.parse(readFileSync(join(root, "units.seal.json"), "utf8"));
  assert(marker.created === statSync(unitsPath).mtime.toISOString(), "T7 merge re-seals after the rewrite (fresh marker)");
  assertBytes(gunzipSync(readFileSync(join(root, "units.seal.gz"))), raw.subarray(0, marker.prefix_len), "T7 post-merge blob byte-equals the merged prefix");
  assertSamePool(sealedReadItems(root), rawReadItems(root), "T7 post-merge sealed read equals raw read");
}

console.log(failed ? `FAILED ${failed}` : "ALL PASS");
process.exit(failed ? 1 : 0);
