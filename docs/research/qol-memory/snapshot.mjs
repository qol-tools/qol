#!/usr/bin/env node
import { readdirSync, statSync, lstatSync, mkdirSync, writeFileSync, unlinkSync } from "node:fs";
import { createInterface } from "node:readline";
import { createReadStream } from "node:fs";
import { join, resolve, basename, dirname } from "node:path";
import { homedir } from "node:os";
import { createHash } from "node:crypto";

const args = process.argv.slice(2);
const pick = (flag, def) => {
  const i = args.indexOf(flag);
  return i >= 0 && args[i + 1] ? args[i + 1] : def;
};
const PI_DIR = resolve(pick("--pi-dir", join(homedir(), ".pi", "agent", "sessions")));
const CLAUDE_DIR = resolve(pick("--claude-dir", join(homedir(), ".claude", "projects")));
const RUN_ID = new Date().toISOString().replace(/[:.]/g, "-");
const OUT_DIR = resolve(pick("--out", join(process.cwd(), "reports", "qol-memory", "snapshot", RUN_ID)));
const rawMaxSamples = Number(pick("--max-samples", "500"));
const MAX_SAMPLES = Number.isInteger(rawMaxSamples) && rawMaxSamples > 0 ? rawMaxSamples : 500;
const KEEP = Number.isInteger(Number(pick("--keep", "5"))) ? Math.max(1, Number(pick("--keep", "5"))) : 5;
const MAX_DEPTH = 8;

const SCRIPTS = [
  ["cjk", /[\u3040-\u30ff\u3400-\u4dbf\u4e00-\u9fff\uf900-\ufaff]/],
  ["cyrillic", /[\u0400-\u04ff]/],
  ["arabic", /[\u0600-\u06ff]/],
  ["greek", /[\u0370-\u03ff]/],
  ["hebrew", /[\u0590-\u05ff]/],
  ["devanagari", /[\u0900-\u097f]/],
  ["hangul", /[\uac00-\ud7af]/],
];

function walk(dir, depth = 0) {
  if (depth > MAX_DEPTH) return [];
  const out = [];
  let names;
  if (depth === 0) {
    names = readdirSync(dir);
  } else {
    try {
      names = readdirSync(dir);
    } catch {
      return out;
    }
  }
  for (const name of names.sort()) {
    const p = join(dir, name);
    let st;
    try {
      st = lstatSync(p);
    } catch {
      continue;
    }
    if (st.isSymbolicLink()) continue;
    if (st.isDirectory()) {
      if (name === "memory") continue;
      out.push(...walk(p, depth + 1));
    } else if (name.endsWith(".jsonl")) {
      out.push(p);
    }
  }
  return out;
}

function textOf(content) {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content
    .filter((b) => b && b.type === "text" && typeof b.text === "string")
    .map((b) => b.text)
    .join("\n");
}

function thinkingOf(content) {
  if (!Array.isArray(content)) return "";
  return content
    .filter((b) => b && b.type === "thinking" && typeof b.thinking === "string")
    .map((b) => b.thinking)
    .join("\n");
}

function countBlocks(content, type) {
  if (!Array.isArray(content)) return 0;
  return content.filter((b) => b && b.type === type).length;
}

function toIso(ts) {
  if (typeof ts === "number") return new Date(ts).toISOString();
  return ts || null;
}

function unitKey(source, file, ts, text) {
  return createHash("sha256").update([source, file, ts, text].join("|")).digest("hex").slice(0, 16);
}

