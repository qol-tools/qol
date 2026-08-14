#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const BASE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(tmpdir(), "qol-memory-recency");
rmSync(ROOT, { recursive: true, force: true });

let failed = 0;
function check(cond, label) {
  if (!cond) {
    failed++;
    console.error(`FAIL ${label}`);
  } else {
    console.log(`pass ${label}`);
  }
}

const FILLER_WORDS = ["alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india", "juliet", "kilo", "lima", "mike", "november", "oscar", "papa", "quebec", "romeo", "sierra", "tango", "uniform", "victor", "whiskey", "xray", "yankee", "zulu", "amber", "bronze", "cobalt", "denim", "emerald", "frost", "garnet", "hazel", "indigo", "jade", "khaki", "lilac", "mauve", "navy", "olive", "pearl", "quartz", "ruby", "silver", "topaz", "umber", "violet", "wheat", "xenon"];

function filler(i) {
  const words = [];
  for (let k = 0; k < 14; k++) words.push(FILLER_WORDS[(i * 31 + k * 17) % FILLER_WORDS.length]);
  return { key: "f" + i, cls: "command", text: "command " + words.join(" "), source_key: "k-f" + i, source_ts: "2026-08-14T09:00:00.000Z", source_kind: "artifact" };
}

function note(key, cls, text, ts) {
  return { key, cls, text, source_key: "k-" + key, source_ts: ts, source_kind: "decision-deter" };
}

function sandbox(name, notes) {
  const root = join(ROOT, name);
  mkdirSync(join(root, "notes", "2026-08-15T00-00-00-000Z"), { recursive: true });
  writeFileSync(join(root, "notes", "2026-08-15T00-00-00-000Z", "notes.jsonl"), notes.map((n) => JSON.stringify(n)).join("\n") + "\n");
  writeFileSync(
    join(root, "units.jsonl"),
    [
      { key: "u1", source: "test", session: "recency-sess", cwd: "/tmp", kind: "user", ts: "2026-08-13T00:00:00.000Z", text: "ordinary recollection of the corpus assembly order" },
      { key: "u2", source: "test", session: "recency-sess", cwd: "/tmp", kind: "user", ts: "2026-08-13T00:01:00.000Z", text: "the archive holds every transcript in chronological sequence" },
    ].map((u) => JSON.stringify(u)).join("\n") + "\n"
  );
  return root;
}

function ask(store) {
  const r = spawnSync("node", [join(BASE, "ask.mjs"), "what is the widget margin policy and the border width", "--brief"], {
    encoding: "utf8",
    timeout: 120000,
    env: { ...process.env, QOL_MEMORY_STORE: store },
  });
  if (r.status !== 0) return { error: r.stderr || r.stdout };
  return JSON.parse(r.stdout);
}

const OLD_HEAD = "Decision: the widget margin policy is set to forty percent";
const OLD = note("old", "decision", OLD_HEAD + " | the border width is two pixels and the fill color is azure, recorded mid-review", "2026-08-14T10:00:00.000Z");
const NEW = note("new", "decision", OLD_HEAD + " | final call", "2026-08-14T11:00:00.000Z");
const UNREL = note("unrel", "decision", "Decision: the teal falcon marker review is postponed | margin and policy notes deferred", "2026-08-14T12:00:00.000Z");
const FILLERS = [];
for (let i = 0; i < 120; i++) FILLERS.push(filler(i));

{
  const store = sandbox("a", [OLD, NEW, ...FILLERS]);
  const d = ask(store);
  check(!d.error, "A ask.mjs runs on the sandbox");
  check(d.verdict === "answered", `A verdict answered (${d.verdict})`);
  check(d.answer && d.answer.text === NEW.text, "A newer decision note wins over the higher-scoring older peer");
  check(d.reason.includes("recency-resolved"), `A reason carries recency-resolved (${d.reason})`);
  check(!!d.answer && d.answer.superseded && d.answer.superseded.length === 1 && d.answer.superseded[0].text === OLD.text && d.answer.superseded[0].source_ts === OLD.source_ts, "A superseded lists the older same-family note");
  check(d.signals && d.signals.recency_resolved === true, "A signals.recency_resolved true");
}

{
  const store = sandbox("b", [OLD, NEW, ...FILLERS].map((n) => ({ ...n, cls: "decision-deter" })));
  const d = ask(store);
  check(!d.error, "B ask.mjs runs on the sandbox");
  check(d.verdict === "answered", `B verdict answered (${d.verdict})`);
  check(d.answer && d.answer.text === NEW.text, "B newer decision-deter note wins over the higher-scoring older peer");
  check(d.answer && d.answer.cls === "decision-deter", `B answer cls decision-deter (${d.answer && d.answer.cls})`);
  check(d.reason.includes("recency-resolved"), `B reason carries recency-resolved (${d.reason})`);
  check(!!d.answer && d.answer.superseded && d.answer.superseded.length === 1 && d.answer.superseded[0].text === OLD.text, "B superseded lists the older same-family note");
}

{
  const store = sandbox("c", [OLD, NEW, UNREL, ...FILLERS]);
  const d = ask(store);
  check(!d.error, "C ask.mjs runs on the sandbox");
  check(d.verdict === "answered", `C verdict answered (${d.verdict})`);
  check(d.answer && d.answer.text === NEW.text, "C unrelated same-class newer note does not steal the answer");
  check(!!d.answer && d.answer.superseded && d.answer.superseded.length === 1 && d.answer.superseded[0].text === OLD.text, "C superseded holds only the same-family older note");
  check(!d.answer.superseded || !d.answer.superseded.some((s) => s.text === UNREL.text), "C unrelated note absent from superseded");
}

{
  const store = sandbox("d", [NEW, ...FILLERS]);
  const d = ask(store);
  check(!d.error, "D ask.mjs runs on the sandbox");
  check(d.verdict === "answered", `D verdict answered (${d.verdict})`);
  check(d.answer && d.answer.text === NEW.text, "D lone decision note answers normally");
  check(d.answer && d.answer.superseded === null, "D no superseded on a lone decision note");
  check(!d.reason.includes("recency-resolved"), `D no recency path without a same-family peer (${d.reason})`);
  check(!d.signals.recency_resolved, "D signals.recency_resolved false");
}

console.log(failed ? `FAILED ${failed}` : "ALL PASS");
process.exit(failed ? 1 : 0);
