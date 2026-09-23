import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const MODULE_DIR = dirname(fileURLToPath(import.meta.url));

export function qolMemoryStore() {
  if (process.env.QOL_MEMORY_STORE && process.env.QOL_MEMORY_STORE.length) {
    return process.env.QOL_MEMORY_STORE;
  }
  const xdg = process.env.XDG_DATA_HOME;
  const base = xdg && xdg.length ? xdg : join(homedir(), ".local", "share");
  return join(base, "qol-tray", "plugins", "qol-memory");
}

export function researchOutputRoot() {
  return resolve(MODULE_DIR, "..", "..", "..", "..", "reports", "qol-memory");
}

export function fixturesRoot() {
  return join(researchOutputRoot(), "fixtures");
}

export function researchRunDir(kind, run) {
  const fixture = join(fixturesRoot(), kind, run);
  return existsSync(fixture) ? fixture : join(researchOutputRoot(), kind, run);
}
