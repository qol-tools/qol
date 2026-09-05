import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { buildWorker, hashFile, readJson, runCommand, writeJson } from "./artifacts.mjs";
import { withLocalModel } from "./local-model.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../../..");
const fixture = "tests/fixtures/answer-contract/cases.json";
const sources = ["examples/matcher-baseline.rs", "examples/matcher-runtime.rs", "examples/support/mod.rs"];
const expectedValues = new Set(["answer", "clarify", "abstain", "qualify"]);

function validateDataset(dataset) {
  if (dataset.schema !== 1 || !Array.isArray(dataset.cases) || dataset.cases.length === 0) {
    throw new Error("Invalid answer-contract dataset");
  }
  const ids = new Set(dataset.cases.map((entry) => entry.id));
  if (ids.size !== dataset.cases.length) throw new Error("Duplicate case IDs");
  for (const entry of dataset.cases) {
    if (!entry.id || !entry.concept || !entry.query?.trim() || !Array.isArray(entry.memories) || entry.memories.length === 0) {
      throw new Error(`Invalid case ${entry.id}`);
    }
    if (!expectedValues.has(entry.expected)) throw new Error(`Invalid expected outcome for ${entry.id}`);
    if (entry.expected_key !== undefined && !entry.memories.some((memory) => memory.id === entry.expected_key)) {
      throw new Error(`Expected key ${entry.expected_key} is not a memory of ${entry.id}`);
    }
    for (const memory of entry.memories) {
      if (!memory.id || (!memory.text && !(memory.question && memory.answer))) {
        throw new Error(`Invalid memory shape in ${entry.id}`);
      }
    }
  }
  return dataset;
}

export function scoreCase(expected, expectedKey, actual) {
  const answered = actual.verdict === "answered";
  if (expected === "qualify") return { match: null, wrong_answer: false };
  if (expected === "answer") {
    if (!answered) return { match: false, wrong_answer: false };
    if (expectedKey !== undefined && expectedKey !== null && actual.answer_key !== expectedKey) {
      return { match: false, wrong_answer: true };
    }
    return { match: true, wrong_answer: false };
  }
  if (expected === "abstain") {
    if (answered || actual.answer_rows > 0) return { match: false, wrong_answer: answered };
    return { match: true, wrong_answer: false };
  }
  if (expected === "clarify") return { match: !answered, wrong_answer: false };
  throw new Error(`Unknown expected outcome: ${expected}`);
}

export function verifiedActual(deterministic, runtime) {
  if (deterministic.verdict === "answered") {
    return { verdict: "answered", answer_key: deterministic.answer_key, answer_rows: deterministic.answer_rows ?? 1 };
  }
  const answered = runtime.verdict === "answered";
  return { verdict: runtime.verdict ?? null, answer_key: runtime.answer_key ?? null, answer_rows: answered ? 1 : 0 };
}

export function runQualifies(acceptance, verified) {
  return Boolean(acceptance?.qualifies) || Boolean(verified?.qualifies);
}

function acceptanceBlock(results, pick) {
  const binary = results.filter((row) => row.expected !== "qualify");
  const mismatches = binary.filter((row) => pick(row).match === false);
  const wrongAnswers = results.filter((row) => pick(row).wrong_answer);
  return {
    binary_cases: binary.length,
    matches: binary.length - mismatches.length,
    mismatches: mismatches.length,
    wrong_answers: wrongAnswers.length,
    qualifies: mismatches.length === 0 && wrongAnswers.length === 0,
  };
}

function answerText(memories, answerKey) {
  if (answerKey === null) return null;
  const memory = memories.find((entry) => entry.id === answerKey);
  if (!memory) return null;
  return memory.text ?? memory.answer;
}

function toActual(memories, raw) {
  const answerKey = raw.answer ?? null;
  return {
    verdict: raw.verdict ?? null,
    outcome: raw.outcome ?? null,
    reason_code: raw.reason_code ?? null,
    answer_key: answerKey,
    answer_text: answerText(memories, answerKey),
    answer_rows: raw.answer_rows ?? null,
  };
}

function runtimeOutcome(raw) {
  return {
    verdict: raw.answer === null || raw.answer === undefined ? "withheld" : "answered",
    answer_key: raw.answer ?? null,
    completion_ms: raw.completion_ms ?? null,
  };
}

function resultsMarkdown(report) {
  const cell = (value) => String(value).replaceAll("|", "\\|");
  const verifiedCell = (row) => {
    if (!row.verified_actual) return "n/a";
    return row.verified_actual.verdict === "answered" ? `answered ${row.verified_actual.answer_key ?? ""}`.trim() : "withheld";
  };
  const rows = report.results.map((row) =>
    `| ${cell(row.concept)} | ${cell(row.query)} | ${cell(row.expected)} | ${cell(row.actual.outcome)} | ${cell(row.match === null ? "qualify" : row.match ? "yes" : "no")} | ${cell(verifiedCell(row))} |`
  );
  return ["| concept | query | expected | actual outcome | match | verified |", "| --- | --- | --- | --- | --- | --- |", ...rows].join("\n") + "\n";
}

