#!/usr/bin/env node
import { readFileSync, writeFileSync, mkdirSync, existsSync } from "node:fs";
import { join, resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { qolMemoryStore } from "../lib/store-path.js";
import { buildIndex, bm25Ranks } from "../lib/retrieval.js";
import { buildMetaDoc, serveSection, poolTokens, bestSection } from "../lib/skills-pool.js";

const BASE = dirname(fileURLToPath(import.meta.url));
const ARGS = process.argv.slice(2);
const pick = (flag, def) => {
  const i = ARGS.indexOf(flag);
  return i >= 0 && ARGS[i + 1] ? ARGS[i + 1] : def;
};
const STORE_ROOT = resolve(pick("--store", qolMemoryStore()));
const SKILLS_ROOT = pick("--skills-root", null);
const OUT_DIR = resolve(pick("--out", join(STORE_ROOT, "skills", "eval", new Date().toISOString().replace(/[:.]/g, "-"))));

const index = JSON.parse(readFileSync(join(STORE_ROOT, "skills", "index.json"), "utf8"));
const root = SKILLS_ROOT || index.root;
const questions = JSON.parse(readFileSync(join(BASE, "skills-questions.json"), "utf8")).questions;
const NO_GLOSSARY = ARGS.includes("--no-glossary");

const STOPWORDS = new Set(["what", "when", "where", "which", "who", "how", "do", "does", "did", "is", "are", "the", "a", "an", "to", "for", "of", "in", "on", "with", "and", "or", "me", "you", "my", "we", "i", "it", "have", "has", "be", "been", "was", "were", "many", "much", "exist", "really", "want", "should", "could", "would", "can", "work", "fix", "this", "that", "these", "those", "there", "about", "get", "make", "use", "tell", "explain"]);

const metaDocs = index.skills.map((s) => ({
  key: s.id,
  text: buildMetaDoc(NO_GLOSSARY ? { ...s, aliases: [] } : s),
}));
const idx = buildIndex(metaDocs);
const rows = [];
for (const q of questions) {
  const qt = poolTokens(q.query).filter((t) => !STOPWORDS.has(t));
  const ranked = bm25Ranks(q.query, idx, null, 5);
  const rank = ranked.findIndex((r) => r.key === q.target);
  const hit1 = rank === 0;
  const hit3 = rank >= 0 && rank < 3;
  const hit5 = rank >= 0 && rank < 5;
  let anchorHit = false;
  const skill = index.skills.find((s) => s.id === q.target);
  if (skill) {
    const best = bestSection(skill, root, qt, idx.idf);
    const served = best ? serveSection(skill, root, best.h, 2048) : serveSection(skill, root, null, 2048);
    anchorHit = served.ok && (served.content || "").toLowerCase().includes(q.answer_substr.toLowerCase());
  }
  rows.push({ id: q.id, query: q.query, target: q.target, rank: rank < 0 ? null : rank + 1, hit1, hit3, hit5, anchor_hit: anchorHit });
}

const stats = {
  n: rows.length,
  hit1: rows.filter((r) => r.hit1).length,
  hit3: rows.filter((r) => r.hit3).length,
  hit5: rows.filter((r) => r.hit5).length,
  anchor_hit: rows.filter((r) => r.anchor_hit).length,
};
const status = stats.hit5 / stats.n >= 0.75 && stats.anchor_hit / stats.n >= 0.5 ? "pass" : "degraded";
const report = {
  name: "qol-memory skills recall eval",
  schemaVersion: 1,
  started_at: new Date().toISOString(),
  status,
  inputs: { skillsRoot: root, head: index.repo ? index.repo.head : null, noGlossary: NO_GLOSSARY },
  stats,
  glossary: glossaryFlags(index, rows),
  rows,
};

function glossaryFlags(index, rows) {
  const flags = [];
  const aliasedTargets = new Set(rows.filter((r) => {
    const s = index.skills.find((x) => x.id === r.target);
    return s && s.aliases && s.aliases.length;
  }).map((r) => r.target));
  for (const s of index.skills) {
    if (!s.aliases || !s.aliases.length) continue;
    const hasQuestion = aliasedTargets.has(s.id);
    const descLower = (s.description || "").toLowerCase();
    const redundant = s.aliases.filter((a) => descLower.includes(a.toLowerCase()));
    const dangling = !existsSync(join(root, s.rel));
    flags.push({ id: s.id, aliases: s.aliases.length, hasQuestion, redundant: redundant.length, dangling });
  }
  return flags;
}
mkdirSync(OUT_DIR, { recursive: true });
writeFileSync(join(OUT_DIR, "report.json"), JSON.stringify(report, null, 2));
console.log(`skills eval: hit@1 ${stats.hit1}/${stats.n} hit@3 ${stats.hit3}/${stats.n} hit@5 ${stats.hit5}/${stats.n} anchor ${stats.anchor_hit}/${stats.n} | ${status}`);
for (const r of rows) {
  if (!r.hit3) console.log(`  MISS ${r.id} :: ${r.query} (target ${r.target}${r.rank ? ` rank ${r.rank}` : " not in top5"})`);
}
console.log(`report: ${OUT_DIR}/report.json`);
