#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { parseUnitsText } from "./lib/seal.js";

const BASE = dirname(fileURLToPath(import.meta.url));
const SANDBOX = join(tmpdir(), "qol-memory-cadence-" + createHash("sha256").update(String(process.pid) + Date.now()).digest("hex").slice(0, 8));
const STORE = join(SANDBOX, "store");
const NOTES = join(STORE, "notes");
const POOL_RUN = "2026-08-12T00:00:00.000Z";
const SESS_A = "cad-sess-a";
const SESS_B = "cad-sess-b";
const SESS_C = "cad-sess-c";
const COMP_A2 = "## Key Decisions\n- The release naming convention is set: vMAJOR.MINOR-PATCH for feature releases.\n## Progress\n- release doc merged into the guide.";
const COMP_B = "## Key Decisions\n- The m4a1 anchoring fix is the press-fit spacer, not the glued shim.\n## Progress\n- cadence fixture built.";
const COMP_C = "## Key Decisions\n- The teal falcon marker stays in the cadence fixture.\n## Progress\n- nothing else.";
const COMP_C2 = "## Key Decisions\n- The teal falcon marker stays and the swallow lane is frozen.\n## Progress\n- nothing else.";

let failed = 0;
function check(cond, label) {
  if (!cond) {
    failed++;
    console.error(`FAIL ${label}`);
  } else {
    console.log(`pass ${label}`);
  }
}

function env() {
  return { ...process.env, QOL_MEMORY_STORE: STORE, QOL_MEMORY_MODEL_DISABLE: "1" };
}

function run(args, e = env()) {
  const t = Date.now();
  const r = spawnSync("node", [join(BASE, "decisions.mjs"), ...args], { encoding: "utf8", timeout: 120000, maxBuffer: 64 * 1024 * 1024, env: e });
  return { status: r.status, stdout: r.stdout || "", stderr: r.stderr || "", ms: Date.now() - t };
}

function runs() {
  return readdirSync(NOTES).filter((n) => /^\d{4}-\d{2}-\d{2}T/.test(n)).sort().reverse();
}

function latestRun() {
  return runs()[0];
}

function readNotes(runName) {
  return parseUnitsText(readFileSync(join(NOTES, runName, "notes.jsonl"), "utf8"));
}

function seedPool() {
  const pool = [
    { key: "p-dec-a", cls: "decision", text: "old decision text for a | tags: cad", tags: "cad", session: SESS_A, source_key: "k-a", source_ts: "2026-08-11T00:00:00.000Z", source_kind: "decision-deter" },
    { key: "p-dec-b", cls: "decision", text: "old decision text for b | tags: cad", tags: "cad", session: SESS_B, source_key: "k-b", source_ts: "2026-08-11T00:00:00.000Z", source_kind: "decision-deter" },
    { key: "p-path", cls: "path", text: "path notes.mjs | the transcript layer of the notes pipeline", session: SESS_A, source_key: "k-a", source_ts: "2026-08-11T00:00:00.000Z", source_kind: "user" },
  ];
  mkdirSync(join(NOTES, POOL_RUN), { recursive: true });
  writeFileSync(join(NOTES, POOL_RUN, "notes.jsonl"), pool.map((n) => JSON.stringify(n)).join("\n") + "\n");
  writeFileSync(join(NOTES, POOL_RUN, "report.json"), JSON.stringify({ stats: {} }));
}

function writeUnits(extra) {
  const units = [
    { key: "c-a1", source: "pi", session: SESS_A, kind: "compaction", ts: "2026-08-13T09:00:00.000Z", text: "## Key Decisions\n- The release naming convention is set: vMAJOR.MINOR-PATCH.\n## Progress\n- naming doc merged." },
    { key: "c-a2", source: "pi", session: SESS_A, kind: "compaction", ts: "2026-08-13T10:00:00.000Z", text: COMP_A2 },
    { key: "c-b1", source: "pi", session: SESS_B, kind: "compaction", ts: "2026-08-13T09:30:00.000Z", text: COMP_B },
    { key: "c-c1", source: "pi", session: SESS_C, kind: "compaction", ts: "2026-08-13T11:00:00.000Z", text: COMP_C },
    { key: "u1", source: "pi", session: SESS_A, kind: "user", ts: "2026-08-13T08:00:00.000Z", text: "we are setting up the release naming" },
  ];
  if (extra) units.push(...extra);
  writeFileSync(join(STORE, "units.jsonl"), units.map((u) => JSON.stringify(u)).join("\n") + "\n");
}

const started = Date.now();
process.on("exit", () => {
  if (process.env.QOL_MEMORY_E2E_KEEP === "1") return;
  try {
    rmSync(SANDBOX, { recursive: true, force: true });
  } catch {}
});

mkdirSync(STORE, { recursive: true });
seedPool();
writeUnits();

const runNames0 = runs();
check(runNames0.length === 1 && runNames0[0] === POOL_RUN, "C0 sandbox seeded with one pool run");

