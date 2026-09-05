import assert from "node:assert/strict";
import test from "node:test";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { prepareModels, sha256 } from "./artifacts.mjs";

test("offline comparison rejects corrupted weights and accepts only the declared digest", async () => {
  const cache = mkdtempSync(join(tmpdir(), "qol-model-integrity-"));
  const path = join(cache, "revision", "model.safetensors");
  const registry = { fixture: { revision: "revision", files: { "model.safetensors": sha256("verified") } } };
  try {
    mkdirSync(join(cache, "revision"));
    writeFileSync(path, "wrong weights");
    await assert.rejects(prepareModels(registry, cache, true), /invalid model file/);
    assert.equal(readFileSync(path, "utf8"), "wrong weights");
    writeFileSync(path, "verified");
    assert.deepEqual(await prepareModels(registry, cache, true), { fixture: join(cache, "revision") });
  } finally {
    rmSync(cache, { recursive: true, force: true });
  }
});