async function processFile(file, source, stats, units) {
  const rl = createInterface({ input: createReadStream(file, "utf8"), crlfDelay: Infinity });
  let sessionId = null;
  let cwd = null;
  let lines = 0;
  let badLines = 0;
  for await (const line of rl) {
    lines++;
    let e;
    try {
      e = JSON.parse(line);
    } catch {
      badLines++;
      continue;
    }
    const t = e.type;
    if (t === "session") {
      sessionId = e.id;
      cwd = e.cwd;
      continue;
    }
    if (t === "message") {
      const m = e.message || {};
      const role = m.role;
      const content = m.content;
      const ts = toIso(m.timestamp);
      if (role === "user") {
        const text = textOf(content);
        stats.chars.user += text.length;
        units.push({ key: unitKey(source, basename(file), ts, text), source, file: basename(file), session: sessionId, cwd, kind: "user", ts, text });
        stats.units.byKind.user++;
      } else if (role === "assistant") {
        stats.chars.assistant += textOf(content).length;
        stats.chars.thinking += thinkingOf(content).length;
      } else if (role === "toolResult" || role === "bashExecution") {
        stats.chars.tool += textOf(content).length;
      }
      stats.images[source] += countBlocks(content, "image");
      continue;
    }
    if (t === "compaction") {
      const d = e.details || {};
      const text = e.summary || "";
      const ts = toIso(e.timestamp);
      stats.chars.summaries += text.length;
      units.push({
        key: unitKey(source, basename(file), ts, text),
        source,
        file: basename(file),
        session: sessionId,
        cwd,
        kind: "compaction",
        ts,
        text,
        filesRead: d.readFiles || [],
        filesModified: d.modifiedFiles || [],
      });
      stats.units.byKind.compaction++;
      continue;
    }
    if (t === "branch_summary") {
      const text = e.summary || "";
      const ts = toIso(e.timestamp);
      stats.chars.summaries += text.length;
      units.push({ key: unitKey(source, basename(file), ts, text), source, file: basename(file), session: sessionId, cwd, kind: "branch", ts, text });
      stats.units.byKind.branch++;
      continue;
    }
    if (t === "user") {
      const content = e.message ? e.message.content : e.content;
      const isToolResult = Array.isArray(content) && content.some((b) => b && b.type === "tool_result");
      if (isToolResult) {
        stats.chars.tool += textOf(content).length;
        continue;
      }
      const text = textOf(content);
      if (!text.trim()) continue;
      sessionId = e.sessionId || sessionId;
      cwd = e.cwd || cwd;
      const ts = toIso(e.timestamp);
      stats.chars.user += text.length;
      units.push({ key: unitKey(source, basename(file), ts, text), source, file: basename(file), session: sessionId, cwd, kind: "user", ts, text });
      stats.units.byKind.user++;
      continue;
    }
    if (t === "summary") {
      const text = e.summary || "";
      if (!text.trim()) continue;
      const ts = toIso(e.timestamp);
      stats.chars.summaries += text.length;
      units.push({ key: unitKey(source, basename(file), ts, text), source, file: basename(file), session: sessionId, cwd, kind: "compaction", ts, text });
      stats.units.byKind.compaction++;
      continue;
    }
    if (t === "assistant") {
      const content = e.message ? e.message.content : e.content;
      stats.chars.assistant += textOf(content).length;
      stats.chars.thinking += thinkingOf(content).length;
      stats.images[source] += countBlocks(content, "image");
    }
  }
  return { lines, badLines };
}

const started = new Date();
const stats = {
  files: { pi: 0, claude: 0 },
  bytes: { pi: 0, claude: 0 },
  units: { pi: 0, claude: 0, byKind: { user: 0, compaction: 0, branch: 0 } },
  chars: { user: 0, assistant: 0, thinking: 0, tool: 0, summaries: 0 },
  images: { pi: 0, claude: 0 },
  errors: { files: 0, lines: 0, filesList: [], sources: [] },
  lang: { sampled: 0, bySource: { pi: 0, claude: 0 }, nonLatin: 0, scripts: {} },
};
for (const [name, s] of SCRIPTS) stats.lang.scripts[name] = 0;

const units = [];
const jobs = [
  [PI_DIR, "pi"],
  [CLAUDE_DIR, "claude"],
];
for (const [dir, source] of jobs) {
  let files;
  try {
    files = walk(dir);
  } catch (err) {
    stats.errors.sources.push({ source, error: err.message });
    continue;
  }
  for (const f of files) {
    let size;
    try {
      size = statSync(f).size;
    } catch {
      stats.errors.files++;
      stats.errors.filesList.push({ file: basename(f), badLines: 0, error: "stat failed" });
      continue;
    }
    stats.files[source]++;
    stats.bytes[source] += size;
    try {
      const { badLines } = await processFile(f, source, stats, units);
      if (badLines > 0) {
        stats.errors.files++;
        stats.errors.lines += badLines;
        stats.errors.filesList.push({ file: basename(f), badLines });
      }
    } catch (err) {
      stats.errors.files++;
      stats.errors.filesList.push({ file: basename(f), badLines: 0, error: err.message });
    }
  }
}
stats.units.pi = units.filter((u) => u.source === "pi").length;
stats.units.claude = units.filter((u) => u.source === "claude").length;

