#!/usr/bin/env node
import { readFileSync } from "node:fs";
import { join, resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { homedir } from "node:os";
import { buildIndex, bm25Ranks } from "./lib/retrieval.js";
import { buildOrLoad } from "./lib/indexcache.js";

const BASE = dirname(fileURLToPath(import.meta.url));
const STORE = process.env.QOL_MEMORY_STORE || join(homedir(), ".local", "share", "qol-tray", "plugins", "qol-memory");
const RUN = process.env.REPLAY_RUN || "2026-08-10T21-38-02-273Z";
const SESSION_A = process.env.REPLAY_SESSION || "";
const WATERMARK = process.env.REPLAY_WATERMARK || "";
const K = 5;

if (!SESSION_A) { console.error("REPLAY_SESSION required"); process.exit(1); }

const units = readFileSync(join(STORE, "snapshot", RUN, "snapshot.jsonl"), "utf8")
  .trim().split("\n").map(JSON.parse);
const userUnits = units.filter((u) => u.kind === "user" && (u.text || "").trim());
const sessionUnits = userUnits.filter((u) => u.session === SESSION_A);
if (!sessionUnits.length) { console.error(`session ${SESSION_A} not found`); process.exit(1); }

const watermark = WATERMARK ? +new Date(WATERMARK) : Math.max(...sessionUnits.map((u) => +new Date(u.ts)));
const tail = sessionUnits.filter((u) => +new Date(u.ts) <= watermark).slice(-5).map((u) => u.text);
const query = tail.join(" \n ");
const newlyLanded = userUnits.filter((u) => u.session !== SESSION_A && +new Date(u.ts) > watermark);
const blockIdx = buildIndex(newlyLanded);
const ranked = bm25Ranks(query, blockIdx, null, K);
const byKey = new Map(newlyLanded.map((u) => [u.key, u]));

const out = {
  session: SESSION_A,
  watermark: new Date(watermark).toISOString(),
  tail_units: tail.length,
  window_units: newlyLanded.length,
  window_bytes: newlyLanded.reduce((n, u) => n + (u.text || "").length, 0),
  top: ranked.map((r) => {
    const u = byKey.get(r.key);
    return { key: r.key, score: Math.round(r.score * 100) / 100, session: u ? u.session : "?", ts: u ? u.ts : "?", snippet: u ? (u.text || "").slice(0, 120).replace(/\n/g, " ") : "" };
  }),
};
console.log(JSON.stringify(out, null, 2));
