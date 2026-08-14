#!/usr/bin/env node
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readdirSync, readFileSync, writeFileSync, mkdirSync, renameSync } from "node:fs";
import { join, resolve, basename, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { qolMemoryStore } from "./lib/store-path.js";
import { trySealedText, parseUnitsText } from "./lib/seal.js";
import { acquireDistillLock } from "./lib/distill-lock.js";
import { redact } from "./lib/redact.js";

const BASE = dirname(fileURLToPath(import.meta.url));
const ARGS = process.argv.slice(2);
const pick = (flag, def) => {
  const i = ARGS.indexOf(flag);
  return i >= 0 && ARGS[i + 1] ? ARGS[i + 1] : def;
};
const FORCE_ALL = ARGS.includes("--force-all");
const LIVE = ARGS.includes("--live");
const SESSION_FILTER = pick("--session", null);
const forceSessions = new Set();
for (let i = 0; i < ARGS.length; i++) {
  if (ARGS[i] === "--force-session" && ARGS[i + 1]) forceSessions.add(ARGS[i + 1]);
}
const PINNED =
  pick("--snapshot-run", null) ||
  JSON.parse(readFileSync(join(BASE, "eval", "questions.json"), "utf8")).run_pin ||
  "2026-08-10T21-38-02-273Z";
const STORE_ROOT = resolve(pick("--store", qolMemoryStore()));
const SNAPSHOT_DIR = join(STORE_ROOT, "snapshot");
const OUT_DIR = join(STORE_ROOT, "notes");
const MODEL_DISABLE = process.env.QOL_MEMORY_MODEL_DISABLE === "1";
const MODEL = process.env.QOL_MEMORY_MODEL || "deepseek-v4-flash";
const PROVIDER = process.env.QOL_MEMORY_PROVIDER || "deepseek";
const THINKING = process.env.QOL_MEMORY_THINKING || "low";

const IDENT_STOP = new Set(["kcd2", "forge", "qol", "node", "git", "pi", "claude", "blender", "commit", "session", "sessions", "unit", "units", "note", "notes", "file", "files", "path", "paths", "model", "models", "flag", "flags", "config", "status", "count", "version", "key", "keys", "data", "repo", "report", "run", "runs", "ts", "id", "src", "the", "and", "with", "was", "were", "had", "has", "have", "this", "that", "from", "into", "onto", "over", "under", "for", "not", "all", "any", "but", "its", "it", "is", "are", "be", "been", "being", "will", "would", "can", "could", "should", "must", "may", "might", "than", "then", "there", "their", "they", "them", "what", "when", "where", "which", "who", "how", "why", "one", "two", "new", "old", "first", "last", "next", "also", "still", "after", "before", "because", "between", "during", "within", "without", "through", "across", "around", "back", "out", "up", "down", "off", "on", "at", "by", "to", "of", "in", "or", "an"]);

function distinctiveTerms(sessionUnits, corpusUnits) {
  const df = new Map();
  for (const u of corpusUnits) {
    const seen = new Set();
    for (const t of (u.text || "").toLowerCase().match(/[a-z0-9]+/g) || []) {
      if (IDENT_STOP.has(t) || /^\d+$/.test(t) || t.length > 24 || t.length < 4) continue;
      if (seen.has(t)) continue;
      seen.add(t);
      df.set(t, (df.get(t) || 0) + 1);
    }
  }
  const counts = new Map();
  for (const u of sessionUnits) {
    const seen = new Set();
    for (const t of (u.text || "").toLowerCase().match(/[a-z0-9]+/g) || []) {
      if (IDENT_STOP.has(t) || /^\d+$/.test(t) || t.length > 24 || t.length < 4) continue;
      if (seen.has(t)) continue;
      seen.add(t);
      counts.set(t, (counts.get(t) || 0) + 1);
    }
  }
  const N = corpusUnits.length || 1;
  const scored = [];
  for (const [t, tf] of counts) {
    const d = df.get(t) || 0;
    const idf = Math.log(1 + N / (1 + d));
    const identBoost = /[a-z]+[0-9]|[0-9]+[a-z]/.test(t) ? 3 : 1;
    scored.push([t, idf * Math.min(tf, 5) * identBoost]);
  }
  scored.sort((a, b) => b[1] - a[1]);
  const idents = scored.filter((x) => /[a-z]+[0-9]|[0-9]+[a-z]/.test(x[0])).slice(0, 6).map((x) => x[0]);
  const topics = scored.filter((x) => !/[a-z]+[0-9]|[0-9]+[a-z]/.test(x[0])).slice(0, 10).map((x) => x[0]);
  const family = new Set();
  const allTokens = new Set();
  for (const u of sessionUnits) {
    for (const t of (u.text || "").toLowerCase().match(/[a-z0-9]+/g) || []) {
      if (t.length >= 4) allTokens.add(t);
    }
  }
  for (const t of topics) family.add(t);
  for (const t of ["anchor", "anchoring", "anchored", "reanchor", "reanchored"].filter((t) => allTokens.has(t))) family.add(t);
  return [...idents, ...family].slice(0, 22);
}

function noteKey(cls, text) {
  return createHash("sha256").update([cls, text].join("|")).digest("hex").slice(0, 16);
}

function normalize(text) {
  return text
    .toLowerCase()
    .replace(/[`"'(),;:]/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

function section(text, name) {
  const re = new RegExp(`## ${name}[\\s\\S]*?(?=\\n## |$)`);
  const m = text.match(re);
  if (!m) return "";
  return m[0].replace(new RegExp(`^## ${name}`), "").trim();
}

function determinize(unit) {
  const decisions = section(unit.text, "Key Decisions");
  const progress = section(unit.text, "Progress");
  const constraints = section(unit.text, "Constraints & Preferences");
  const goal = section(unit.text, "Goal");
  const pick = (src, filterHeadings) => {
    if (!src) return "";
    const lines = src
      .split("\n")
      .map((l) => l.replace(/^[-*•]\s*/, "").trim())
      .filter((l) => l.length > 12 && (!filterHeadings || !/^(Goal|Next Steps|Critical Context|Constraints)/.test(l)));
    return lines.join(" ").slice(0, 240);
  };
  const out = [];
  const main = pick(decisions || progress, true);
  if (main) out.push(main);
  const pref = pick(constraints, false);
  if (pref && !out.includes(pref)) out.push(pref);
  const goalTxt = pick(goal, false);
  if (goalTxt && !out.includes(goalTxt)) out.push(goalTxt);
  return out;
}

function piAvailable() {
  if (MODEL_DISABLE) return false;
  const r = spawnSync("which", ["pi"], { encoding: "utf8" });
  return r.status === 0;
}

function llmResolve(compactions, tags) {
  const sorted = [...compactions].sort((a, b) => (a.ts || "").localeCompare(b.ts || ""));
  const parts = [];
  let budget = 60000;
  for (let i = sorted.length - 1; i >= 0; i--) {
    const u = sorted[i];
    const head = `--- compaction ${i + 1}/${sorted.length} (${(u.ts || "").slice(0, 19)}) ---\n`;
    const cap = Math.max(2000, Math.min(u.text.length, budget - parts.join("\n").length - head.length));
    if (cap <= 0) break;
    parts.unshift(head + u.text.slice(0, cap));
  }
  const seq = parts.join("\n\n");
  const newest = sorted[sorted.length - 1];
  const prefBlock = section(newest.text, "Constraints & Preferences");
  const goalBlock = section(newest.text, "Goal");
  const extraBlocks = [];
  if (prefBlock) extraBlocks.push(`--- Constraints & Preferences (settled preferences; distill durable ones as decisions) ---\n${prefBlock.slice(0, 4000)}`);
  if (goalBlock) extraBlocks.push(`--- Goal (distill only if still the settled final intent) ---\n${goalBlock.slice(0, 2000)}`);
  const extra = extraBlocks.length ? "\n\n" + extraBlocks.join("\n\n") : "";
  const prompt = `You are distilling a long working session into settled decisions.

Below is a sequence of compaction summaries from ONE session (each later summary supersedes the earlier ones). Extract ONLY the SETTLED final-state decisions - what was decided in the end, including things that changed course (e.g. "X was tried, then reverted, final fix is Y").

RULES:
- Extract only SETTLED facts/decisions. NEVER extract planned actions, pending steps, "next steps", or commands to run.
- Maximum 10 decisions. Merge near-identical decisions into one; never repeat the same decision with different wording.
- Each decision on its own line, plain text, 1-2 sentences, no markdown.
- Include the evidence chain when a decision changed (e.g. "July reanchor CCD lane was reverted because it caused a regression; final fix = ...").
- Prefer the LAST summary's state as the settled truth, unless an even later summary overrides it.
- THIS SESSION'S DISTINCTIVE TERMS (use them naturally inside decision lines where they belong, verbatim, so the decisions stay findable by these terms): ${tags}
- Also extract SETTLED constraints and preferences from the Constraints & Preferences section (e.g. "never directly on the main clone for feature work") and the Goal when it is still the settled final intent; they count toward the 10-decision cap.
- Output ONLY the decisions, one per line. No preamble, no code blocks.

${seq}
${extra}`;
  return new Promise((resolveP) => {
    const child = spawn("pi", ["-p", "--provider", PROVIDER, "--model", MODEL, "--thinking", THINKING, "--no-session", "--no-tools"], {
      stdio: ["pipe", "pipe", "pipe"],
    });
    let out = "";
    let err = "";
    child.stdout.on("data", (c) => {
      out += c;
    });
    child.stderr.on("data", (c) => {
      err += c;
    });
    const timer = setTimeout(() => child.kill("SIGKILL"), 120000);
    child.on("error", (e) => {
      clearTimeout(timer);
      resolveP({ ok: false, error: `spawn failed: ${e.message}` });
    });
    child.on("close", (code) => {
      clearTimeout(timer);
      if (code !== 0) {
        return resolveP({ ok: false, error: `pi exit ${code}: ${err.slice(0, 200)}` });
      }
      const lines = (out || "")
        .split("\n")
        .map((l) => l.trim().replace(/^\d{1,2}[.)]\s+/, ""))
        .filter((l) => l.length > 15 && !l.startsWith("#") && !l.startsWith("```") && !/^(DECISION|Decision|decision):?\s*$/i.test(l))
        .map((l) => l.replace(/^[-*•]?\s*(?:DECISION|Decision|decision):?\s*/i, "").trim());
      resolveP({ ok: lines.length > 0, decisions: lines, error: lines.length ? null : "empty output" });
    });
    child.stdin.write(prompt);
    child.stdin.end();
  });
}

const mode = LIVE ? (SESSION_FILTER ? "live-session" : "live-all") : "snapshot";
const lock = acquireDistillLock(STORE_ROOT, mode);
if (!lock) {
  console.log(`distill skipped: lock busy (mode ${mode})`);
  process.exit(0);
}
const fail = (msg) => {
  console.error(msg);
  lock.release();
  process.exit(1);
};
process.on("exit", () => lock.release());

try {
  const TS = new Date().toISOString();
  let notesRunDir = null;
  let existing;
  let existingKeys;
  let bySessionBaseline;
  let olderBySession;
  let compactions;
  if (LIVE) {
    const raw = readFileSync(join(STORE_ROOT, "units.jsonl"));
    const unitsText = trySealedText(STORE_ROOT, raw) || raw.toString("utf8");
    compactions = parseUnitsText(unitsText).filter((u) => u.kind === "compaction");
    const allNoteRuns = readdirSync(join(STORE_ROOT, "notes")).filter((n) => /^\d{4}-\d{2}-\d{2}T/.test(n)).sort().reverse();
    const poolRun = allNoteRuns[0] || null;
    existing = poolRun
      ? readFileSync(join(STORE_ROOT, "notes", poolRun, "notes.jsonl"), "utf8")
          .split("\n")
          .filter(Boolean)
          .map((l) => JSON.parse(l))
      : [];
    existingKeys = new Set(existing.map((n) => n.key));
    bySessionBaseline = new Map();
    for (const n of existing) {
      if (n.cls !== "decision") continue;
      if (!bySessionBaseline.has(n.session)) bySessionBaseline.set(n.session, []);
      bySessionBaseline.get(n.session).push(n);
    }
    olderBySession = new Map();
  } else {
    const notesRun = spawnSync("node", [join(BASE, "notes.mjs"), "--snapshot-run", PINNED, "--with-artifacts"], {
      encoding: "utf8",
      timeout: 120000,
      env: { ...process.env, QOL_MEMORY_DISTILL_LOCK_HELD: "1" },
    });
    if (notesRun.status !== 0) {
      fail(`notes.mjs failed: ${(notesRun.stderr || "").slice(0, 400)}`);
    }
    const runDirLine = (notesRun.stdout || "").match(/report: (.+)\/report\.json/);
    if (!runDirLine) {
      fail(`could not locate notes run dir from notes.mjs output: ${(notesRun.stdout || "").slice(0, 300)}`);
    }
    notesRunDir = runDirLine[1];
    existing = readFileSync(join(notesRunDir, "notes.jsonl"), "utf8")
      .split("\n")
      .filter(Boolean)
      .map((l) => JSON.parse(l));
    existingKeys = new Set(existing.map((n) => n.key));

    const priorByRun = [];
    const allNoteRuns = readdirSync(join(STORE_ROOT, "notes")).filter((n) => /^\d{4}-\d{2}-\d{2}T/.test(n)).sort().reverse();
    for (const runName of allNoteRuns) {
      if (join(STORE_ROOT, "notes", runName) === notesRunDir) continue;
      try {
        const prior = readFileSync(join(STORE_ROOT, "notes", runName, "notes.jsonl"), "utf8")
          .split("\n")
          .filter(Boolean)
          .map((l) => JSON.parse(l))
          .filter((n) => n.cls === "decision");
        if (prior.length) priorByRun.push({ runName, notes: prior });
      } catch {}
    }
    const baseline = priorByRun.length ? priorByRun[0].notes : [];
    bySessionBaseline = new Map();
    for (const n of baseline) {
      if (!bySessionBaseline.has(n.session)) bySessionBaseline.set(n.session, []);
      bySessionBaseline.get(n.session).push(n);
    }
    olderBySession = new Map();
    for (const { notes } of priorByRun.slice(1, 3)) {
      for (const n of notes) {
        if (!olderBySession.has(n.session)) olderBySession.set(n.session, []);
        olderBySession.get(n.session).push(n);
      }
    }

    const snapshot = readFileSync(join(SNAPSHOT_DIR, PINNED, "snapshot.jsonl"), "utf8")
      .split("\n")
      .filter(Boolean)
      .map((l) => JSON.parse(l));
    compactions = snapshot.filter((u) => u.kind === "compaction");
  }

  const bySession = new Map();
  for (const u of compactions) {
    if (SESSION_FILTER && u.session !== SESSION_FILTER) continue;
    if (!bySession.has(u.session)) bySession.set(u.session, []);
    bySession.get(u.session).push(u);
  }

  const started = Date.now();
  const added = [];
  const carried = [];
  const stats = { sessions: bySession.size, llm_calls: 0, deterministic: 0, pi_available: piAvailable() };
  const llmErrors = [];
  const pushCarried = (n) => {
    if (existingKeys.has(n.key)) return;
    if (n.tags && !(n.text || "").includes(" | tags:")) n.text = `${n.text} | tags: ${n.tags}`;
    existingKeys.add(n.key);
    carried.push(n);
    existing.push(n);
  };
  const pending = [];
  for (const [session, us] of bySession) {
    const sorted = [...us].sort((a, b) => (a.ts || "").localeCompare(b.ts || ""));
    const newest = sorted[sorted.length - 1];
    const baseNotes = bySessionBaseline.get(session) || [];
    const olderNotes = olderBySession.get(session) || [];
    const newestBaselineTs = baseNotes.reduce((m, n) => (n.source_ts > m ? n.source_ts : m), "");
    const forced = FORCE_ALL || [...forceSessions].some((f) => session.startsWith(f));
    if (!forced && baseNotes.length && newestBaselineTs && newest.ts <= newestBaselineTs) {
      for (const n of baseNotes) pushCarried(n);
      for (const n of olderNotes) pushCarried(n);
      continue;
    }
    const tags = distinctiveTerms(sorted, compactions).join(" ");
    const useLlm = sorted.length >= 2 && stats.pi_available;
    if (useLlm) stats.llm_calls++;
    pending.push({ session, sorted, newest, tags, useLlm, baseNotes, olderNotes });
  }
  const results = await Promise.all(
    pending.map((p) => (p.useLlm ? llmResolve(p.sorted, p.tags).then((res) => ({ ...p, res })) : Promise.resolve({ ...p, res: null })))
  );
  for (const p of results) {
    const { session, sorted, newest, tags, useLlm, baseNotes, olderNotes } = p;
    const pushNoteLocal = (text, sourceKind) => {
      const body = redact(text).slice(0, 240);
      const k = noteKey("decision", normalize(body));
      if (existingKeys.has(k)) return;
      existingKeys.add(k);
      added.push({
        key: k,
        cls: "decision",
        text: `${body} | tags: ${tags}`,
        tags,
        source_key: newest.key,
        source_ts: newest.ts,
        source_kind: sourceKind,
        session,
        supersedes: sorted.slice(0, -1).map((u) => u.key),
      });
    };
    if (useLlm && p.res.ok) {
      for (const d of p.res.decisions) pushNoteLocal(d, "decision");
      for (const det of determinize(newest)) {
        pushNoteLocal(det, "decision-deter");
        stats.deterministic++;
      }
    } else {
      if (useLlm) llmErrors.push({ session, error: p.res.error });
      for (const det of determinize(newest)) {
        pushNoteLocal(det, "decision-deter");
        stats.deterministic++;
      }
    }
    for (const n of [...baseNotes, ...olderNotes]) pushCarried(n);
  }
  const all = [...existing, ...added];
  const ms = Date.now() - started;

  if (LIVE) {
    const carriedCount = new Set(existing.filter((n) => n.cls === "decision").map((n) => n.key)).size;
    const byKind = {};
    for (const n of all) byKind[n.cls] = (byKind[n.cls] || 0) + 1;
    const line = `decisions added: ${added.length} (carried ${carriedCount}) | sessions changed ${new Set(added.map((n) => n.session)).size} | llm_calls ${stats.llm_calls} | deterministic ${stats.deterministic} | pi ${stats.pi_available ? "available" : "missing/disabled"} | mode live | ${ms}ms`;
    if (added.length === 0) {
      console.log(line);
      console.log("no run written: nothing added");
    } else {
      const tmpDir = join(OUT_DIR, `.tmp-${process.pid}-${TS}`);
      mkdirSync(tmpDir, { recursive: true });
      writeFileSync(join(tmpDir, "notes.jsonl"), all.map((n) => JSON.stringify(n)).join("\n") + "\n");
      const report = {
        name: "qol-memory notes (live distill)",
        schemaVersion: 2,
        started_at: new Date(started).toISOString(),
        finished_at: new Date().toISOString(),
        status: "pass",
        inputs: { mode: "live", session: SESSION_FILTER, compactions: compactions.length, pool: existing.length },
        artifacts: { notes: `${TS}/notes.jsonl`, report: `${TS}/report.json` },
        commands: [`node ${basename(BASE)}/decisions.mjs --live${SESSION_FILTER ? ` --session ${SESSION_FILTER}` : ""}`],
        stats: {
          notes: all.length,
          byClass: byKind,
          decisions: { added: added.length, carried: carriedCount, total: all.filter((n) => n.cls === "decision").length },
          decision_model: MODEL_DISABLE ? "disabled (deterministic)" : `${PROVIDER}/${MODEL} thinking=${THINKING}`,
          decision_llm_calls: stats.llm_calls,
          decision_deterministic: stats.deterministic,
          decision_pi_available: stats.pi_available,
          decision_errors: llmErrors.slice(0, 5),
        },
      };
      writeFileSync(join(tmpDir, "report.json"), JSON.stringify(report, null, 2));
      const runDir = join(OUT_DIR, TS);
      renameSync(tmpDir, runDir);
      console.log(line);
      if (llmErrors.length) console.log(`llm errors: ${llmErrors.length} (first: ${llmErrors[0].error})`);
      console.log(`notes run: ${runDir} (total notes ${all.length})`);
    }
  } else {
    writeFileSync(join(notesRunDir, "notes.jsonl"), all.map((n) => JSON.stringify(n)).join("\n") + "\n");
    const report = JSON.parse(readFileSync(join(notesRunDir, "report.json"), "utf8"));
    report.stats.decisions = { added: added.length, carried: carried.length, total: all.filter((n) => n.cls === "decision").length };
    report.stats.decision_model = MODEL_DISABLE ? "disabled (deterministic)" : `${PROVIDER}/${MODEL} thinking=${THINKING}`;
    report.stats.decision_llm_calls = stats.llm_calls;
    report.stats.decision_deterministic = stats.deterministic;
    report.stats.decision_pi_available = stats.pi_available;
    report.stats.decision_errors = llmErrors.slice(0, 5);
    writeFileSync(join(notesRunDir, "report.json"), JSON.stringify(report, null, 2));

    console.log(`decisions added: ${added.length} (carried ${carried.length}) | sessions ${stats.sessions} | llm_calls ${stats.llm_calls} | deterministic ${stats.deterministic} | pi ${stats.pi_available ? "available" : "missing/disabled"} | mode snapshot | ${ms}ms`);
    if (llmErrors.length) console.log(`llm errors: ${llmErrors.length} (first: ${llmErrors[0].error})`);
    console.log(`notes run: ${notesRunDir} (total notes ${all.length})`);
  }
} finally {
  lock.release();
}
