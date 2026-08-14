import { gzipSync, gunzipSync } from "node:zlib";
import { readFileSync, writeFileSync, mkdirSync, existsSync, statSync, renameSync, rmSync } from "node:fs";
import { join } from "node:path";

export const SEAL_SCHEMA = "qol-memory-seal-v1";
export const SEAL_TAIL_DEFAULT = 1048576;

export function sealPaths(storeRoot) {
  return {
    markerPath: join(storeRoot, "units.seal.json"),
    blobPath: join(storeRoot, "units.seal.gz"),
  };
}

export function parseUnitsText(text) {
  return text
    .split("\n")
    .filter(Boolean)
    .map((l) => {
      try {
        return JSON.parse(l);
      } catch {
        return null;
      }
    })
    .filter(Boolean);
}

function writeTmpRename(path, data) {
  const tmp = path + ".tmp";
  writeFileSync(tmp, data);
  renameSync(tmp, path);
}

export function seal(storeRoot, opts = {}) {
  const unitsPath = join(storeRoot, "units.jsonl");
  const tail = opts.tail === undefined ? SEAL_TAIL_DEFAULT : opts.tail;
  const raw = readFileSync(unitsPath);
  const cutAt = Math.max(0, raw.length - tail);
  const prefixLen = raw.subarray(0, cutAt).lastIndexOf(10) + 1;
  const prefix = raw.subarray(0, prefixLen);
  const blob = gzipSync(prefix, { level: 6 });
  let sealedUnits = 0;
  for (let i = 0; i < prefix.length; i++) if (prefix[i] === 10) sealedUnits++;
  const { markerPath, blobPath } = sealPaths(storeRoot);
  mkdirSync(storeRoot, { recursive: true });
  writeTmpRename(blobPath, blob);
  const marker = {
    schema: SEAL_SCHEMA,
    prefix_len: prefixLen,
    blob: "units.seal.gz",
    blob_len: blob.length,
    sealed_units: sealedUnits,
    created: statSync(unitsPath).mtime.toISOString(),
  };
  writeTmpRename(markerPath, JSON.stringify(marker, null, 2) + "\n");
  return marker;
}

export function unseal(storeRoot) {
  const { markerPath, blobPath } = sealPaths(storeRoot);
  rmSync(markerPath, { force: true });
  rmSync(blobPath, { force: true });
}

export function trySealedText(storeRoot, raw) {
  const { markerPath, blobPath } = sealPaths(storeRoot);
  try {
    if (!existsSync(markerPath) || !existsSync(blobPath)) return null;
    const marker = JSON.parse(readFileSync(markerPath, "utf8"));
    if (!marker || marker.schema !== SEAL_SCHEMA) return null;
    if (!Number.isInteger(marker.prefix_len) || marker.prefix_len < 0 || marker.prefix_len > raw.length) return null;
    if (!Number.isInteger(marker.blob_len) || statSync(blobPath).size !== marker.blob_len) return null;
    const prefix = gunzipSync(readFileSync(blobPath));
    if (prefix.length !== marker.prefix_len) return null;
    return Buffer.concat([prefix, raw.subarray(marker.prefix_len)]).toString("utf8");
  } catch {
    return null;
  }
}
