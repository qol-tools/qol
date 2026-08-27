#!/usr/bin/env node
import { readdirSync, readFileSync, mkdirSync, writeFileSync, existsSync, rmSync, cpSync, mkdtempSync } from "node:fs";
import { join, resolve, dirname, basename } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { qolMemoryStore } from "./lib/store-path.js";
import { trySealedText } from "./lib/seal.js";

const BASE = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(BASE, "..", "..", "..");
const STARTED_AT = new Date().toISOString();

const args = process.argv.slice(2);
const pick = (flag, def) => {
  const i = args.indexOf(flag);
  return i >= 0 && args[i + 1] ? args[i + 1] : def;
};
const BIN = resolve(pick("--bin", process.env.QOL_MEMORY_BIN || join(REPO_ROOT, "target", "debug", "qol-memory")));
const STORE_ROOT = resolve(pick("--store", qolMemoryStore()));
const LIMIT = (() => {
  const raw = pick("--limit", null);
  if (raw === null) return Infinity;
  const n = Number(raw);
  if (!Number.isFinite(n) || n < 0) throw new Error(`--limit must be a non-negative number, got ${raw}`);
  return Math.trunc(n);
})();
const QUESTION_FILES = pick("--questions", null)
  ? pick("--questions", null)
      .split(",")
      .map((f) => f.trim())
      .filter(Boolean)
      .map((f) => resolve(f))
  : ["eval/questions.json", "eval/heldout.json", "eval/skills-questions.json"].map((f) => join(BASE, f));

function loadQuestions() {
  const out = [];
  for (const file of QUESTION_FILES) {
    if (!existsSync(file)) throw new Error(`questions file missing: ${file}`);
    const data = JSON.parse(readFileSync(file, "utf8"));
    if (!Array.isArray(data.questions)) throw new Error(`${file}: no questions array`);
    for (const q of data.questions) {
      if (!q.query) continue;
      out.push({ id: q.id ?? `@${out.length}`, src: basename(file), query: q.query });
    }
  }
  return out;
}

function newestRunDir(root) {
  const runs = readdirSync(root)
    .filter((n) => /^\d{4}-\d{2}-\d{2}T/.test(n))
    .sort()
    .reverse();
  return runs.length ? join(root, runs[0]) : null;
}

function firstUnitSession(storeRoot) {
  const live = join(storeRoot, "units.jsonl");
  let text = null;
  if (existsSync(live)) {
    const raw = readFileSync(live);
    text = trySealedText(storeRoot, raw) || raw.toString("utf8");
  } else {
    const snapDir = existsSync(join(storeRoot, "snapshot")) ? newestRunDir(join(storeRoot, "snapshot")) : null;
    if (snapDir && existsSync(join(snapDir, "snapshot.jsonl"))) {
      text = readFileSync(join(snapDir, "snapshot.jsonl"), "utf8");
    }
  }
  if (!text) return null;
  for (const line of text.split("\n")) {
    if (!line.trim()) continue;
    try {
      const u = JSON.parse(line);
      return u && u.session ? String(u.session) : null;
    } catch {
      return null;
    }
  }
  return null;
}

const MISSING = Symbol("missing");

const render = (v) => (v === MISSING ? "<absent>" : v);

function diffValue(a, b, ptr, add) {
  if ((a === MISSING) !== (b === MISSING)) {
    add(ptr || "/", render(a), render(b));
    return;
  }
  if (a === MISSING && b === MISSING) return;
  if (typeof a === "number" && typeof b === "number") {
    if (!(a === b || (Number.isNaN(a) && Number.isNaN(b)))) add(ptr || "/", a, b);
    return;
  }
  if (a === null || b === null) {
    if (a !== b) add(ptr || "/", a, b);
    return;
  }
  if (typeof a !== typeof b) {
    add(ptr || "/", a, b);
    return;
  }
  if (Array.isArray(a)) {
    const n = Math.max(a.length, b.length);
    for (let i = 0; i < n; i++) diffValue(i < a.length ? a[i] : MISSING, i < b.length ? b[i] : MISSING, `${ptr}/${i}`, add);
    return;
  }
  if (typeof a === "object") {
    for (const k of Object.keys(a)) diffValue(a[k], k in b ? b[k] : MISSING, `${ptr}/${k}`, add);
    for (const k of Object.keys(b)) if (!(k in a)) diffValue(MISSING, b[k], `${ptr}/${k}`, add);
    return;
  }
  if (a !== b) add(ptr || "/", a, b);
}

