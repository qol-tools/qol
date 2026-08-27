#!/usr/bin/env node
import { mkdirSync, writeFileSync, renameSync } from "node:fs";
import { join, resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { qolMemoryStore } from "./lib/store-path.js";
import { walkSkills, loadGlossary } from "./lib/skills-pool.js";

const BASE = dirname(fileURLToPath(import.meta.url));
const ARGS = process.argv.slice(2);
const pick = (flag, def) => {
  const i = ARGS.indexOf(flag);
  return i >= 0 && ARGS[i + 1] ? ARGS[i + 1] : def;
};
const STORE_ROOT = resolve(pick("--store", qolMemoryStore()));
const GLOSSARY_PATH = pick("--glossary", join(BASE, "..", "..", "..", "plugins", "qol-memory", "assets", "skills-glossary.json"));
const SKILLS_ROOT = resolve(
  pick("--skills-root", process.env.QOL_MEMORY_SKILLS_ROOT || join(BASE, "..", "..", "..", "..", "..", "..", "qol-skills"))
);
const SKILLS_DIR = join(STORE_ROOT, "skills");

const started = Date.now();
const glossary = loadGlossary(GLOSSARY_PATH);
const { skills, errors, head, dirty } = walkSkills(SKILLS_ROOT, GLOSSARY_PATH);
const index = {
  schema: 1,
  walked_at: Date.now(),
  root: SKILLS_ROOT,
  repo: { name: "qol-skills", head, dirty },
  skills,
};
const tmp = join(SKILLS_DIR, "index.json.tmp");
mkdirSync(SKILLS_DIR, { recursive: true });
writeFileSync(tmp, JSON.stringify(index));
renameSync(tmp, join(SKILLS_DIR, "index.json"));

const report = {
  name: "qol-memory skills pool (metadata index)",
  schemaVersion: 1,
  started_at: new Date(started).toISOString(),
  finished_at: new Date().toISOString(),
  status: errors.length ? "degraded" : skills.length ? "pass" : "fail",
  inputs: { skillsRoot: SKILLS_ROOT, excerptChars: 600, glossary: glossary ? Object.keys(glossary).length : 0 },
  artifacts: { index: "skills/index.json", report: "skills/report.json" },
  commands: [`node ${join(BASE, "skills.mjs")} --skills-root ${SKILLS_ROOT}`],
  stats: { skills: skills.length, sections: skills.reduce((n, s) => n + s.sections.length, 0), bytes: skills.reduce((n, s) => n + s.bytes, 0), errors: errors.length, head, dirty: !!dirty, aliased: skills.filter((s) => s.aliases && s.aliases.length).length },
  errors: errors.slice(0, 5),
};
writeFileSync(join(SKILLS_DIR, "report.json"), JSON.stringify(report, null, 2));
console.log(`skills: ${skills.length} (${report.stats.sections} sections, ${(report.stats.bytes / 1024).toFixed(0)}KB) | head ${head ? head.slice(0, 8) : "n/a"}${dirty ? " (dirty)" : ""} | ${report.status}`);
if (errors.length) console.log(`errors: ${errors.length} (first: ${errors[0]})`);
console.log(`index: ${join(SKILLS_DIR, "index.json")}`);
