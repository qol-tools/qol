#!/usr/bin/env node
import { mkdtempSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { walkSkills } from "./lib/skills-pool.js";

let fails = 0;
function check(name, cond, extra = "") {
  if (cond) {
    console.log(`pass ${name}`);
  } else {
    fails++;
    console.log(`FAIL ${name}${extra ? " :: " + extra : ""}`);
  }
}

function repo() {
  const root = mkdtempSync(join(tmpdir(), "qol-memory-skills-"));
  mkdirSync(join(root, "plugins", "alpha", "skills", "folded"), { recursive: true });
  mkdirSync(join(root, "plugins", "alpha", "skills", "folded-blank"), { recursive: true });
  mkdirSync(join(root, "plugins", "alpha", "skills", "plain"), { recursive: true });
  writeFileSync(
    join(root, "plugins", "alpha", "skills", "folded", "SKILL.md"),
    "---\nname: folded\ndescription: >\n  First line of the folded description.\n  Second line, joined with a space.\n---\n\n# folded\n\nBody.\n",
  );
  writeFileSync(
    join(root, "plugins", "alpha", "skills", "folded-blank", "SKILL.md"),
    "---\nname: folded-blank\ndescription: >\n  First paragraph of the folded description.\n\n  Second paragraph, kept after the blank line.\n---\n\n# folded-blank\n\nBody.\n",
  );
  writeFileSync(
    join(root, "plugins", "alpha", "skills", "plain", "SKILL.md"),
    "---\nname: plain\ndescription: Single-line description.\n---\n\n# plain\n\nBody.\n",
  );
  return root;
}

const index = walkSkills(repo(), null);
check("three skills walked", index.skills.length === 3, `got ${index.skills.length}`);
const folded = index.skills.find((s) => s.id === "alpha/folded");
const foldedBlank = index.skills.find((s) => s.id === "alpha/folded-blank");
const plain = index.skills.find((s) => s.id === "alpha/plain");
check("folded description not the indicator", folded && folded.description !== ">", JSON.stringify(folded && folded.description));
check(
  "folded description joins continuation lines with a space",
  folded && folded.description === "First line of the folded description. Second line, joined with a space.",
  folded && folded.description,
);
check(
  "folded description keeps the text after a blank line",
  foldedBlank && foldedBlank.description === "First paragraph of the folded description.\nSecond paragraph, kept after the blank line.",
  foldedBlank && foldedBlank.description,
);
check("plain description unchanged", plain && plain.description === "Single-line description.", plain && plain.description);

process.exit(fails ? 1 : 0);