const META_RE = /^idx-.+\.json\.meta$/;
const IDX_FILE_RE = /^idx-.+\.json(\.meta)?$/;

function clearCaches(root) {
  if (!existsSync(root)) return;
  for (const n of readdirSync(root)) {
    if (!IDX_FILE_RE.test(n)) continue;
    try {
      rmSync(join(root, n));
    } catch {}
  }
}

function snapMetas(dest, root) {
  mkdirSync(dest, { recursive: true });
  if (!existsSync(root)) return 0;
  let count = 0;
  for (const n of readdirSync(root)) {
    if (!META_RE.test(n)) continue;
    cpSync(join(root, n), join(dest, n));
    count++;
  }
  return count;
}

function compareMetaDirs(dirA, dirB, pushMetaMismatch) {
  const names = [...new Set([...readdirSync(dirA), ...readdirSync(dirB)])].sort();
  let compared = 0;
  for (const name of names) {
    compared++;
    const pathA = join(dirA, name);
    const pathB = join(dirB, name);
    if (!existsSync(pathA) || !existsSync(pathB)) {
      pushMetaMismatch(name, existsSync(pathA) ? "<present>" : "<absent>", existsSync(pathB) ? "<present>" : "<absent>", "");
      continue;
    }
    let metaA;
    let metaB;
    try {
      metaA = JSON.parse(readFileSync(pathA, "utf8"));
      metaB = JSON.parse(readFileSync(pathB, "utf8"));
    } catch (e) {
      pushMetaMismatch(name, "<invalid json>", "<invalid json>", String(e.message || e));
      continue;
    }
    diffValue(metaA, metaB, "", (p, js, rust) =>
      pushMetaMismatch(name, js, rust, p === "/" ? "" : p.slice(1))
    );
  }
  return compared;
}

const childEnv = () => ({ ...process.env, QOL_MEMORY_STORE: STORE_ROOT });

function runSide(side, query, flags) {
  const argv =
    side === "js"
      ? [join(BASE, "ask.mjs"), query, ...flags]
      : ["--json", "ask", query, ...flags];
  return spawnSync(side === "js" ? process.execPath : BIN, argv, {
    env: childEnv(),
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
    timeout: 300000,
  });
}

function parseOne(side, res) {
  if (res.error) return { err: `<${side}> spawn error: ${res.error.message}` };
  if (res.status !== 0) return { err: `<${side}> exit ${res.status}: ${(res.stderr || "").slice(0, 300).trim()}` };
  try {
    return { doc: JSON.parse(res.stdout) };
  } catch (e) {
    return { err: `<${side}> stdout not JSON (${String(e.message || e)}): ${res.stdout.slice(0, 200)}` };
  }
}