function options(args) {
  const settings = {};
  while (args.length) {
    const flag = args.shift();
    if (flag === "--verify") { settings.verify = true; continue; }
    if (flag === "--endpoint" && args[0]) { settings.endpoint = args.shift(); continue; }
    throw new Error(`Unknown or incomplete contract option: ${flag}`);
  }
  return settings;
}

async function inputHashes() {
  const paths = [fixture, ...sources];
  return Object.fromEntries(await Promise.all(paths.map(async (path) => [path, await hashFile(join(root, "plugins/qol-memory", path))])));
}

export async function contract(args) {
  const started = new Date().toISOString();
  const out = join(root, "reports/qol-memory/contract", started.replaceAll(":", "-"));
  mkdirSync(out, { recursive: true });
  const report = {
    name: "qol-memory-answer-contract",
    started_at: started,
    execution: { status: "failed", error: null },
    acceptance: null,
    acceptance_verified: null,
    qualification: [],
    results: [],
    inputs: {},
    artifacts: {},
    commands: [],
  };
  try {
    const settings = options([...args]);
    const dataset = validateDataset(readJson(join(root, "plugins/qol-memory", fixture)));
    const baseline = await buildWorker(root, out, report, "matcher-baseline", ["-p", "qol-memory", "--example", "matcher-baseline"]);
    report.inputs = {
      ...settings,
      hashes: await inputHashes(),
      matcher_baseline_sha256: report.artifacts["matcher-baseline"].sha256,
    };
    for (const entry of dataset.cases) {
      const raw = JSON.parse(runCommand(
        root, out, report, `${entry.id}-baseline`,
        [baseline, join(out, `${entry.id}-store`)],
        { facts: entry.memories, queries: [{ id: entry.id, query: entry.query }], repeats: 1 },
      )).results[0];
      const actual = toActual(entry.memories, raw);
      const score = scoreCase(entry.expected, entry.expected_key, actual);
      report.results.push({ id: entry.id, concept: entry.concept, query: entry.query, expected: entry.expected, actual, match: score.match, wrong_answer: score.wrong_answer });
    }
    if (settings.verify) {
      const runtime = await buildWorker(root, out, report, "matcher-runtime", ["-p", "qol-memory", "--example", "matcher-runtime"]);
      const profile = readJson(join(root, "plugins/qol-memory/src/verification/profile.json"));
      const settingsWithEndpoint = settings.endpoint ? { ...profile, endpoint: settings.endpoint } : profile;
      await withLocalModel(settingsWithEndpoint, out, report, async (endpoint) => {
        for (const entry of dataset.cases) {
          const raw = JSON.parse(runCommand(
            root, out, report, `${entry.id}-runtime`,
            [runtime, join(out, `${entry.id}-runtime-store`), endpoint],
            { facts: entry.memories, queries: [{ id: entry.id, query: entry.query }], repeats: 1 },
          )).rounds[0][0];
          const row = report.results.find((result) => result.id === entry.id);
          row.runtime = runtimeOutcome(raw);
          row.verified_actual = verifiedActual(row.actual, row.runtime);
          row.verified = scoreCase(entry.expected, entry.expected_key, row.verified_actual);
        }
      });
    }
    report.acceptance = acceptanceBlock(report.results, (row) => row);
    report.qualification = report.results
      .filter((row) => row.expected === "qualify")
      .map((row) => ({ id: row.id, concept: row.concept, query: row.query, expected: row.expected, actual: row.actual, runtime: row.runtime }));
    if (settings.verify) {
      report.acceptance_verified = acceptanceBlock(report.results, (row) => row.verified);
    }
    report.execution.status = "pass";
    if (!runQualifies(report.acceptance, report.acceptance_verified)) process.exitCode = 1;
    process.stdout.write(`${report.name}: ${report.acceptance.matches}/${report.acceptance.binary_cases} binary matches, ${report.acceptance.wrong_answers} wrong answers, qualifies=${report.acceptance.qualifies}\n`);
    if (report.acceptance_verified) {
      process.stdout.write(`${report.name} verified: ${report.acceptance_verified.matches}/${report.acceptance_verified.binary_cases} binary matches, ${report.acceptance_verified.wrong_answers} wrong answers, qualifies=${report.acceptance_verified.qualifies}\n`);
    }
  } catch (error) {
    report.execution = { status: "failed", error: String(error) };
    process.exitCode = 1;
    process.stderr.write(`${error}\n`);
  }
  report.finished_at = new Date().toISOString();
  writeJson(join(out, "report.json"), report);
  writeFileSync(join(out, "results.md"), resultsMarkdown(report));
  process.stdout.write(`${join(out, "report.json")}\n`);
}
