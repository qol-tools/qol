import assert from "node:assert/strict";
import test from "node:test";
import { runQualifies, scoreCase, verifiedActual } from "./contract.mjs";

const answered = (answer_key, answer_rows = 1) => ({ verdict: "answered", outcome: "answered", answer_key, answer_rows });
const withheld = (verdict, outcome, answer_rows = 0) => ({ verdict, outcome, answer_key: null, answer_rows });

test("answer cases match only on an answered verdict with the expected key", () => {
  assert.deepEqual(scoreCase("answer", "a", answered("a")), { match: true, wrong_answer: false });
  assert.deepEqual(scoreCase("answer", "a", withheld("no-memory", "abstain")), { match: false, wrong_answer: false });
  assert.deepEqual(scoreCase("answer", "a", answered("b")), { match: false, wrong_answer: true });
  assert.deepEqual(scoreCase("answer", undefined, answered("b")), { match: true, wrong_answer: false });
});

test("abstain cases require no answer and no answer rows", () => {
  assert.deepEqual(scoreCase("abstain", undefined, withheld("no-memory", "abstain")), { match: true, wrong_answer: false });
  assert.deepEqual(scoreCase("abstain", undefined, answered("a")), { match: false, wrong_answer: true });
  assert.deepEqual(scoreCase("abstain", undefined, withheld("candidates", "clarify", 2)), { match: false, wrong_answer: false });
});

test("clarify cases accept any withheld answer", () => {
  assert.deepEqual(scoreCase("clarify", undefined, withheld("candidates", "clarify")), { match: true, wrong_answer: false });
  assert.deepEqual(scoreCase("clarify", undefined, withheld("no-memory", "abstain")), { match: true, wrong_answer: false });
  assert.deepEqual(scoreCase("clarify", undefined, answered("a")), { match: false, wrong_answer: false });
});

test("qualify cases are excluded from the binary totals", () => {
  assert.deepEqual(scoreCase("qualify", undefined, answered("a")), { match: null, wrong_answer: false });
  assert.deepEqual(scoreCase("qualify", undefined, withheld("no-memory", "abstain")), { match: null, wrong_answer: false });
});

test("unknown expectations are rejected", () => {
  assert.throws(() => scoreCase("pass", undefined, answered("a")), /Unknown expected outcome/);
});

test("the report qualifies only with zero mismatches and zero wrong answers", () => {
  const acceptance = (results) => {
    const binary = results.filter((row) => row.expected !== "qualify");
    const mismatches = binary.filter((row) => row.match === false);
    const wrongAnswers = results.filter((row) => row.wrong_answer);
    return {
      binary_cases: binary.length,
      matches: binary.length - mismatches.length,
      mismatches: mismatches.length,
      wrong_answers: wrongAnswers.length,
      qualifies: mismatches.length === 0 && wrongAnswers.length === 0,
    };
  };
  const rows = (expected, verdict, expectedKey, actualKey) => {
    const actual = verdict === "answered" ? answered(actualKey) : withheld(verdict, "abstain");
    return { expected, ...scoreCase(expected, expectedKey, actual) };
  };
  assert.deepEqual(
    acceptance([rows("answer", "answered", "a", "a"), rows("clarify", "candidates"), rows("abstain", "no-memory"), rows("qualify", "answered", "a", "a")]),
    { binary_cases: 3, matches: 3, mismatches: 0, wrong_answers: 0, qualifies: true },
  );
  assert.equal(acceptance([rows("clarify", "answered")]).qualifies, false);
  assert.equal(acceptance([rows("answer", "answered", "a", "b")]).wrong_answers, 1);
  assert.equal(acceptance([rows("abstain", "answered")]).wrong_answers, 1);
});

test("the verified actual keeps a deterministic answer and adopts the runtime verdict otherwise", () => {
  assert.deepEqual(verifiedActual(answered("a"), { verdict: "withheld", answer_key: null }), { verdict: "answered", answer_key: "a", answer_rows: 1 });
  assert.deepEqual(verifiedActual(withheld("no-memory", "abstain"), { verdict: "answered", answer_key: "b" }), { verdict: "answered", answer_key: "b", answer_rows: 1 });
  assert.deepEqual(verifiedActual(withheld("no-memory", "abstain"), { verdict: "withheld", answer_key: null }), { verdict: "withheld", answer_key: null, answer_rows: 0 });
});

test("verified actuals score with the same scoreCase rules", () => {
  const fromRuntime = (expected, expectedKey, deterministic, runtime) => scoreCase(expected, expectedKey, verifiedActual(deterministic, runtime));
  assert.deepEqual(fromRuntime("answer", "a", answered("a"), { verdict: "withheld", answer_key: null }), { match: true, wrong_answer: false });
  assert.deepEqual(fromRuntime("answer", "a", withheld("no-memory", "abstain"), { verdict: "answered", answer_key: "a" }), { match: true, wrong_answer: false });
  assert.deepEqual(fromRuntime("abstain", undefined, withheld("no-memory", "abstain"), { verdict: "answered", answer_key: "b" }), { match: false, wrong_answer: true });
  assert.deepEqual(fromRuntime("abstain", undefined, withheld("no-memory", "abstain"), { verdict: "withheld", answer_key: null }), { match: true, wrong_answer: false });
  assert.deepEqual(fromRuntime("clarify", undefined, withheld("candidates", "clarify"), { verdict: "withheld", answer_key: null }), { match: true, wrong_answer: false });
});

test("the run qualifies when either stage qualifies", () => {
  const pass = { qualifies: true };
  const fail = { qualifies: false };
  assert.equal(runQualifies(fail, undefined), false);
  assert.equal(runQualifies(fail, fail), false);
  assert.equal(runQualifies(fail, pass), true);
  assert.equal(runQualifies(pass, undefined), true);
  assert.equal(runQualifies(pass, fail), true);
});
