import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const startedAt = new Date().toISOString();
const out = join(root, "reports/qol-memory/answers", startedAt.replaceAll(":", "-"));
mkdirSync(out, { recursive: true });
const command = ["cargo", "test", "--locked", "-p", "qol-memory", "--test", "answer_selection"];
const result = spawnSync(command[0], command.slice(1), {
  cwd: root,
  env: { ...process.env, QOL_MEMORY_EVAL_OUT: out },
  encoding: "utf8",
});
writeFileSync(join(out, "tests.log"), `${result.stdout ?? ""}${result.stderr ?? ""}${result.error ?? ""}`);
let cases = [];
try {
  cases = JSON.parse(readFileSync(join(out, "cases.json"), "utf8"));
} catch {}
const passed = result.status === 0 && cases.length > 0 && cases.every((entry) => entry.passed);
const report = {
  name: "qol-memory-answer-selection",
  started_at: startedAt,
  finished_at: new Date().toISOString(),
  status: passed ? "pass" : "failed",
  inputs: { fixture_suite: "plugins/qol-memory/tests/answer_selection.rs", platform: process.platform },
  artifacts: { cases: join(out, "cases.json"), log: join(out, "tests.log") },
  commands: [{ argv: command, exit_code: result.status, signal: result.signal }],
  paraphrases: { total: cases.length, passed: cases.filter((entry) => entry.passed).length },
  next: passed ? [] : ["Inspect tests.log and cases.json before accepting the change."],
};
const path = join(out, "report.json");
writeFileSync(path, `${JSON.stringify(report, null, 2)}\n`);
process.stdout.write(`${report.status}: ${report.paraphrases.passed}/${cases.length} paraphrase cases; negative, conflict, visibility, and daemon checks ${passed ? "passed" : "see log"}.\n${path}\n`);
process.exitCode = passed ? 0 : 1;