const userUnits = units.filter((u) => u.kind === "user");
const piUsers = userUnits.filter((u) => u.source === "pi");
const claudeUsers = userUnits.filter((u) => u.source === "claude");
const piShare = userUnits.length ? piUsers.length / userUnits.length : 0;
const piBudget = Math.round(MAX_SAMPLES * piShare);
const sampled = [...piUsers.slice(0, piBudget), ...claudeUsers.slice(0, MAX_SAMPLES - piBudget)];
stats.lang.sampled = sampled.length;
for (const u of sampled) {
  stats.lang.bySource[u.source]++;
  let hit = false;
  for (const [name, re] of SCRIPTS) {
    if (re.test(u.text)) {
      stats.lang.scripts[name]++;
      hit = true;
    }
  }
  if (hit) stats.lang.nonLatin++;
}
stats.lang.nonLatinShare = stats.lang.sampled ? stats.lang.nonLatin / stats.lang.sampled : 0;

const status = stats.errors.sources.length > 0 || stats.errors.files > 0 ? (stats.units.pi + stats.units.claude === 0 ? "fail" : "degraded") : "pass";

mkdirSync(OUT_DIR, { recursive: true });
const snapshotPath = join(OUT_DIR, "snapshot.jsonl");
writeFileSync(snapshotPath, units.map((u) => JSON.stringify(u)).join("\n") + (units.length ? "\n" : ""));
const reportPath = join(OUT_DIR, "report.json");
const finished = new Date();
writeFileSync(
  reportPath,
  JSON.stringify(
    {
      name: "qol-memory snapshot",
      schemaVersion: 2,
      started_at: started.toISOString(),
      finished_at: finished.toISOString(),
      status,
      inputs: { piDir: PI_DIR, claudeDir: CLAUDE_DIR, maxSamples: MAX_SAMPLES, keep: KEEP },
      artifacts: { snapshot: join(basename(dirname(snapshotPath)), basename(snapshotPath)), report: basename(reportPath) },
      commands: [`node docs/research/qol-memory/snapshot.mjs`],
      stats,
      next: [
        "Draft the held-out question set from memory of the work, before reading snapshot units",
        "Build the eval harness: zero-dep BM25 baseline over the pinned snapshot run, hit@1/hit@5",
      ],
    },
    null,
    2
  )
);

const snapshotRoot = dirname(dirname(OUT_DIR));
try {
  const runs = readdirSync(snapshotRoot)
    .filter((n) => /^\d{4}-\d{2}-\d{2}T/.test(n))
    .map((n) => ({ n, d: statSync(join(snapshotRoot, n)).mtimeMs }))
    .sort((a, b) => b.d - a.d);
  for (const run of runs.slice(KEEP)) {
    const runDir = join(snapshotRoot, run.n);
    try {
      for (const f of readdirSync(runDir)) {
        try {
          unlinkSync(join(runDir, f));
        } catch {}
      }
    } catch {}
  }
} catch {}

console.log(`status: ${status}`);
console.log(`files: pi ${stats.files.pi} (${(stats.bytes.pi / 1e6).toFixed(0)}MB), claude ${stats.files.claude} (${(stats.bytes.claude / 1e9).toFixed(1)}GB)`);
console.log(`units: ${stats.units.pi + stats.units.claude} (user ${stats.units.byKind.user}, compaction ${stats.units.byKind.compaction}, branch ${stats.units.byKind.branch})`);
console.log(`chars: user ${(stats.chars.user / 1e6).toFixed(1)}M, assistant ${(stats.chars.assistant / 1e6).toFixed(1)}M, thinking ${(stats.chars.thinking / 1e6).toFixed(2)}M, tool ${(stats.chars.tool / 1e6).toFixed(1)}M, summaries ${(stats.chars.summaries / 1e3).toFixed(0)}k`);
console.log(`lang: ${stats.lang.nonLatin}/${stats.lang.sampled} sampled user units non-Latin (${(stats.lang.nonLatinShare * 100).toFixed(1)}%), pi ${stats.lang.bySource.pi} / claude ${stats.lang.bySource.claude}`);
console.log(`report: ${reportPath}`);
