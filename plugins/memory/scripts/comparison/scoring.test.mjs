import assert from "node:assert/strict";
import test from "node:test";
import { calibrate, evaluate, predict, validateSplitSeparation, workerInput } from "./scoring.mjs";

const dataset = {
  split: "development",
  facts: [{ id: "a", question: "Where is a?", answer: "/a" }, { id: "b", question: "Where is b?", answer: "/b" }],
  cases: [{ id: "yes", query: "locate a", expected: "a" }, { id: "no", query: "locate c", expected: null }],
};
const results = [
  { id: "yes", samples_ms: [10, 12], scores: [{ key: "a", score: 0.9 }, { key: "b", score: 0.2 }] },
  { id: "no", samples_ms: [11, 20], scores: [{ key: "a", score: 0.7 }, { key: "b", score: 0.1 }] },
];

test("development calibration trades coverage for zero false answers and never accepts holdout labels", () => {
  const policy = calibrate(dataset, results);
  assert.equal(policy.threshold, 0.9);
  const { summary } = evaluate(dataset, results, policy.threshold);
  assert.equal(summary.correct_answers, 1);
  assert.equal(summary.correct_abstentions, 1);
  assert.equal(summary.warm_p95_ms, 20);
  assert.throws(() => calibrate({ ...dataset, split: "heldout" }, results), /development/);
  assert.equal(evaluate(dataset, results, 0.6).summary.wrong_answers, 1);
});

test("unresolvable candidates abstain without inventing an answer", () => {
  assert.equal(predict([{ key: "a", score: 0.9 }, { key: "b", score: 0.9 }], 0.8), null);
  assert.equal(predict([{ key: "a", score: 0.9 }, { key: "a", score: 0.9 }], 0.8), "a");
  assert.equal(evaluate(dataset, results, 1.000001).summary.precision, null);
  assert.equal(evaluate(dataset, results, 1.000001).summary.qualifies, false);
});

test("missing predictions and invalid model output fail the evaluation", () => {
  assert.throws(() => evaluate(dataset, results.slice(1), 0.9), /Missing/);
  assert.throws(() => evaluate(dataset, [{ ...results[0], scores: [{ key: "unknown", score: 0.9 }] }, results[1]], 0.9), /Invalid/);
  assert.throws(() => evaluate(dataset, [{ ...results[0], samples_ms: [NaN] }, results[1]], 0.9), /timing/);
});

test("worker payloads exclude gold labels and split overlap is rejected", () => {
  assert.equal(JSON.stringify(workerInput(dataset, 3)).includes("expected"), false);
  assert.throws(() => validateSplitSeparation(dataset, dataset), /leaked/);
});
