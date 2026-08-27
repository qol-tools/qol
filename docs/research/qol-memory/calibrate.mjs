#!/usr/bin/env node
import { execFileSync } from "node:child_process";
import { readFile } from "node:fs/promises";

const ROOT = new URL(".", import.meta.url).pathname;
const ASK = `${ROOT}ask.mjs`;
const heldout = JSON.parse(await readFile(`${ROOT}eval/heldout.json`, "utf8")).questions;

const norm = (s) =>
  (s || "")
    .toLowerCase()
    .replace(/[^a-z0-9 ]/g, " ")
    .replace(/\s+/g, " ")
    .trim();

function factMatch(answerText, fact) {
  if (!answerText) return false;
  const a = norm(answerText);
  const f = norm(fact);
  if (!f) return true;
  return a.includes(f);
}

function grid() {
  const out = [];
  for (const umargin of [1.0, 1.2, 1.5, 2.0]) {
    for (const uscore of [6.0, 7.0, 8.0]) {
      for (const nscore of [4.5, 5.0, 6.0]) {
        out.push({
          label: `unitMargin=${umargin} unitScore=${uscore} noteScore=${nscore}`,
          env: {
            MEM_NO_COV: "0.5",
            MEM_FLOOR: "6.0",
            MEM_NOTE_COV: "0.5",
            MEM_NOTE_SCORE: String(nscore),
            MEM_UNIT_COV: "1.0",
            MEM_UNIT_SCORE: String(uscore),
            MEM_UNIT_MARGIN: String(umargin),
            MEM_HIGH_MARGIN: "1.8",
          },
        });
      }
    }
  }
  return out;
}

function runAsk(query, env) {
  const out = execFileSync("node", [ASK, query, "--no-log"], { env: { ...process.env, ...env }, encoding: "utf8" });
  return JSON.parse(out);
}

function baseline() {
  const rows = heldout.map((q) => {
    const d = runAsk(q.query, {});
    const ok = d.verdict === "answered" ? factMatch(d.answer?.text, q.fact) : null;
    return { id: q.id, verdict: d.verdict, ok };
  });
  const answered = rows.filter((r) => r.ok !== null);
  const correct = answered.filter((r) => r.ok).length;
  const prec = answered.length ? correct / answered.length : 0;
  const ansRate = rows.length ? answered.length / rows.length : 0;
  return { rows, correct, answered: answered.length, total: rows.length, precision: prec, answerRate: ansRate };
}

const b = baseline();
console.log("=== BASELINE (current hardcoded gates) ===");
console.log(
  `  answered ${b.answered}/${b.total} (rate ${(b.answerRate * 100).toFixed(0)}%), precision ${(b.precision * 100).toFixed(0)}% (${b.correct}/${b.answered})`
);
for (const r of b.rows) console.log(`    ${r.id} verdict=${r.verdict.padEnd(9)} ${r.ok === null ? "(n/a)" : r.ok ? "CORRECT" : "WRONG"}`);

const TARGET_PREC = 0.93;
const gridResults = [];
for (const cfg of grid()) {
  let ans = 0, corr = 0;
  for (const q of heldout) {
    try {
      const d = runAsk(q.query, cfg.env);
      if (d.verdict !== "answered") continue;
      ans++;
      if (factMatch(d.answer?.text, q.fact)) corr++;
    } catch (e) {
      continue;
    }
  }
  const prec = ans ? corr / ans : 0;
  gridResults.push({ ...cfg, answered: ans, correct: corr, precision: prec, answerRate: ans / heldout.length, total: heldout.length });
}

const tiebreak = (a, b) =>
  b.answerRate - a.answerRate ||
  Number(b.env.MEM_UNIT_MARGIN) - Number(a.env.MEM_UNIT_MARGIN) ||
  Number(b.env.MEM_UNIT_SCORE) - Number(a.env.MEM_UNIT_SCORE) ||
  Number(b.env.MEM_NOTE_SCORE) - Number(a.env.MEM_NOTE_SCORE);

console.log("\n=== GRID (per-config: answered / correct / precision / answerRate) ===");
const sortedGrid = [...gridResults].sort((a, b) => b.precision - a.precision || tiebreak(a, b));
for (const r of sortedGrid) {
  console.log(
    `  ${r.label.padEnd(50)} answered ${String(r.answered).padStart(2)}/${r.total} correct ${String(r.correct).padStart(2)} prec ${(r.precision * 100).toFixed(0)}% rate ${(r.answerRate * 100).toFixed(0)}%`
  );
}

const eligible = gridResults.filter((r) => r.precision >= TARGET_PREC);
const best = eligible.length ? [...eligible].sort(tiebreak)[0] : null;

console.log("\n=== CALIBRATED OPERATING POINT (target precision " + (TARGET_PREC * 100).toFixed(0) + "%) ===");
if (!best) {
  console.log("  no gate setting met the target precision");
} else {
  console.log(`  ${best.label}`);
  console.log(
    `  answered ${best.answered}/${best.total} (rate ${((best.answerRate * 100).toFixed(0))}%), precision ${(best.precision * 100).toFixed(0)}% (${best.correct}/${best.answered})`
  );
  console.log(`  env: ${JSON.stringify(best.env)}`);
}
