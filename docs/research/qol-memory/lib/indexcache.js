import { createHash } from "node:crypto";
import { readFileSync, writeFileSync, mkdirSync, statSync, readdirSync, unlinkSync } from "node:fs";
import { join, dirname } from "node:path";
import { buildIndex, tokens } from "./retrieval.js";

export function layerFingerprint(items) {
  const h = createHash("sha1");
  for (const it of items) {
    h.update(it.key);
    h.update(String(it.text ? it.text.length : 0));
  }
  h.update(String(items.length));
  return h.digest("hex").slice(0, 16);
}

export function persistedIndexPath(storeRoot, layer) {
  return join(storeRoot, `idx-${layer}.json`);
}

function prefixProof(sourcePath, items) {
  let size;
  try {
    size = statSync(sourcePath).size;
  } catch {
    return null;
  }
  const count = items.length;
  const firstKey = count ? items[0].key : "";
  const lastKey = count ? items[count - 1].key : "";
  const h = createHash("sha1");
  h.update(String(size));
  h.update(":");
  h.update(String(count));
  h.update(":");
  h.update(firstKey);
  h.update(":");
  h.update(lastKey);
  return { size, count, firstKey, lastKey, fp: h.digest("hex").slice(0, 16) };
}

function readMeta(storeRoot, layer) {
  try {
    return JSON.parse(readFileSync(persistedIndexPath(storeRoot, layer) + ".meta", "utf8"));
  } catch {
    return null;
  }
}

function writeMeta(storeRoot, layer, meta) {
  writeFileSync(persistedIndexPath(storeRoot, layer) + ".meta", JSON.stringify(meta));
}

function pruneSessionCaches(storeRoot) {
  let entries;
  try {
    entries = readdirSync(storeRoot);
  } catch {
    return;
  }
  const session = entries
    .filter((n) => /^idx-pool-x-.+\.json$/.test(n))
    .map((n) => {
      let m = 0;
      try {
        m = statSync(join(storeRoot, n)).mtimeMs;
      } catch {}
      return { n, m };
    })
    .sort((a, b) => b.m - a.m);
  for (const { n } of session.slice(5)) {
    try {
      unlinkSync(join(storeRoot, n));
      unlinkSync(join(storeRoot, n + ".meta"));
    } catch {}
  }
}

export function saveIndex(storeRoot, layer, idx, items, sourcePath) {
  const p = persistedIndexPath(storeRoot, layer);
  mkdirSync(dirname(p), { recursive: true });
  const vocab = new Map();
  const df = new Map();
  let totalLength = 0;
  const docRows = items.map((unit, di) => {
    const u = idx.docs[di];
    const tf = [];
    for (const [t, f] of u.tf) {
      let id = vocab.get(t);
      if (id === undefined) {
        id = vocab.size;
        vocab.set(t, id);
      }
      tf.push(id, f);
      df.set(t, (df.get(t) || 0) + 1);
    }
    totalLength += u.len;
    return { k: unit.key, L: u.len, tf };
  });
  const { idf, N } = idx;
  const idfArr = new Array(idf.size);
  for (const [t, v] of idf) idfArr[vocab.get(t)] = v;
  const dfArr = new Array(vocab.size);
  for (const [t, n] of df) dfArr[vocab.get(t)] = n;
  const terms = new Array(vocab.size);
  for (const [t, id] of vocab) terms[id] = t;
  const avgdl = totalLength / Math.max(1, N);
  writeFileSync(p, JSON.stringify({ N, avgdl, totalLength, terms, idfArr, dfArr, rows: docRows }));
  const proof = sourcePath ? prefixProof(sourcePath, items) : null;
  writeMeta(
    storeRoot,
    layer,
    proof
      ? { fp: proof.fp, size: proof.size, count: proof.count, firstKey: proof.firstKey, lastKey: proof.lastKey, fingerprint: layerFingerprint(items) }
      : { fingerprint: layerFingerprint(items) }
  );
  pruneSessionCaches(storeRoot);
  return p;
}

export function loadIndex(p, storeRoot, layer, items) {
  const raw = JSON.parse(readFileSync(p, "utf8"));
  const docs = raw.rows.map((row) => {
    const tf = new Map();
    for (let j = 0; j < row.tf.length; j += 2) tf.set(raw.terms[row.tf[j]], row.tf[j + 1]);
    return { unit: { key: row.k, text: "" }, tf, len: row.L };
  });
  const idf = new Map();
  raw.idfArr.forEach((v, id) => idf.set(raw.terms[id], v));
  const df = new Map();
  if (Array.isArray(raw.dfArr)) {
    raw.dfArr.forEach((n, id) => df.set(raw.terms[id], n));
  } else {
    for (const d of docs) for (const t of d.tf.keys()) df.set(t, (df.get(t) || 0) + 1);
  }
  const totalLength = Number.isFinite(raw.totalLength) ? raw.totalLength : docs.reduce((s, d) => s + d.len, 0);
  return { docs, idf, df, N: raw.N, avgdl: raw.avgdl, totalLength };
}

function canMerge(proof, meta, items) {
  if (proof.size <= meta.size) return false;
  if (items.length < meta.count) return false;
  if (meta.count === 0) return true;
  return items[meta.count - 1].key === meta.lastKey && items[0].key === meta.firstKey;
}

function mergeTail(cached, tail) {
  const { docs, idf, df, totalLength } = cached;
  for (const u of tail) {
    const tf = new Map();
    for (const t of tokens(u.text)) tf.set(t, (tf.get(t) || 0) + 1);
    for (const t of tf.keys()) df.set(t, (df.get(t) || 0) + 1);
    docs.push({ unit: u, tf, len: u.text.length });
  }
  const mergedN = docs.length;
  const mergedTotal = totalLength + tail.reduce((s, u) => s + u.text.length, 0);
  for (const [t, n] of df) idf.set(t, Math.log(1 + (mergedN - n + 0.5) / (n + 0.5)));
  return { docs, idf, df, N: mergedN, avgdl: mergedTotal / Math.max(1, mergedN), totalLength: mergedTotal };
}

export function buildOrLoad(storeRoot, layer, items, sourcePath) {
  const p = persistedIndexPath(storeRoot, layer);
  const meta = readMeta(storeRoot, layer);
  const proof = sourcePath ? prefixProof(sourcePath, items) : null;
  if (meta) {
    try {
      if (proof && proof.fp === meta.fp) return loadIndex(p, storeRoot, layer, items);
      if (proof && meta.count !== undefined && meta.size !== undefined && canMerge(proof, meta, items)) {
        const cached = loadIndex(p, storeRoot, layer, items);
        if (items.length === meta.count) {
          if (meta.fingerprint !== undefined && meta.fingerprint === layerFingerprint(items)) {
            writeMeta(storeRoot, layer, { fp: proof.fp, size: proof.size, count: meta.count, firstKey: meta.firstKey, lastKey: meta.lastKey, fingerprint: meta.fingerprint });
            return cached;
          }
        } else {
          const merged = mergeTail(cached, items.slice(meta.count));
          saveIndex(storeRoot, layer, merged, items, sourcePath);
          return merged;
        }
      }
      if (meta.fingerprint !== undefined && meta.fingerprint === layerFingerprint(items)) return loadIndex(p, storeRoot, layer, items);
    } catch {}
  }
  const idx = buildIndex(items);
  saveIndex(storeRoot, layer, idx, items, sourcePath);
  return idx;
}
