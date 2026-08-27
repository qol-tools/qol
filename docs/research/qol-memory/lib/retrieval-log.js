import { createHash } from "node:crypto";
import { appendFileSync, mkdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { parseUnitsText } from "./seal.js";

export const RETRIEVAL_SCHEMA = "qol-memory-retrieval-v1";
export const CANDIDATES_SCHEMA = "qol-memory-candidates-v1";
export const RETRIEVAL_LOG_CAP = 10 * 1024 * 1024;
export const RETRIEVAL_LOG_TAIL = 1024 * 1024;
export const CANDIDATE_COOLDOWN_MS = 24 * 60 * 60 * 1000;

export function normalizeQuery(s) {
  return (s || "")
    .toLowerCase()
    .replace(/[^a-z0-9 ]/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

export function isMiss(event) {
  const v = event && event.verdict;
  return v === "no-memory" || v === "candidates";
}

export function candidateKey(normQuery) {
  return createHash("sha256").update(normQuery).digest("hex").slice(0, 16);
}

export function discriminatorCount(fact, noteTexts) {
  if (typeof fact !== "string" || !fact.length) return 0;
  return noteTexts.filter((t) => typeof t === "string" && t.includes(fact)).length;
}

export function rotateIfNeeded(logPath, cap = RETRIEVAL_LOG_CAP, tail = RETRIEVAL_LOG_TAIL) {
  let size;
  try {
    size = statSync(logPath).size;
  } catch {
    return;
  }
  if (size <= cap) return;
  if (statSync(logPath).size <= cap) return;
  const raw = readFileSync(logPath);
  const cutAt = Math.max(0, raw.length - tail);
  const prefixLen = raw.subarray(0, cutAt).lastIndexOf(10) + 1;
  writeFileSync(logPath, raw.subarray(prefixLen));
}

function correctnessOf(out, ctx) {
  const fact = ctx.fact;
  if (!fact) return null;
  if (ctx.source === "eval" && fact.startsWith("trap:")) {
    return out.verdict === "answered" ? "trapped" : "untrapped";
  }
  if (out.verdict !== "answered") return "unanswered";
  const f = normalizeQuery(fact);
  if (!f) return "correct";
  return normalizeQuery(out.answer && out.answer.text).includes(f) ? "correct" : "wrong";
}

export function appendRetrieval(out, ctx) {
  try {
    if (process.env.QOL_MEMORY_RETRIEVAL_LOG_DISABLE === "1" || ctx.noLog) return;
    const event = {
      ts: new Date().toISOString(),
      source: ctx.source || "ask-cli",
      session: ctx.session || null,
      cwd: ctx.cwd || null,
      query: out.query,
      verdict: out.verdict,
      confidence: out.confidence,
      correctness: correctnessOf(out, ctx),
      latency_ms: ctx.latencyMs,
      k: ctx.k,
      exclusion: { exclude_session: !!ctx.session, non_default_gates: !!out.non_default_gates },
      gates: out.gates,
      signals: out.signals,
      answer_key: out.answer ? out.answer.key : null,
      recalled_keys: out.recalled ? out.recalled.map((r) => r.key) : [],
      counts: out.counts,
    };
    const logPath = join(ctx.storeRoot, "retrievals.jsonl");
    mkdirSync(ctx.storeRoot, { recursive: true });
    rotateIfNeeded(logPath);
    appendFileSync(logPath, JSON.stringify(event) + "\n");
  } catch {}
}

export function countPendingCandidates(storeRoot) {
  try {
    const raw = readFileSync(join(storeRoot, "candidates.jsonl"), "utf8");
    return parseUnitsText(raw).filter((c) => c && c.status === "candidate").length;
  } catch {
    return 0;
  }
}