const a = run(["--live", "--session", SESS_A]);
console.log(a.stdout);
check(a.status === 0, "C1 live --session exits 0");
check(a.stdout.includes("mode live"), "C1 output line carries mode live");
check(/sessions changed 1/.test(a.stdout), "C1 sessions changed 1");
check(/decisions added: 1 \(carried 2\)/.test(a.stdout), "C1 added 1 carried 2 (pool decisions carried)");
const runsA = runs();
check(runsA.length === 2 && runsA[0] !== POOL_RUN, "C1 atomic new run dir created");
const notesA = readNotes(runsA[0]);
check(notesA.length === 4, `C1 newest run holds full pool + addition (${notesA.length} notes)`);
check(notesA.some((n) => n.key === "p-path"), "C1 transcript-class pool note carried");
check(notesA.some((n) => n.key === "p-dec-a"), "C1 prior decision note carried");
const addedA = notesA.filter((n) => n.cls === "decision" && n.session === SESS_A && n.source_ts === "2026-08-13T10:00:00.000Z");
check(addedA.length === 1 && addedA[0].text.includes("vMAJOR.MINOR-PATCH for feature releases"), "C1 decision distilled from newest compaction");
check(JSON.stringify(addedA[0].supersedes) === JSON.stringify(["c-a1"]), "C1 supersedes carries the prior compaction key");
const reportA = JSON.parse(readFileSync(join(NOTES, runsA[0], "report.json"), "utf8"));
check(reportA.inputs.mode === "live" && reportA.inputs.session === SESS_A && reportA.stats.decisions.added === 1, "C1 live report.json fields correct");
check(!readdirSync(NOTES).some((n) => n.startsWith(".tmp-")), "C1 no .tmp residue after a completed run");
check(!existsSync(join(STORE, ".distill.lock")), "C1 lock released after the run");

const b = run(["--live", "--session", SESS_A]);
console.log(b.stdout);
check(b.status === 0, "C2 second run exits 0");
check(b.stdout.includes("nothing added"), "C2 no-write-when-nothing-added line");
check(/decisions added: 0/.test(b.stdout), "C2 added 0");
check(runs().length === 2, "C2 NO new run dir on unchanged session");
check(JSON.stringify(readNotes(runs()[0])) === JSON.stringify(notesA), "C2 newest run byte-identical (carry intact)");

const lockFresh = { pid: 424242, started_at: new Date().toISOString(), mode: "live-all" };
writeFileSync(join(STORE, ".distill.lock"), JSON.stringify(lockFresh) + "\n");
const c = run(["--live"]);
console.log(c.stdout);
check(c.status === 0, "C3 contended run exits 0");
check(c.stdout.includes("skipped") && c.stdout.includes("lock busy"), "C3 skip line on lock contention");
check(runs().length === 2, "C3 no run dir written while lock held");

const lockStale = { pid: 424243, started_at: new Date(Date.now() - 20 * 60 * 1000).toISOString(), mode: "live-all" };
writeFileSync(join(STORE, ".distill.lock"), JSON.stringify(lockStale) + "\n");
const d = run(["--live"]);
console.log(d.stdout);
check(d.status === 0, "C4 stale-lock run exits 0");
check(!d.stdout.includes("skipped"), "C4 stale lock stolen, no skip");
check(/sessions changed 2/.test(d.stdout), "C4 distill-all re-distills grown sessions b and c");
const runsD = runs();
check(runsD.length === 3, "C4 new run dir written after steal");
const notesD = readNotes(runsD[0]);
check(notesD.some((n) => n.session === SESS_B && n.text.includes("press-fit spacer")), "C4 session b decision distilled");
check(notesD.some((n) => n.session === SESS_C && n.text.includes("teal falcon")), "C4 session c decision distilled");
check(notesD.some((n) => n.session === SESS_A && n.text.includes("vMAJOR.MINOR-PATCH")), "C4 session a carried note intact");
check(!existsSync(join(STORE, ".distill.lock")), "C4 lock released after steal-and-run");

const preKillNewest = JSON.stringify(readNotes(runsD[0]));
mkdirSync(join(NOTES, ".tmp-424244-2026-08-13T00:00:00.000Z"), { recursive: true });
writeFileSync(join(NOTES, ".tmp-424244-2026-08-13T00:00:00.000Z", "notes.jsonl"), "{\"partial\":true}\n");
const lockStale2 = { pid: 424244, started_at: new Date(Date.now() - 20 * 60 * 1000).toISOString(), mode: "live-all" };
writeFileSync(join(STORE, ".distill.lock"), JSON.stringify(lockStale2) + "\n");
writeUnits([{ key: "c-c2", source: "pi", session: SESS_C, kind: "compaction", ts: "2026-08-13T12:00:00.000Z", text: COMP_C2 }]);
const e = run(["--live"]);
console.log(e.stdout);
check(e.status === 0, "C5 heal run exits 0");
check(readdirSync(NOTES).some((n) => n === ".tmp-424244-2026-08-13T00:00:00.000Z"), "C5 killed-run .tmp dir still present");
check(JSON.stringify(readNotes(runsD[0])) === preKillNewest, "C5 pre-kill newest run untouched");
const runsE = runs();
check(runsE.length === 4, "C5 next run heals with a new run dir");
check(readNotes(runsE[0]).some((n) => n.session === SESS_C && n.text.includes("swallow lane is frozen")), "C5 heal run distilled the newest compaction");
check(readNotes(runsE[0]).length === 7, `C5 healed run holds the full pool + all additions (${readNotes(runsE[0]).length} notes)`);
check(!existsSync(join(STORE, ".distill.lock")), "C5 lock released after heal run");

const f = spawnSync("node", [join(BASE, "test-e2e.mjs")], { encoding: "utf8", timeout: 600000, maxBuffer: 64 * 1024 * 1024 });
console.log(f.stdout.slice(-400));
check(f.status === 0 && f.stdout.includes("ALL PASS"), "C6 test-e2e.mjs still ALL PASS after the cadence changes");

console.log(failed ? `FAILED ${failed}` : "ALL PASS");
console.log(`cadence runtime ${((Date.now() - started) / 1000).toFixed(1)}s`);
process.exit(failed ? 1 : 0);
