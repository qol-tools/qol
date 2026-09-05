import { mkdirSync, readFileSync, readdirSync } from "node:fs";
import { dirname, join, resolve, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { arch, availableParallelism, cpus, platform } from "node:os";
import { acceptance, calibrate, evaluate, validateDataset, validateSplitSeparation, workerInput } from "./scoring.mjs";
import { buildWorker, hashFile, prepareModels, readJson, runCommand, sha256, writeJson } from "./artifacts.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../../..");
const fixtures = join(root, "plugins/qol-memory/tests/fixtures/matcher-comparison");
const directory = dirname(fileURLToPath(import.meta.url));

function options(args) {
  const result = { offline: false, repeats: 3, cache: join(root, "target/qol-memory-models") };
  while (args.length) {
    const flag = args.shift();
    if (flag === "--offline") { result.offline = true; continue; }
    if (flag === "--model-cache" && args[0]) { result.cache = resolve(args.shift()); continue; }
    if (flag === "--repeats" && /^[1-9]$|^10$/.test(args[0] ?? "")) { result.repeats = Number(args.shift()); continue; }
    throw new Error(`Unknown or incomplete option: ${flag}`);
  }
  return result;
}

function sourceFiles(path) {
  return readdirSync(path, { withFileTypes: true }).flatMap((entry) => {
    const child = join(path, entry.name);
    return entry.isDirectory() ? sourceFiles(child) : [child];
  });
}

function sourceProof() {
  const files = [
    ...sourceFiles(directory), ...sourceFiles(fixtures),
    ...sourceFiles(join(root, "docs/research/qol-memory/tier1/src")),
    ...sourceFiles(join(root, "plugins/qol-memory/src")),
    join(root, "plugins/qol-memory/examples/matcher-baseline.rs"),
    join(root, "plugins/qol-memory/scripts/evaluate.mjs"),
    join(root, "plugins/qol-memory/Cargo.toml"), join(root, "Cargo.lock"),
    join(root, "docs/research/qol-memory/tier1/Cargo.toml"),
    join(root, "docs/research/qol-memory/tier1/Cargo.lock"),
  ].sort();
  return Object.fromEntries(files.map((file) => [relative(root, file), sha256(readFileSync(file))]));
}

function modelResults(baseline, models) {
  const timings = new Map(baseline.results.map((row) => [row.id, row.lexical_ms]));
  return {
    current: baseline.results.map((row) => ({ ...row, retrieved: row.lexical.slice(0, 3) })),
    embedding: models.embedding,
    hybrid_equivalence: models.hybrid_equivalence.map((row) => ({ ...row, samples_ms: row.samples_ms.map((ms) => ms + timings.get(row.id)) })),
  };
}

function summarize(dataset, results, policies) {
  return Object.fromEntries(Object.entries(results).map(([name, rows]) => [name, evaluate(dataset, rows, name === "current" ? null : policies[name].threshold)]));
}

function metrics(report, out) {
  const baseline = report.heldout.current.summary;
  return Object.entries(report.heldout).flatMap(([name, result]) => [
    { metric: "Correct answers / answerable", before: `${baseline.correct_answers}/${baseline.answerable}`, after: `${result.summary.correct_answers}/${result.summary.answerable}` },
    { metric: "Wrong answers", before: baseline.wrong_answers, after: result.summary.wrong_answers },
    { metric: "Warm p95 (ms)", before: baseline.warm_p95_ms.toFixed(1), after: result.summary.warm_p95_ms.toFixed(1) },
  ].map((entry) => ({ improvement_vector: "Memory matcher comparison", scenario: name, context: `${platform()}; isolated fixture stores; ${report.inputs.repeats} repeats; model worker uses 2 CPU threads`, ...entry, delta: "N/A", correctness: result.summary.qualifies ? "Qualified" : "Rejected", evidence: join(out, "report.json") })));
}

