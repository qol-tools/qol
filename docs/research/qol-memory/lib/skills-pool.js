import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync, readdirSync, statSync, existsSync } from "node:fs";
import { join, relative, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { tokens } from "./retrieval.js";

const POOL_BASE = dirname(fileURLToPath(import.meta.url));
export function loadGlossary(path) {
  try {
    const raw = JSON.parse(readFileSync(path, "utf8"));
    return raw.aliases || {};
  } catch {
    return null;
  }
}

const STEM_MAP = {
  built: "build",
  building: "build",
  fixed: "fix",
  fixing: "fix",
  formatt: "format",
  wrote: "write",
  writing: "write",
  ran: "run",
  running: "run",
  done: "do",
  does: "do",
  made: "make",
  making: "make",
  used: "use",
  using: "use",
  tested: "test",
  testing: "test",
};

export function poolTokens(text) {
  return tokens(text).map((t) => STEM_MAP[t] || t);
}

function gitHead(root) {
  const r = spawnSync("git", ["-C", root, "rev-parse", "HEAD"], { encoding: "utf8", timeout: 5000 });
  return r.status === 0 ? r.stdout.trim() : null;
}

function gitDirty(root) {
  const r = spawnSync("git", ["-C", root, "status", "--porcelain"], { encoding: "utf8", timeout: 5000 });
  return r.status === 0 ? r.stdout.trim().length > 0 : null;
}

export function walkSkills(root, glossaryPath) {
  const glossary = glossaryPath ? loadGlossary(glossaryPath) : null;
  const files = [];
  const plugins = join(root, "plugins");
  if (!existsSync(plugins)) return { files: [], errors: [`no plugins dir under ${root}`] };
  for (const plugin of readdirSync(plugins)) {
    const skillsDir = join(plugins, plugin, "skills");
    if (!existsSync(skillsDir)) continue;
    for (const skill of readdirSync(skillsDir)) {
      const f = join(skillsDir, skill, "SKILL.md");
      if (existsSync(f) && statSync(f).isFile()) files.push(f);
    }
  }
  const errors = [];
  const skills = [];
  for (const f of files.sort()) {
    try {
      const raw = readFileSync(f, "utf8");
      const fm = raw.match(/^---\n([\s\S]*?)\n---\n?/);
      const meta = {};
      if (fm) {
        const lines = fm[1].split("\n");
        for (let i = 0; i < lines.length; i++) {
          const m = lines[i].match(/^([a-zA-Z_]+):\s*(.*)$/);
          if (!m) continue;
          const key = m[1];
          let value = m[2].trim();
          const fold = value === ">" || value === "|" || value === ">-" || value === "|-" || value === ">+" || value === "|+";
          if (fold) {
            const keep = value.startsWith("|");
            const parts = [];
            while (i + 1 < lines.length && (lines[i + 1].trim() === "" || /^\s/.test(lines[i + 1]))) {
              parts.push(lines[i + 1].trim());
              i++;
            }
            if (keep) {
              value = parts.join("\n");
            } else {
              const segments = [];
              let current = [];
              for (const p of parts) {
                if (p === "") {
                  if (current.length) segments.push(current.join(" "));
                  current = [];
                } else {
                  current.push(p);
                }
              }
              if (current.length) segments.push(current.join(" "));
              value = segments.join("\n");
            }
          }
          meta[key] = value;
        }
      }
      const body = fm ? raw.slice(fm[0].length) : raw;
      const titleMatch = body.match(/^#\s+(.+)$/m);
      const parts = f.split(/[\\/]/);
      const plugin = parts[parts.length - 4];
      const skill = parts[parts.length - 2];
      const title = titleMatch ? titleMatch[1].trim() : skill;
      const sections = [];
      let inFence = false;
      for (const line of body.split("\n")) {
        if (line.trim().startsWith("```")) inFence = !inFence;
        if (!inFence && line.startsWith("## ")) {
          sections.push({ h: line.slice(3).trim(), lead: "" });
        } else if (sections.length && !line.startsWith("# ")) {
          sections[sections.length - 1].lead += line + "\n";
        }
      }
      for (const s of sections) s.lead = s.lead.trim().slice(0, 600);
      const rel = relative(root, f).split("\\").join("/");
      const hash = createHash("sha256").update(raw).digest("hex").slice(0, 16);
      const id = `${plugin}/${skill}`;
      skills.push({
        id,
        name: meta.name || skill,
        description: meta.description || "",
        title,
        rel,
        hash,
        bytes: raw.length,
        sections,
        aliases: glossary && glossary[id] ? glossary[id] : [],
      });
    } catch (e) {
      errors.push(`${f}: ${e.message}`);
    }
  }
  const head = gitHead(root);
  const dirty = gitDirty(root);
  return { skills, errors, head, dirty };
}

export function buildMetaDoc(skill) {
  const headers = skill.sections.map((s) => s.h).join(" ");
  const leads = skill.sections.map((s) => s.lead).join(" ");
  const aliases = skill.aliases && skill.aliases.length ? skill.aliases.join(" ") : "";
  return `${skill.name} ${skill.title} ${skill.description} ${headers} ${leads} ${aliases}`;
}

export function probeFresh(index, root) {
  if (!index || !index.walked_at) return "not-indexed";
  if (!existsSync(root)) return "unavailable";
  let changed = 0;
  for (const s of index.skills || []) {
    const p = join(root, s.rel);
    if (!existsSync(p)) {
      changed++;
      continue;
    }
    const st = statSync(p);
    if (st.mtimeMs > index.walked_at) changed++;
  }
  return changed ? "stale" : "fresh";
}

export function serveSection(skill, root, headerHint, cap = 2048) {
  const p = join(root, skill.rel);
  if (!existsSync(p)) return { ok: false, reason: "missing" };
  let raw;
  try {
    raw = readFileSync(p, "utf8");
  } catch (e) {
    return { ok: false, reason: `read-error: ${e.message}` };
  }
  const hash = createHash("sha256").update(raw).digest("hex").slice(0, 16);
  const sections = splitSections(raw);
  const norm = (s) => s.toLowerCase().replace(/`/g, "").trim();
  let target = null;
  if (headerHint) target = sections.find((s) => norm(s.h) === norm(headerHint));
  if (!target) target = sections.find((s) => s.text.length >= 24 && s.text.length <= 400);
  if (!target) return { ok: false, reason: "no-anchor" };
  const content = target.text.trim().slice(0, cap);
  const truncated = target.text.trim().length > cap;
  return { ok: true, content, section: target.h, truncated, hash_match: hash === skill.hash, live_hash: hash };
}

export function splitSections(raw) {
  const sections = [];
  let inFence = false;
  let cur = null;
  for (const line of raw.split("\n")) {
    if (line.trim().startsWith("```")) inFence = !inFence;
    if (!inFence && line.startsWith("## ")) {
      cur = { h: line.slice(3).trim(), text: "" };
      sections.push(cur);
    } else if (cur && !line.startsWith("# ")) {
      cur.text += line + "\n";
    }
  }
  return sections;
}

export function bestSection(skill, root, qtokens, idf, cap = 2048) {
  const p = join(root, skill.rel);
  if (!existsSync(p)) return null;
  let raw;
  try {
    raw = readFileSync(p, "utf8");
  } catch {
    return null;
  }
  const sections = splitSections(raw);
  const weights = qtokens.map((t) => (idf && idf.get(t)) || 1);
  let best = null;
  let bestScore = 0;
  for (let si = 0; si < sections.length; si++) {
    const s = sections[si];
    const contentTokens = new Set(poolTokens(s.text.slice(0, cap)));
    const headerTokens = new Set(poolTokens(s.h));
    const introPenalty = si === 0 ? 0.5 : 1;
    const score = qtokens.reduce((acc, t, i) => {
      const inContent = contentTokens.has(t) ? 1 : 0;
      const inHeader = headerTokens.has(t) ? 1 : 0;
      return acc + ((inContent ? 2 : 0) + (inHeader ? 3 : 0)) * weights[i];
    }, 0) * introPenalty;
    if (score > bestScore) {
      bestScore = score;
      best = s;
    }
  }
  return best && bestScore > 0 ? { ...best, score: bestScore } : null;
}