async function main() {
  if (!existsSync(BIN)) {
    console.error(`parity: binary missing: ${BIN}`);
    process.exit(1);
  }

  const allQuestions = loadQuestions();
  const questions = Number.isFinite(LIMIT) ? allQuestions.slice(0, LIMIT) : allQuestions;
  if (!questions.length) throw new Error("no questions to compare");

  const excludeSession = firstUnitSession(STORE_ROOT);

  const outDir = join(STORE_ROOT, "eval", `parity-${STARTED_AT.replace(/[:.]/g, "-")}`);
  mkdirSync(outDir, { recursive: true });
  const reportPath = join(outDir, "report.json");
  const scratch = mkdtempSync(join(tmpdir(), "qol-memory-parity-"));

  const fullCount = Math.min(5, questions.length);
  const exclCount = Math.min(2, questions.length);
  const results = questions.map((q) => ({
    id: q.id,
    src: q.src,
    query: q.query,
    brief: [],
    full: [],
    exclude: [],
    ok: true,
  }));

  const mismatches = [];
  let metasCompared = 0;

  const plan = [];
  questions.forEach((_, qi) => plan.push({ qi, mode: "brief" }));
  for (let qi = 0; qi < fullCount; qi++) plan.push({ qi, mode: "full" });
  if (excludeSession) {
    for (let qi = 0; qi < exclCount; qi++) plan.push({ qi, mode: "exclude" });
  } else {
    for (let qi = 0; qi < exclCount; qi++) {
      results[qi].exclude.push({ skipped: "no unit session found in store (units.jsonl or newest snapshot)" });
    }
  }

  for (const passNo of [1, 2]) {
    const jsFirst = passNo === 1;
    clearCaches(STORE_ROOT);
    for (const step of plan) {
      const q = questions[step.qi];
      const flags = [];
      if (step.mode !== "full") flags.push("--brief");
      if (step.mode === "exclude") flags.push("--exclude-session", excludeSession);
      flags.push("--no-log");

      const tag = `pass${passNo}-${step.mode}-q${step.qi}`;
      const runs = {};
      for (const side of jsFirst ? ["js", "rust"] : ["rust", "js"]) {
        runs[side] = runSide(side, q.query, flags);
        snapMetas(join(scratch, `${tag}-${side}`), STORE_ROOT);
      }

      const problems = [];
      const parsed = { js: parseOne("js", runs.js), rust: parseOne("rust", runs.rust) };
      for (const side of ["js", "rust"]) {
        if (parsed[side].err) {
          problems.push({
            kind: "process",
            pass: passNo,
            mode: step.mode,
            question: q.id,
            path: "/",
            js: side === "js" ? parsed[side].err : "",
            rust: side === "rust" ? parsed[side].err : "",
          });
        }
      }
      if (parsed.js.doc && parsed.rust.doc) {
        diffValue(parsed.js.doc, parsed.rust.doc, "", (p, js, rust) =>
          problems.push({ kind: "output", pass: passNo, mode: step.mode, question: q.id, path: p, js, rust })
        );
      }
      metasCompared += compareMetaDirs(
        join(scratch, `${tag}-js`),
        join(scratch, `${tag}-rust`),
        (layerName, jsVal, rustVal, field) =>
          problems.push({
            kind: "meta",
            pass: passNo,
            mode: step.mode,
            question: q.id,
            path: field ? `${layerName}/${field}` : layerName,
            js: jsVal,
            rust: rustVal,
          })
      );

      results[step.qi][step.mode].push({ pass: passNo, ok: problems.length === 0 });
      if (problems.length) {
        results[step.qi].ok = false;
        mismatches.push(...problems);
      }
    }
  }

  const passed = results.filter((r) => r.ok).length;
  const total = results.length;

  writeFileSync(
    reportPath,
    JSON.stringify(
      {
        name: "qol-memory parity (ask.mjs vs qol-memory binary)",
        schemaVersion: 1,
        started_at: STARTED_AT,
        finished_at: new Date().toISOString(),
        status: mismatches.length ? "fail" : "pass",
        inputs: {
          bin: BIN,
          store: STORE_ROOT,
          questions: QUESTION_FILES,
          limit: Number.isFinite(LIMIT) ? LIMIT : null,
          exclude_session: excludeSession,
        },
        artifacts: { report: reportPath },
        commands: [`node docs/research/qol-memory/parity.mjs ${args.join(" ")}`.trimEnd()],
        summary: { total, passed, mismatches: mismatches.length, metas_compared: metasCompared },
        scratch_meta_dir: scratch,
        results,
        mismatches,
      },
      null,
      2
    )
  );

  console.log(`report: ${reportPath}`);
  console.log(`parity: ${passed}/${total} questions, ${mismatches.length} mismatches`);
  process.exitCode = mismatches.length ? 1 : 0;
}

main().catch((e) => {
  console.error(e && e.stack ? e.stack : String(e));
  process.exit(1);
});
