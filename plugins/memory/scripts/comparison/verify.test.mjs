import assert from "node:assert/strict";
import test from "node:test";
import { score } from "./verify.mjs";
import { localEndpoint } from "./local-model.mjs";

const dataset = {
  facts: [{ id: "a", question: "Where is a?", answer: "/a" }],
  cases: [{ id: "yes", query: "locate a", expected: "a" }, { id: "no", query: "locate b", expected: null }],
};
const rows = [
  { id: "yes", answer: "a", samples_ms: [1], initial_ms: 2, completion_ms: 800 },
  { id: "no", answer: null, samples_ms: [1], initial_ms: 2, completion_ms: 900 },
];

test("runtime qualification requires accuracy, immediate retrieval, cached response and completion latency", () => {
  const good = score(dataset, rows);
  assert.equal(good.qualifies, true);
  assert.equal(good.cached.summary.retrieval_recall, null);
  assert.equal(good.first_completion_ms, 800);
  assert.equal(good.max_completion_ms, 900);
  for (const mutation of [
    { answer: null },
    { initial_ms: 201 },
    { samples_ms: [201] },
    { completion_ms: 10_001 },
  ]) {
    assert.equal(score(dataset, [{ ...rows[0], ...mutation }, rows[1]]).qualifies, false);
  }
  assert.equal(score(dataset, [rows[0], { ...rows[1], answer: "a" }]).qualifies, false);
});

test("evaluation endpoints accept local origins and reject external or redirecting addresses", () => {
  assert.equal(localEndpoint("http://127.0.0.1:11434/"), "http://127.0.0.1:11434");
  for (const endpoint of ["https://127.0.0.1:11434", "http://example.com:11434", "http://user@127.0.0.1:11434", "http://127.0.0.1:11434/api", "http://127.0.0.1:11434?remote"]) {
    assert.throws(() => localEndpoint(endpoint), /loopback/);
  }
});
