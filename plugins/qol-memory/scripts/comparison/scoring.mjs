export const acceptance = Object.freeze({ max_wrong_answers: 0, min_answer_coverage: 0.9, max_warm_p95_ms: 200 });

export function validateDataset(dataset, split) {
  if (dataset.schema !== 1 || dataset.split !== split || !dataset.facts?.length || !dataset.cases?.length) {
    throw new Error(`Invalid ${split} dataset`);
  }
  const facts = new Set(dataset.facts.map((fact) => fact.id));
  const cases = new Set(dataset.cases.map((entry) => entry.id));
  if (facts.size !== dataset.facts.length || cases.size !== dataset.cases.length) throw new Error("Duplicate IDs");
  for (const fact of dataset.facts) {
    if (![fact.id, fact.question, fact.answer].every((value) => typeof value === "string" && value.trim())) throw new Error("Incomplete fact");
  }
  for (const entry of dataset.cases) {
    if (!entry.query?.trim() || !entry.category || (entry.expected !== null && !facts.has(entry.expected))) throw new Error(`Invalid case ${entry.id}`);
  }
  return dataset;
}

export function validateSplitSeparation(development, heldout) {
  for (const field of ["id", "question"]) {
    const seen = new Set(development.facts.map((fact) => fact[field].toLowerCase().trim()));
    if (heldout.facts.some((fact) => seen.has(fact[field].toLowerCase().trim()))) throw new Error(`Fact ${field} leaked across splits`);
  }
  const queries = new Set(development.cases.map((entry) => entry.query.toLowerCase().trim()));
  if (heldout.cases.some((entry) => queries.has(entry.query.toLowerCase().trim()))) throw new Error("Query leaked across splits");
}

export function workerInput(dataset, repeats, lexical = []) {
  const ranks = new Map(lexical.map((row) => [row.id, row.lexical]));
  return {
    repeats,
    facts: dataset.facts.map(({ id, question, answer }) => ({ id, question, answer })),
    queries: dataset.cases.map(({ id, query }) => ({ id, query, lexical: ranks.get(id) ?? [] })),
  };
}

export function predict(scores, threshold) {
  const accepted = [...new Set(scores.filter((entry) => entry.score >= threshold).map((entry) => entry.key))];
  return accepted.length === 1 ? accepted[0] : null;
}

function percentile(samples, fraction) {
  if (!samples.length) throw new Error("Missing latency samples");
  const sorted = [...samples].sort((a, b) => a - b);
  return sorted[Math.ceil(sorted.length * fraction) - 1];
}

export function evaluate(dataset, results, threshold = null) {
  const byId = new Map(results.map((row) => [row.id, row]));
  if (byId.size !== dataset.cases.length || byId.size !== results.length) throw new Error("Missing or duplicate predictions");
  const factIds = new Set(dataset.facts.map((fact) => fact.id));
  const rows = dataset.cases.map((entry) => {
    const raw = byId.get(entry.id);
    if (!raw?.samples_ms?.length || raw.samples_ms.some((ms) => !Number.isFinite(ms) || ms < 0)) throw new Error(`Missing timing for ${entry.id}`);
    if (threshold !== null && (!raw.scores?.length || raw.scores.some((score) => !factIds.has(score.key) || !Number.isFinite(score.score)))) throw new Error(`Invalid scores for ${entry.id}`);
    const actual = threshold === null ? raw.answer : predict(raw.scores, threshold);
    if (actual !== null && !factIds.has(actual)) throw new Error(`Unknown answer for ${entry.id}`);
    const outcome = actual === null ? (entry.expected === null ? "correct_abstention" : "miss") : (actual === entry.expected ? "correct_answer" : "wrong_answer");
    return { ...entry, actual, outcome, samples_ms: raw.samples_ms, retrieved: raw.retrieved ?? raw.lexical ?? [] };
  });
  const count = (outcome) => rows.filter((row) => row.outcome === outcome).length;
  const correct = count("correct_answer");
  const wrong = count("wrong_answer");
  const answerable = rows.filter((row) => row.expected !== null);
  const summary = {
    total: rows.length,
    answerable: answerable.length,
    correct_answers: correct,
    wrong_answers: wrong,
    correct_abstentions: count("correct_abstention"),
    misses: count("miss"),
    precision: correct + wrong ? correct / (correct + wrong) : null,
    answer_coverage: correct / Math.max(1, answerable.length),
    retrieval_recall: answerable.filter((row) => row.retrieved.includes(row.expected)).length / Math.max(1, answerable.length),
    warm_p50_ms: percentile(rows.flatMap((row) => row.samples_ms), 0.5),
    warm_p95_ms: percentile(rows.flatMap((row) => row.samples_ms), 0.95),
  };
  summary.qualifies = summary.wrong_answers <= acceptance.max_wrong_answers
    && summary.answer_coverage >= acceptance.min_answer_coverage
    && summary.warm_p95_ms <= acceptance.max_warm_p95_ms;
  return { summary, rows };
}

export function calibrate(dataset, results) {
  if (dataset.split !== "development") throw new Error("Threshold calibration requires the development split");
  const values = results.flatMap((row) => row.scores.map((score) => score.score));
  const thresholds = [...new Set([Math.max(1, ...values) + 0.000001, ...values])].sort((a, b) => b - a);
  let winner = null;
  for (const threshold of thresholds) {
    const { summary } = evaluate(dataset, results, threshold);
    if (summary.wrong_answers !== 0) continue;
    if (!winner || summary.correct_answers > winner.correct_answers) winner = { threshold, correct_answers: summary.correct_answers };
  }
  if (!winner) throw new Error("No valid abstention policy");
  return winner;
}
