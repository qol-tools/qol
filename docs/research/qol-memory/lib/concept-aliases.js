import { readFileSync } from "node:fs";
import { tokens } from "./retrieval.js";

export const ALIAS_CAP = 4;
const TERM_RE = /^[a-z0-9]{2,}$/;

export function loadAliases(path) {
  try {
    const raw = JSON.parse(readFileSync(path, "utf8"));
    if (!raw || raw.schema !== 1) throw new Error("schema must be 1");
    const map = new Map();
    for (const [term, exps] of Object.entries(raw.aliases || {})) {
      const flat = [];
      for (const e of exps) {
        for (const t of tokens(e)) {
          if (flat.length >= ALIAS_CAP) break;
          flat.push(t);
        }
      }
      map.set(term, flat);
    }
    return map;
  } catch (e) {
    console.error(`concept-aliases: load failed for ${path}: ${e.message}; using empty alias map`);
    return new Map();
  }
}

export function expandTokens(list, map) {
  const out = [];
  for (const t of list) {
    const ex = map.get(t);
    if (!ex) {
      out.push(t);
      continue;
    }
    for (const e of ex) out.push(e);
  }
  return out;
}

export function expandTokensKeep(list, map) {
  const out = [];
  for (const t of list) {
    out.push(t);
    const ex = map.get(t);
    if (!ex) continue;
    for (const e of ex) out.push(e);
  }
  return out;
}

export function validate(path) {
  const errors = [];
  let raw;
  try {
    raw = JSON.parse(readFileSync(path, "utf8"));
  } catch (e) {
    return { ok: false, errors: [`unreadable: ${e.message}`] };
  }
  if (!raw || raw.schema !== 1) errors.push(`schema must be 1, found ${raw && raw.schema}`);
  if (!raw.aliases || typeof raw.aliases !== "object" || Array.isArray(raw.aliases)) {
    errors.push("aliases must be an object of term -> term arrays");
  } else {
    for (const [term, exps] of Object.entries(raw.aliases)) {
      if (!TERM_RE.test(term)) errors.push(`alias term "${term}" is not a valid token`);
      if (!Array.isArray(exps)) {
        errors.push(`alias "${term}" must map to an array of terms`);
        continue;
      }
      for (const e of exps) {
        if (typeof e !== "string") errors.push(`alias "${term}" has a non-string expansion`);
        else if (!TERM_RE.test(e)) errors.push(`alias "${term}" expansion "${e}" is not a valid token`);
      }
    }
  }
  return { ok: errors.length === 0, errors };
}