export async function compare(args) {
  const started = new Date().toISOString();
  const out = join(root, "reports/qol-memory/comparison", started.replaceAll(":", "-"));
  mkdirSync(out, { recursive: true });
  const report = { name: "qol-memory-matcher-comparison", started_at: started, status: "failed", inputs: {}, artifacts: {}, commands: [], next: [] };
  try {
    const settings = options([...args]);
    const proof = sourceProof();
    report.inputs = { ...settings, platform: platform(), arch: arch(), cpu: cpus()[0]?.model, available_parallelism: availableParallelism(), model_threads: 2, retrieval_k: 3, acceptance, source_sha256: proof, models: readJson(join(directory, "models.json")) };
    report.inputs.source_commit = runCommand(root, out, report, "source-commit", ["git", "rev-parse", "HEAD"]).trim();
    writeJson(join(out, "experiment.json"), report.inputs);
    const paths = await prepareModels(report.inputs.models, settings.cache, settings.offline);
    const baselineBin = await buildWorker(root, out, report, "matcher-baseline", ["-p", "qol-memory", "--example", "matcher-baseline"]);
    const modelBin = await buildWorker(root, out, report, "compare", ["--manifest-path", "docs/research/qol-memory/tier1/Cargo.toml", "--bin", "compare", "--target-dir", "target/qol-memory-compare"]);
    const runSplit = (dataset) => {
      const name = dataset.split;
      const input = workerInput(dataset, settings.repeats);
      writeJson(join(out, `${name}-input.json`), input);
      const baseline = JSON.parse(runCommand(root, out, report, `${name}-baseline`, [baselineBin, join(out, `${name}-store`)], input));
      writeJson(join(out, `${name}-baseline.json`), baseline);
      const modelInput = workerInput(dataset, settings.repeats, baseline.results);
      writeJson(join(out, `${name}-model-input.json`), modelInput);
      const models = JSON.parse(runCommand(root, out, report, `${name}-models`, [modelBin, paths.embedding, paths.equivalence], modelInput));
      writeJson(join(out, `${name}-models.json`), models);
      report.artifacts[name] = { baseline: join(out, `${name}-baseline.json`), models: join(out, `${name}-models.json`) };
      return modelResults(baseline, models);
    };
    const development = validateDataset(readJson(join(fixtures, "development.json")), "development");
    const developmentResults = runSplit(development);
    const policies = Object.fromEntries(["embedding", "hybrid_equivalence"].map((name) => [name, calibrate(development, developmentResults[name])]));
    const frozen = { frozen_at: new Date().toISOString(), source_split: "development", policies, acceptance };
    writeJson(join(out, "frozen-policy.json"), frozen);
    report.artifacts.policy = { path: join(out, "frozen-policy.json"), sha256: await hashFile(join(out, "frozen-policy.json")) };
    report.development = summarize(development, developmentResults, policies);
    process.stdout.write(`Thresholds frozen: ${JSON.stringify(policies)}\n`);
    const heldout = validateDataset(readJson(join(fixtures, "heldout.json")), "heldout");
    validateSplitSeparation(development, heldout);
    report.heldout = summarize(heldout, runSplit(heldout), policies);
    if (JSON.stringify(sourceProof()) !== JSON.stringify(proof)) throw new Error("Evaluation sources changed during the run");
    for (const name of ["matcher-baseline", "compare"]) {
      const artifact = report.artifacts[name];
      if (await hashFile(artifact.path) !== artifact.sha256) throw new Error("Evaluation executable changed during the run");
    }
    report.decision = { qualified: Object.entries(report.heldout).filter(([, value]) => value.summary.qualifies).map(([name]) => name) };
    report.metrics = metrics(report, out);
    report.status = "pass";
    report.next = report.decision.qualified.length ? ["Validate qualified candidates against a larger independently labelled corpus and guest launcher runtime before integration."] : ["No candidate meets the frozen gate. Inspect wrong_answer and miss rows; add evidence-aware constraint verification before another development round. Preserve this heldout run as an audit result."];
    for (const [name, { summary }] of Object.entries(report.heldout)) {
      process.stdout.write(`${name}: ${summary.correct_answers}/${summary.answerable} correct answers; ${summary.wrong_answers} wrong; ${summary.misses} misses; p95 ${summary.warm_p95_ms.toFixed(1)} ms\n`);
    }
    process.stdout.write(`Qualified: ${report.decision.qualified.join(", ") || "none"}\n`);
  } catch (error) {
    report.error = String(error);
    report.next = ["Resolve the failed workflow step and rerun the comparison; no candidate is accepted from an incomplete run."];
    process.stderr.write(`${error}\n`);
    process.exitCode = 1;
  }
  report.finished_at = new Date().toISOString();
  writeJson(join(out, "report.json"), report);
  process.stdout.write(`${join(out, "report.json")}\n`);
}
