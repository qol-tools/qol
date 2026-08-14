import { readFileSync } from "node:fs";
import { join } from "node:path";
import { homedir } from "node:os";
import { buildIndex, bm25Ranks } from "./lib/retrieval.js";
import { buildOrLoad } from "./lib/indexcache.js";

const STORE = homedir() + "/.local/share/qol-tray/plugins/qol-memory";
const RUN = process.env.REPLAY_RUN || "2026-08-10T21-38-02-273Z";
const SESSION_A = process.env.REPLAY_SESSION || "019fec67be4a5f2e";
const ROOT = new URL(".", import.meta.url).pathname;
const questions = JSON.parse(readFileSync(join(ROOT, "eval", "questions.json"), "utf8")).questions;
const heldout = JSON.parse(readFileSync(join(ROOT, "eval", "heldout.json"), "utf8")).questions;
const allQ = [...questions, ...heldout.map((q) => ({ ...q, source: "heldout" }))];

const units = readFileSync(join(STORE, "snapshot", RUN, "snapshot.jsonl"), "utf8")
  .trim().split("\n").map(JSON.parse);
const userUnits = units.filter((u) => u.kind === "user");
const byKey = new Map(userUnits.map((u) => [u.key, u]));
const sessionUnits = userUnits.filter((u) => u.session === SESSION_A);
if (!sessionUnits.length) { console.error(`session ${SESSION_A} not found`); process.exit(1); }

const aStart = +new Date(sessionUnits[0].ts);
const aEnd = +new Date(sessionUnits[sessionUnits.length - 1].ts);
const storeMax = Math.max(...userUnits.map((u) => +new Date(u.ts)).filter(Number.isFinite));

const scope = process.env.REPLAY_SCOPE || "other-chat";
let windowEnd = storeMax;
let newlyLanded;
if (scope === "other-chat") {
  newlyLanded = userUnits.filter((u) => u.session !== SESSION_A && +new Date(u.ts) > aEnd && +new Date(u.ts) <= windowEnd);
} else if (scope === "after-start") {
  newlyLanded = userUnits.filter((u) => +new Date(u.ts) > aStart && +new Date(u.ts) <= windowEnd);
} else {
  newlyLanded = userUnits;
}

const blockIdx = buildIndex(newlyLanded);
const surfaced = newlyLanded.map((u) => u.key);
const surfacedSet = new Set(surfaced);

function goldUnit(q) {
  if (!q.target_key) return null;
  const u = byKey.get(q.target_key);
  return u || null;
}

function goldInBlock(q) {
  const u = goldUnit(q);
  if (!u) return { yes: false, inA: false, other: false, resolved: false };
  const inBlock = surfacedSet.has(q.target_key);
  const isA = u.session === SESSION_A;
  return { yes: inBlock, inA: isA, other: !isA && inBlock, resolved: inBlock && !isA };
}

const rows = [];
for (const q of allQ) {
  const g = goldInBlock(q);
  let rank = -1;
  if (g.other) {
    const r = bm25Ranks(q.query, blockIdx, null, 20);
    rank = r.findIndex(([k]) => k === q.target_key);
  }
  const probe = { id: q.id, query: q.query, source: q.source || "q30", inBlock: g.yes, inA: g.inA, other: g.other, rank };
  if (g.resolved) probe.resolved_by_block = true;
  rows.push(probe);
}

const golds = rows.filter((r) => r.inBlock || r.inA);
const otherGold = rows.filter((r) => r.other);
const resOther = otherGold.filter((r) => r.resolved_by_block && r.rank >= 0 && r.rank < 5);
const inOwn = rows.filter((r) => r.inA && !r.other).length;

const line = (s, v) => console.log(`${s.padEnd(46)}${v}`);
console.log(`replay | run ${RUN} | session A ${SESSION_A} | scope ${scope}`);
console.log(`session A: ${sessionUnits.length} user units ${new Date(aStart).toISOString()} -> ${new Date(aEnd).toISOString()}`);
console.log(`store max ${new Date(storeMax).toISOString()} | other-chat user units in window: ${newlyLanded.length}`);
console.log("");
console.log("T2 relevance: resolve gold questions whose answer landed from OTHER sessions after A ended");
line("q with gold unit elsewhere (resolvable-by-block)", otherGold.length);
line("  of those, in surfaced block", otherGold.filter((r) => r.inBlock).length);
line("  hit@5 in block (bm25 over block)", resOther.length);
line("q with gold in A's own transcript", inOwn);
line("surfaced block size", surfaced.length);
console.log("");
console.log("T3 utility: would surfacing the block change ask.mjs verdict for these queries?");
line("answered-by-block only queries", otherGold.filter((r) => r.rank === 0).length);
console.log("");
const missed = otherGold.filter((r) => !resOther.some((x) => x.id === r.id));
if (missed.length) {
  console.log("gold landed elsewhere but NOT hit@5 in block:");
  for (const r of missed) console.log(`  ${r.id} ${r.query}${r.inBlock ? " (in block but low rank)" : " (NOT in block)"}`);
}
