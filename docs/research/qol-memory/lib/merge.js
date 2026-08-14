import { readFileSync, writeFileSync, mkdirSync, existsSync, readdirSync, rmSync } from "node:fs";
import { join } from "node:path";
import { parseUnitsText, seal, unseal } from "./seal.js";

export function mergeUnits(storeRoot, snapUnits, opts = {}) {
  const unitsPath = join(storeRoot, "units.jsonl");
  let existing = [];
  if (existsSync(unitsPath)) existing = parseUnitsText(readFileSync(unitsPath, "utf8"));
  const seen = new Set(existing.map((u) => u.key));
  const added = snapUnits.filter((u) => !seen.has(u.key));
  const merged = existing.concat(added);
  mkdirSync(storeRoot, { recursive: true });
  for (const n of readdirSync(storeRoot).filter((n) => /^idx-.*\.(json|meta)$/.test(n))) {
    rmSync(join(storeRoot, n), { force: true });
  }
  unseal(storeRoot);
  writeFileSync(unitsPath, merged.map((u) => JSON.stringify(u)).join("\n") + (merged.length ? "\n" : ""));
  seal(storeRoot, opts);
  return { merged, added };
}

export function mergeStep(storeRoot, run) {
  const snapUnits = readFileSync(join(storeRoot, "snapshot", run, "snapshot.jsonl"), "utf8")
    .trim()
    .split("\n")
    .filter(Boolean)
    .map((l) => JSON.parse(l));
  const { merged, added } = mergeUnits(storeRoot, snapUnits);
  console.log(`[ingest] merge done (${merged.length} units in store, ${added.length} new from run ${run})`);
}
