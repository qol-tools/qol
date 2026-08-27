#!/usr/bin/env node
import { existsSync, statSync } from "node:fs";
import { join, resolve } from "node:path";
import { qolMemoryStore } from "./lib/store-path.js";
import { seal, SEAL_TAIL_DEFAULT } from "./lib/seal.js";

const args = process.argv.slice(2);
const pick = (flag, def) => {
  const i = args.indexOf(flag);
  return i >= 0 && args[i + 1] ? args[i + 1] : def;
};
const STORE_ROOT = resolve(pick("--store", qolMemoryStore()));
const TAIL = Number(pick("--tail", String(SEAL_TAIL_DEFAULT)));

const unitsPath = join(STORE_ROOT, "units.jsonl");
if (!existsSync(unitsPath)) {
  console.error(`[seal] no units.jsonl under ${STORE_ROOT}`);
  process.exit(1);
}
const t = Date.now();
const marker = seal(STORE_ROOT, { tail: TAIL });
const size = statSync(unitsPath).size;
console.log(`[seal] ${STORE_ROOT}: prefix ${marker.prefix_len}/${size} bytes (${(100 * marker.prefix_len / size).toFixed(1)}%), ${marker.sealed_units} units -> ${marker.blob_len} bytes gzip, ${Date.now() - t}ms`);
