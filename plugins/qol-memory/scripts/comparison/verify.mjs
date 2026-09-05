import { mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { evaluate, validateDataset, validateSplitSeparation, workerInput } from "./scoring.mjs";
import { buildWorker, hashFile, readJson, runCommand, writeJson } from "./artifacts.mjs";
import { withLocalModel } from "./local-model.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../../..");
const gate = Object.freeze({ max_wrong_answers: 0, min_answer_coverage: 0.9, max_response_p95_ms: 200, max_verification_p95_ms: 10_000 });
const inputs = ["src/verification/mod.rs", "src/verification/ollama.rs", "src/verification/service.rs", "src/verification/profile.json", "src/ask/semantic.rs", "tests/fixtures/answer-verification/development.json", "tests/fixtures/answer-verification/heldout.json"];

function options(args) {
  const settings = { ...readJson(join(root, "plugins/qol-memory/src/verification/profile.json")), repeats: 2 };
  while (args.length) {
    const flag = args.shift();
    if (flag === "--prepare") { settings.prepare = true; continue; }
    if (flag === "--endpoint" && args[0]) { settings.endpoint = args.shift(); continue; }
    if (flag === "--repeats" && /^[1-3]$/.test(args[0] ?? "")) { settings.repeats = Number(args.shift()); continue; }
    throw new Error(`Unknown or incomplete verifier option: ${flag}`);
  }
  return settings;
}

async function inputHashes() {
  return Object.fromEntries(await Promise.all(inputs.map(async path => [path, await hashFile(join(root, "plugins/qol-memory", path))])));
}

export function score(dataset, rows) {
  const cached = evaluate(dataset, rows);
  const initial = evaluate(dataset, rows.map(row => ({ ...row, samples_ms: [row.initial_ms] })));
  const completion = evaluate(dataset, rows.map(row => ({ ...row, samples_ms: [row.completion_ms] })));
  const qualifies = cached.summary.wrong_answers <= gate.max_wrong_answers
    && cached.summary.answer_coverage >= gate.min_answer_coverage
    && cached.summary.warm_p95_ms <= gate.max_response_p95_ms
    && initial.summary.warm_p95_ms <= gate.max_response_p95_ms
    && completion.summary.warm_p95_ms <= gate.max_verification_p95_ms;
  return { cached, initial, completion, first_completion_ms: rows[0].completion_ms, max_completion_ms: Math.max(...rows.map(row => row.completion_ms)), qualifies };
}

export async function verify(args) {
  const started = new Date().toISOString();
  const out = join(root, "reports/qol-memory/verification", started.replaceAll(":", "-"));
  mkdirSync(out, { recursive: true });
  const report = { name: "qol-memory-runtime-verification", started_at: started, status: "failed", inputs: {}, artifacts: {}, commands: [] };
  try {
    const settings = options([...args]);
    report.inputs = { ...settings, gate, hashes: await inputHashes() };
    const baseline = await buildWorker(root, out, report, "matcher-baseline", ["-p", "qol-memory", "--example", "matcher-baseline"]);
    const runtime = await buildWorker(root, out, report, "matcher-runtime", ["-p", "qol-memory", "--example", "matcher-runtime"]);
    writeJson(join(out, "frozen-policy.json"), report.inputs);
    await withLocalModel(settings, out, report, async endpoint => {
      const runSplit = dataset => {
        const input = workerInput(dataset, settings.repeats);
        const cold = JSON.parse(runCommand(root, out, report, `${dataset.split}-baseline`, [baseline, join(out, `${dataset.split}-store`)], input));
        const live = JSON.parse(runCommand(root, out, report, `${dataset.split}-runtime`, [runtime, join(out, `${dataset.split}-runtime-store`), endpoint], input));
        const rounds = live.rounds.map(rows => score(dataset, rows));
        const worst = [...rounds].sort((a, b) => b.cached.summary.wrong_answers - a.cached.summary.wrong_answers || a.cached.summary.correct_answers - b.cached.summary.correct_answers || b.completion.summary.warm_p95_ms - a.completion.summary.warm_p95_ms)[0];
        return { current: evaluate(dataset, cold.results), rounds, summary: worst.cached.summary, qualifies: rounds.every(round => round.qualifies) };
      };
      const development = validateDataset(readJson(join(root, "plugins/qol-memory/tests/fixtures/answer-verification/development.json")), "development");
      report.development = runSplit(development);
      if (!report.development.qualifies) throw new Error("Development runtime gate failed; fresh held-out questions remain unused");
      const heldout = validateDataset(readJson(join(root, "plugins/qol-memory/tests/fixtures/answer-verification/heldout.json")), "heldout");
      validateSplitSeparation(development, heldout);
      validateSplitSeparation(readJson(join(root, "plugins/qol-memory/tests/fixtures/matcher-comparison/heldout.json")), heldout);
      report.heldout = runSplit(heldout);
    });
    for (const name of ["matcher-baseline", "matcher-runtime"]) {
      if (await hashFile(report.artifacts[name].path) !== report.artifacts[name].sha256) throw new Error("Evaluation binary changed");
    }
    if (JSON.stringify(await inputHashes()) !== JSON.stringify(report.inputs.hashes)) throw new Error("Evaluation inputs changed during the run");
    const before = report.heldout.current.summary;
    const after = report.heldout.summary;
    report.decision = { qualified: report.development.qualifies && report.heldout.qualifies };
    report.metrics = [
      { metric: "Correct answers / answerable", before: `${before.correct_answers}/${before.answerable}`, after: `${after.correct_answers}/${after.answerable}`, delta: `${after.correct_answers - before.correct_answers} answers` },
      { metric: "Wrong answers", before: before.wrong_answers, after: after.wrong_answers, delta: after.wrong_answers - before.wrong_answers },
      { metric: "Cached answer p95 (ms)", before: before.warm_p95_ms.toFixed(2), after: Math.max(...report.heldout.rounds.map(round => round.cached.summary.warm_p95_ms)).toFixed(2), delta: "includes binding validation" },
    ].map(row => ({ improvement_vector: "Guarded answer verification", scenario: "Fresh held-out questions", context: `${settings.repeats} fresh stores; ${after.total} queries; worst round shown; every round must qualify`, ...row, correctness: report.decision.qualified ? "Qualified" : "Rejected", evidence: join(out, "report.json") }));
    report.status = report.decision.qualified ? "pass" : "failed";
    if (!report.decision.qualified) process.exitCode = 1;
    process.stdout.write(`${JSON.stringify({ development: report.development.summary, heldout: after, decision: report.decision }, null, 2)}\n`);
  } catch (error) {
    report.error = String(error);
    process.exitCode = 1;
    process.stderr.write(`${error}\n`);
  }
  report.finished_at = new Date().toISOString();
  writeJson(join(out, "report.json"), report);
  process.stdout.write(`${join(out, "report.json")}\n`);
}
