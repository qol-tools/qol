#!/usr/bin/env node
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { qolMemoryStore } from "./lib/store-path.js";
import { parseUnitsText } from "./lib/seal.js";
import { normalizeQuery, candidateKey, discriminatorCount, rotateIfNeeded } from "./lib/retrieval-log.js";

const BASE = dirname(fileURLToPath(import.meta.url));
const ASK = join(BASE, "ask.mjs");
const CANDIDATES = join(BASE, "candidates.mjs");
const VERDICT_EVAL = join(BASE, "eval", "verdict-eval.mjs");
const HELDOUT = join(BASE, "eval", "heldout.json");
const REAL_STORE = qolMemoryStore();
const ROOT = join(tmpdir(), "qol-memory-retrieval-log");
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

process.on("exit", () => {
  if (process.env.QOL_MEMORY_E2E_KEEP === "1") return;
  try {
    rmSync(ROOT, { recursive: true, force: true });
  } catch {}
});

const WORDS = ["blender", "kcd2", "weapon", "rig", "lattice", "geometry", "node", "mesh", "topology", "vertex", "weight", "paint", "bone", "armature", "pose", "driver", "modifier", "texture", "shader", "light", "camera", "viewport", "render", "engine", "curve", "bevel", "mirror", "array", "instance", "collection", "scene", "keyframe", "timeline", "action", "clip", "audio", "mixer", "track", "gain", "compressor", "equalizer", "reverb", "delay", "chorus", "oscillator", "filter", "envelope", "sequencer", "pattern", "drum", "kick", "snare", "bass", "lead", "pad", "chord", "melody", "tempo", "meter", "scale", "phase", "waveform", "spectrum", "harmonic", "resonance", "decay", "attack", "loop", "layer", "strip", "monitor", "console", "panel", "dial", "knob", "fader", "bus", "route", "stem", "master", "bounce", "export", "import", "archive", "backup", "restore", "canvas", "brush", "stroke", "dodge", "burn", "smudge", "clone", "heal", "grain", "noise", "vignette", "gradient", "opacity", "blend", "mask", "alpha", "depth", "normal", "roughness", "metallic", "emission", "subsurface", "translucent", "refraction", "dispersion", "lens", "focal", "aperture", "shutter", "white", "balance", "histogram", "curves", "levels", "hue", "saturation", "vibrance", "warmth", "tint", "contrast", "shadow", "highlight", "midtone", "tonemap", "exposure", "dynamic", "range", "bitrate", "codec", "buffer", "latency", "jitter", "throughput", "bandwidth", "packet", "frame", "hertz", "decibel", "limiter", "expander", "deesser", "sidechain", "stereo", "mono", "width", "room", "hall", "plate", "spring", "impulse", "convolution", "flanger", "phaser", "ring", "modulator", "pitch", "formant", "vowel", "grit", "drive", "tube", "tape", "digital", "analog", "hybrid", "modular", "cable", "rack", "pedal", "amp", "cabinet", "speaker", "pair", "signal", "chain", "insert", "return", "send", "aux", "group", "folder", "preserve", "template", "macro", "mapping", "assign", "layout", "workspace", "sidebar", "toolbar", "menu", "shortcut", "gesture", "drag", "hover", "scroll", "zoom", "pan", "rotate", "orbit", "fly", "walk", "teleport", "cursor", "pointer", "selection", "marquee", "lasso", "ruler", "grid", "snap", "align", "distribute", "center", "average", "symmetry", "radial", "circular", "linear", "spiral", "helix", "toroid", "sphere", "cube", "plane", "disc", "cylinder", "cone", "pyramid", "prism", "wedge", "chamfer", "fillet", "hollow", "shell", "thickness", "wall", "strut", "brace", "girder", "truss", "chassis", "hull", "deck", "bulkhead", "keel", "mast", "boom", "sail", "rudder", "propeller", "rotor", "blade", "turbine", "fan", "duct", "nozzle", "combustor", "plenum", "manifold", "header", "riser", "stack", "flue", "damper", "louver", "grille", "vent", "port", "orifice", "throttle", "carburetor", "injector", "piston", "crank", "camshaft", "valve", "hinge", "pivot", "bearing", "bushing", "seal", "gasket", "flange", "coupler", "spline", "keyway", "broach", "reamer", "tap", "die", "mold", "cast", "forge", "weld", "braze", "solder", "rivet", "bolt", "screw", "stud", "pin", "dowel", "shim", "spacer", "washer", "nut", "bracket", "mount", "pedestal", "base", "plinth", "footer", "trim", "molding", "cornice", "baseboard", "wainscot", "chair", "sofa", "table", "desk", "shelf", "cabinet", "dresser", "nightstand", "lamp", "chandelier", "sconce", "pendant", "spot", "flood", "wash", "uplight", "downlight", "accent", "ambient", "task", "mood", "dimmer", "switch", "outlet", "breaker", "conduit", "raceway", "junction", "box", "gang", "device", "lampholder", "socket", "plug", "cord", "wire", "conductor", "insulation", "sheath", "armor", "jacket", "wrap", "lacing", "tie", "label", "clamp", "binder", "notebook", "journal", "ledger", "logbook", "vault", "vessel", "chamber", "cavern", "grotto", "alcove", "niche", "ledge", "pocket", "pouch", "satchel", "haversack", "knapsack", "backpack", "rucksack", "carrier", "tote", "handbag", "purse", "billfold", "cardholder", "keychain", "lanyard", "badge", "emblem", "insignia", "crest", "coat", "mantle", "shroud", "cloak", "robe", "tunic", "sweater", "cardigan", "parka", "anorak", "poncho", "cape", "hood", "cowl", "bonnet", "cap", "beret", "beanie", "tam", "toque", "fedora", "bowler", "trilby", "homburg", "panama", "straw", "boater", "derby", "cloche", "wimple", "veil", "mantilla", "tiara", "diadem", "coronet", "crown", "scepter", "orb", "mace", "halberd", "pike", "lance", "javelin", "spear", "trident", "glaive", "partisan", "spontoon", "billhook", "fauchard", "voulge", "bec", "corseque", "ranseur", "spiancon", "gisarme", "guisarme", "couteau", "dagger", "dirk", "stiletto", "misericorde", "bowie", "kukri", "machete", "falchion", "scimitar", "sabre", "rapier", "foil", "epee", "estoc", "smallsword", "claymore", "zweihander", "flamberge", "espada", "tulwar", "shamshir", "kilij", "palash", "khanda", "firangi", "pata", "katar", "sai", "tonfa", "nunchaku", "bo", "jo", "hanbo", "eku", "kusarigama", "manriki", "kusari", "fundow", "jitte", "sasumata", "tsukubo", "sodegarami", "torimono", "tekkan", "jutte"];
const NOTES_RUN = "2026-08-14T09-00-00-000Z";
const DECISION_QUERY = "what did we set for the release naming convention";
const MARKER_QUERY = "what is the sandbox fixture marker";
const TRAP_QUERY = "what is the airspeed velocity of an unladen swallow";

function env(store, extra = {}) {
  const e = { ...process.env };
  delete e.QOL_MEMORY_RETRIEVAL_LOG_DISABLE;
  return { ...e, QOL_MEMORY_STORE: store, ...extra };
}

function runAsk(store, query, extra = [], e = env(store)) {
  const r = spawnSync("node", [ASK, query, "--brief", ...extra], { encoding: "utf8", timeout: 120000, env: e });
  return { status: r.status, stdout: r.stdout || "", stderr: r.stderr || "" };
}

function runCandidates(store, args) {
  const r = spawnSync("node", [CANDIDATES, "--store", store, ...args], { encoding: "utf8", timeout: 300000, maxBuffer: 64 * 1024 * 1024, env: env(store) });
  return { status: r.status, stdout: r.stdout || "", stderr: r.stderr || "" };
}

function spawnAsk(store, query) {
  return new Promise((resolve) => {
    const child = spawn("node", [ASK, query, "--brief"], { env: env(store), stdio: ["ignore", "pipe", "pipe"] });
    let out = "";
    let err = "";
    child.stdout.on("data", (d) => (out += d));
    child.stderr.on("data", (d) => (err += d));
    child.on("close", (code) => resolve({ code, out, err }));
  });
}

function logLines(store) {
  try {
    return parseUnitsText(readFileSync(join(store, "retrievals.jsonl"), "utf8"));
  } catch {
    return [];
  }
}

function candLines(store) {
  try {
    return parseUnitsText(readFileSync(join(store, "candidates.jsonl"), "utf8"));
  } catch {
    return [];
  }
}

function reportOf(store) {
  return JSON.parse(readFileSync(join(store, "ingest", "report.json"), "utf8"));
}

function seedCorpus(store) {
  mkdirSync(join(store, "notes", NOTES_RUN), { recursive: true });
  mkdirSync(join(store, "snapshot"), { recursive: true });
  const units = [];
  for (let i = 0; i < 40; i++) {
    const pick = (k) => WORDS[(i * 31 + k * 17) % WORDS.length];
    let text = "";
    for (let j = 0; j < 24; j++) text += pick(j) + " ";
    units.push({ key: "u-" + String(i).padStart(3, "0"), source: "test", session: "rl-synth", cwd: "/tmp/rl", kind: "user", ts: new Date(Date.UTC(2026, 7, 12) + i * 1000).toISOString(), text: text.trim() + "." });
  }
  writeFileSync(join(store, "units.jsonl"), units.map((u) => JSON.stringify(u)).join("\n") + "\n");
  const notes = [
    { key: "n-dec", cls: "decision", text: "The release naming convention is set: vMAJOR.MINOR-PATCH for feature releases.", source_key: "k-dec", source_ts: "2026-08-13T10:00:00.000Z", source_kind: "decision-deter" },
    { key: "n-con", cls: "constraint", text: "Never edit generated files directly; always regenerate through the tool that owns them.", source_key: "k-con", source_ts: "2026-08-13T10:00:00.000Z", source_kind: "decision-deter" },
    { key: "n-mark", cls: "decision", text: "The sandbox fixture marker is teal falcon.", source_key: "k-mark", source_ts: "2026-08-13T10:00:00.000Z", source_kind: "decision-deter" },
  ];
  for (let i = 0; i < 120; i++) {
    const pick = (k) => WORDS[(i * 37 + k * 13) % WORDS.length];
    let text = "";
    for (let j = 0; j < 10; j++) text += pick(j) + " ";
    notes.push({ key: "n-fill-" + String(i).padStart(3, "0"), cls: "decision", text: text.trim() + " while the lane stays on track.", source_key: "k-f" + i, source_ts: "2026-08-13T10:00:00.000Z", source_kind: "decision-deter" });
  }
  writeFileSync(join(store, "notes", NOTES_RUN, "notes.jsonl"), notes.map((n) => JSON.stringify(n)).join("\n") + "\n");
}

function seedNotes(store) {
  const notes = [
    { key: "n-dec", cls: "decision", text: "The release naming convention is set: vMAJOR.MINOR-PATCH for feature releases.", source_key: "k-dec", source_ts: "2026-08-13T10:00:00.000Z", source_kind: "decision-deter" },
    { key: "n-mark", cls: "decision", text: "The sandbox fixture marker is teal falcon.", source_key: "k-mark", source_ts: "2026-08-13T10:00:00.000Z", source_kind: "decision-deter" },
  ];
  mkdirSync(join(store, "notes", NOTES_RUN), { recursive: true });
  writeFileSync(join(store, "notes", NOTES_RUN, "notes.jsonl"), notes.map((n) => JSON.stringify(n)).join("\n") + "\n");
}

function missEvent(ts, query, source, verdict, recalled) {
  return {
    ts,
    source,
    query,
    verdict,
    confidence: "low",
    correctness: null,
    latency_ms: 123,
    k: 5,
    exclusion: { exclude_session: false, non_default_gates: false },
    gates: {},
    signals: { notes_run_ts: NOTES_RUN },
    answer_key: null,
    recalled_keys: recalled,
    counts: { units: 40, notes: 123 },
  };
}

{
  const store = join(ROOT, "h1");
  seedCorpus(store);
  const q1 = runAsk(store, DECISION_QUERY);
  const q2 = runAsk(store, MARKER_QUERY);
  const q3 = runAsk(store, TRAP_QUERY);
  check(q1.status === 0 && q2.status === 0 && q3.status === 0, "H1 three verdict-mode runs succeed");
  const lines = logLines(store);
  check(lines.length === 3, "H1 three runs append exactly three events");
  const queries = lines.map((l) => l.query);
  check(queries.includes(DECISION_QUERY) && queries.includes(MARKER_QUERY) && queries.includes(TRAP_QUERY), "H1 one event per query");
  for (const l of lines) {
    check(/^\d{4}-\d{2}-\d{2}T/.test(l.ts || ""), "H1 event carries ISO ts");
    check(l.source === "ask-cli", "H1 default source is ask-cli");
    check(l.session === null && l.cwd === null, "H1 session and cwd null without flags");
    check(["answered", "candidates", "no-memory"].includes(l.verdict), "H1 verdict present");
    check(typeof l.confidence === "string", "H1 confidence present");
    check(l.correctness === null, "H1 correctness null without --log-fact");
    check(Number.isInteger(l.latency_ms) && l.latency_ms >= 0, "H1 latency_ms integer");
    check(l.k === 5, "H1 k defaults to 5");
    check(l.exclusion && l.exclusion.exclude_session === false && l.exclusion.non_default_gates === false, "H1 exclusion object present");
    check(l.gates && typeof l.gates.NO_MEMORY_COV === "number" && typeof l.gates.HIGH_MARGIN === "number", "H1 gates projected verbatim");
    check(l.signals && (typeof l.signals.top_note_score === "number" || l.signals.top_note_score === null) && "notes_run_ts" in l.signals, "H1 signals projected verbatim");
    check(Array.isArray(l.recalled_keys) && l.recalled_keys.length === 5, "H1 recalled_keys holds the top-5 note keys");
    check(l.counts && l.counts.units === 40 && l.counts.notes === 123, "H1 counts projected");
    check(l.answer_key === null || typeof l.answer_key === "string", "H1 answer_key null or key");
  }
}

{
  const store = join(ROOT, "h2");
  seedCorpus(store);
  const r = runAsk(store, DECISION_QUERY, [], env(store, { QOL_MEMORY_RETRIEVAL_LOG_DISABLE: "1" }));
  check(r.status === 0, "H2 kill-switch ask still succeeds");
  check(logLines(store).length === 0, "H2 kill-switch yields zero appends");
}

{
  const store = join(ROOT, "h3");
  seedCorpus(store);
  const r = runAsk(store, DECISION_QUERY, ["--no-log"]);
  check(r.status === 0, "H3 --no-log ask still succeeds");
  check(logLines(store).length === 0, "H3 --no-log yields zero appends (the calibrate path)");
}

{
  const store = join(ROOT, "h4");
  seedCorpus(store);
  runAsk(store, MARKER_QUERY, ["--log-source", "tool", "--exclude-session", "rl-synth", "--log-cwd", "/tmp/proj"]);
  runAsk(store, DECISION_QUERY, ["--log-source", "eval", "--log-fact", "vMAJOR.MINOR-PATCH"]);
  const lines = logLines(store);
  check(lines.length === 2, "H4 two flagged runs append two events");
  const tool = lines.find((l) => l.source === "tool");
  check(tool && tool.session === "rl-synth" && tool.cwd === "/tmp/proj" && tool.exclusion.exclude_session === true, "H4 tool source carries session + cwd + exclusion flag");
  const ev = lines.find((l) => l.source === "eval");
  check(ev && ev.correctness === "correct", "H4 eval source carries the fact annotation");
}

{
  const store = join(ROOT, "h5");
  seedCorpus(store);
  runAsk(store, DECISION_QUERY, ["--log-source", "eval", "--log-fact", "vMAJOR.MINOR-PATCH"]);
  runAsk(store, DECISION_QUERY, ["--log-source", "eval", "--log-fact", "the anchoring was fixed with shims"]);
  runAsk(store, TRAP_QUERY, ["--log-source", "eval", "--log-fact", "gold fact"]);
  runAsk(store, TRAP_QUERY, ["--log-source", "eval", "--log-fact", "trap:no-memory"]);
  runAsk(store, DECISION_QUERY, ["--log-source", "eval", "--log-fact", "trap:no-memory"]);
  const lines = logLines(store);
  check(lines.length === 5, "H5 five eval runs append five events");
  check(lines[0].correctness === "correct", "H5 answered + match annotates correct");
  check(lines[1].correctness === "wrong", "H5 answered without match annotates wrong");
  check(lines[2].correctness === "unanswered", "H5 abstained annotates unanswered");
  check(lines[3].correctness === "untrapped", "H5 trap not answered annotates untrapped");
  check(lines[4].correctness === "trapped", "H5 trap answered annotates trapped");
}

{
  const store = join(ROOT, "h6");
  seedNotes(store);
  const now = Date.now();
  const events = [
    missEvent(new Date(now - 3600e3).toISOString(), "what happened to the old blender rigs", "tool", "no-memory", ["n-dec"]),
    missEvent(new Date(now - 1800e3).toISOString(), "which session discussed the marker color", "ask-cli", "candidates", ["n-mark"]),
    missEvent(new Date(now - 900e3).toISOString(), "what did we set for the release naming convention", "tool", "answered", ["n-dec"]),
  ];
  writeFileSync(join(store, "retrievals.jsonl"), events.map((e) => JSON.stringify(e)).join("\n") + "\n");
  const r = runCandidates(store, ["harvest"]);
  check(r.status === 0, "H6 harvest exits 0");
  const cands = candLines(store);
  check(cands.length === 2, "H6 two miss events yield exactly two candidates");
  check(cands.every((c) => c.status === "candidate"), "H6 candidates carry status candidate");
  check(cands.some((c) => c.norm_query === normalizeQuery("what happened to the old blender rigs")), "H6 no-memory miss captured");
  check(cands.some((c) => c.norm_query === normalizeQuery("which session discussed the marker color")), "H6 candidates verdict miss captured");
  const rig = cands.find((c) => c.norm_query === normalizeQuery("what happened to the old blender rigs"));
  check(rig && rig.fact === "The release naming convention is set: vMAJOR.MINOR-PATCH for feature releases.", "H6 fact resolved from the event's notes run");
  check(rig && rig.fact_norm === normalizeQuery(rig.fact), "H6 fact_norm carries the normalized fact");
  check(rig && rig.source_unit_key === "n-dec" && rig.source === "tool" && rig.verdict === "no-memory" && rig.source_event_ts === events[0].ts, "H6 provenance fields carried");
  check(rig && /^[0-9a-f]{16}$/.test(rig.key), "H6 key is a 16-hex slice");
  check(rig && rig.key === createHash("sha256").update(rig.norm_query).digest("hex").slice(0, 16), "H6 key is sha256(norm_query) slice(0,16)");
  const report = reportOf(store);
  check(report.harvest.misses === 2 && report.harvest.candidates_added === 2, "H6 report.json carries harvest counts");
  check(report.candidates.length === 2 && report.pending === 2, "H6 report.json lists the proposals and pending count");
  check(!cands.some((c) => c.verdict === "answered"), "H6 answered event never harvested");
}

{
  const store = join(ROOT, "h7");
  seedNotes(store);
  const now = Date.now();
  const eH = missEvent(new Date(now - 7200e3).toISOString(), "How many sessions does the snapshot sample per source?", "tool", "no-memory", ["n-mark"]);
  const eB = missEvent(new Date(now).toISOString(), "what is the sandbox fixture marker", "tool", "no-memory", ["n-mark"]);
  const eC = missEvent(new Date(now + 10e3).toISOString(), "What is the sandbox fixture marker?", "tool", "no-memory", ["n-mark"]);
  writeFileSync(join(store, "retrievals.jsonl"), [eH, eB, eC].map((e) => JSON.stringify(e)).join("\n") + "\n");
  const h1 = runCandidates(store, ["harvest"]);
  check(h1.status === 0, "H7 first harvest exits 0");
  check(candLines(store).length === 1, "H7 same norm_query dedupes to one candidate");
  check(reportOf(store).harvest.candidates_added === 1, "H7 first harvest adds one");
  check(reportOf(store).harvest.skipped.heldout === 1 && reportOf(store).harvest.skipped.duplicate === 1, "H7 heldout-matching query skipped, near-duplicate skipped");
  const eD = missEvent(new Date(now + 3600e3).toISOString(), "what is the sandbox fixture marker", "tool", "no-memory", ["n-mark"]);
  writeFileSync(join(store, "retrievals.jsonl"), [eH, eB, eC, eD].map((e) => JSON.stringify(e)).join("\n") + "\n");
  const h2 = runCandidates(store, ["harvest"]);
  check(h2.status === 0, "H7 second harvest exits 0");
  check(candLines(store).length === 1, "H7 re-miss within 24h skipped by cooldown");
  check(reportOf(store).harvest.candidates_added === 0, "H7 second harvest adds nothing");
  const eE = missEvent(new Date(now + 25 * 3600e3).toISOString(), "what is the sandbox fixture marker", "tool", "no-memory", ["n-mark"]);
  writeFileSync(join(store, "retrievals.jsonl"), [eH, eB, eC, eD, eE].map((e) => JSON.stringify(e)).join("\n") + "\n");
  const h3 = runCandidates(store, ["harvest"]);
  check(h3.status === 0, "H7 third harvest exits 0");
  check(candLines(store).length === 2, "H7 re-miss after the cooldown captures again");
}

{
  const p = join(ROOT, "rotate.jsonl");
  const lines = [];
  for (let i = 0; i < 200; i++) lines.push(JSON.stringify({ i, pad: "x".repeat(80) }));
  writeFileSync(p, lines.join("\n") + "\n");
  const before = readFileSync(p);
  rotateIfNeeded(p, 4000, 2000);
  const after = readFileSync(p);
  check(after.length < before.length, "H8 oversized log truncates");
  check(before.subarray(before.length - after.length).equals(after), "H8 kept slice is a suffix of the original");
  check(after.length >= 2000 && after.length < 4000, "H8 kept size lands in the tail band");
  const kept = after.toString("utf8").trim().split("\n");
  check(kept.every((l) => { try { JSON.parse(l); return true; } catch { return false; } }), "H8 every kept line parses, no partial line survives");
  check(before[before.length - after.length - 1] === 10, "H8 cut lands on a newline boundary");
  const small = join(ROOT, "rotate-small.jsonl");
  writeFileSync(small, lines.slice(0, 5).join("\n") + "\n");
  const smallBefore = readFileSync(small);
  rotateIfNeeded(small, 4000, 2000);
  check(readFileSync(small).equals(smallBefore), "H8 under-cap file untouched");
  const missing = join(ROOT, "rotate-missing.jsonl");
  try {
    rotateIfNeeded(missing);
    check(true, "H8 missing file is a no-op");
  } catch {
    check(false, "H8 missing file is a no-op");
  }
}

{
  const a = join(ROOT, "h10a");
  const b = join(ROOT, "h10b");
  seedCorpus(a);
  seedCorpus(b);
  const bigLine = JSON.stringify({ seq: 0, pad: "x".repeat(90) });
  writeFileSync(join(b, "retrievals.jsonl"), Array(103000).fill(bigLine).join("\n") + "\n");
  const ra = runAsk(a, DECISION_QUERY);
  const rb = runAsk(b, DECISION_QUERY);
  check(ra.status === 0 && rb.status === 0, "H10 both neutrality asks succeed");
  check(ra.stdout === rb.stdout, "H10 ask.mjs stdout byte-identical with and without a populated log");
  const size = statSync(join(b, "retrievals.jsonl")).size;
  check(size > 1024 * 1024 && size < 2 * 1024 * 1024, "H10 >cap log rotated down to the tail band");
  const rotated = logLines(b);
  check(rotated[rotated.length - 1].query === DECISION_QUERY, "H10 rotated log ends with the fresh event");
  check(rotated.length > 9000, "H10 rotated log keeps the full tail lines intact");
  const realEnv = { ...process.env };
  delete realEnv.QOL_MEMORY_STORE;
  delete realEnv.QOL_MEMORY_RETRIEVAL_LOG_DISABLE;
  const gate = spawnSync("node", [VERDICT_EVAL, "--rebuild"], { encoding: "utf8", timeout: 600000, maxBuffer: 64 * 1024 * 1024, env: { ...realEnv, QOL_MEMORY_ALIASES_DISABLE: "1" } });
  check(gate.status === 0, "H10 frozen verdict-eval gate exits 0");
  check(gate.stdout.includes("heldout 30 | answered 22 | correct 22 | wrong 0 | unanswered 8 | traps 8/8 safe | gate PASS"), "H10 frozen invariant 22/22/0/8 traps 8/8 PASS unchanged");
  check(/candidates pending \d+/.test(gate.stdout), "H10 gate line carries the informational candidates pending count");
}

{
  const store = join(ROOT, "h9");
  mkdirSync(store, { recursive: true });
  const sandboxHeldout = join(store, "heldout.json");
  writeFileSync(sandboxHeldout, readFileSync(HELDOUT, "utf8"));
  const mk = (query, fact, verdict, sourceUnitKey = "n-x") => ({
    key: candidateKey(normalizeQuery(query)),
    query,
    norm_query: normalizeQuery(query),
    fact,
    fact_norm: normalizeQuery(fact),
    source_unit_key: sourceUnitKey,
    source_event_ts: "2026-08-14T06:00:00.000Z",
    source: "tool",
    session: null,
    cwd: null,
    verdict,
    created_ts: "2026-08-14T06:01:00.000Z",
    status: "candidate",
    promoted_ts: null,
    heldout_id: null,
    rejected_ts: null,
    reject_reason: null,
  });
  const passQ = "how are run and walk states played in the engine";
  const failGateQ = "how was tiered orchestration settled";
  const failDiscQ = "is qol-memory a worktree project";
  const wrongKeyQ = "how does the engine play the run and walk states";
  const passKey = candidateKey(normalizeQuery(passQ));
  const failGateKey = candidateKey(normalizeQuery(failGateQ));
  const failDiscKey = candidateKey(normalizeQuery(failDiscQ));
  const wrongKey = candidateKey(normalizeQuery(wrongKeyQ));
  const cands = [mk(passQ, "bspaces", "no-memory", "79046028d14b1cec"), mk(failGateQ, "zzz definitely not a fact anywhere zzz", "no-memory"), mk(failDiscQ, "zzz nowhere in the corpus zzz", "candidates"), mk(wrongKeyQ, "bspaces", "no-memory")];
  writeFileSync(join(store, "candidates.jsonl"), cands.map((c) => JSON.stringify(c)).join("\n") + "\n");
  check(discriminatorCount("bspaces", ["the engine plays run and walk states as bspaces", "unrelated note", "another note"]) === 1, "H9 discriminatorCount counts verbatim single-note matches");
  check(discriminatorCount("zzz nowhere", ["the engine plays run and walk states as bspaces"]) === 0, "H9 discriminatorCount zero for absent facts");
  const runPromote = (key) => spawnSync("node", [CANDIDATES, "--store", store, "--promote", key, "--heldout", sandboxHeldout], { encoding: "utf8", timeout: 300000, maxBuffer: 64 * 1024 * 1024, env: env(store) });
  const pass = runPromote(passKey);
  check(pass.status === 0, "H9 passing candidate promotes (exit 0)");
  const held = JSON.parse(readFileSync(sandboxHeldout, "utf8"));
  check(held.questions.length === 31, "H9 heldout file gains the question");
  check(JSON.stringify(held.questions[held.questions.length - 1]) === JSON.stringify({ id: passKey, query: passQ, fact: "bspaces" }), "H9 appended question carries id + query + fact");
  const passC = candLines(store).find((c) => c.key === passKey);
  check(passC && passC.status === "promoted" && passC.heldout_id === passKey && typeof passC.promoted_ts === "string", "H9 candidate flips to promoted with promoted_ts + heldout_id");
  const failWrong = runPromote(wrongKey);
  check(failWrong.status !== 0, "H9 wrong-source candidate exits non-zero");
  check((failWrong.stderr || "").includes("answer_key=79046028d14b1cec") && (failWrong.stderr || "").includes("source_unit_key=n-x"), "H9 wrong-source candidate reports answer_key != source_unit_key");
  check(candLines(store).find((c) => c.key === wrongKey).status === "candidate", "H9 wrong-source candidate never promotes");
  const failGate = runPromote(failGateKey);
  check(failGate.status !== 0, "H9 gate-failing candidate exits non-zero");
  check(failGate.stdout.includes("gate FAIL"), "H9 gate-failing candidate prints the gate output as the reason");
  check((failGate.stderr || "").includes("FAIL") && (failGate.stderr || "").includes("row=wrong"), "H9 gate-failing candidate reports row=wrong");
  check(candLines(store).find((c) => c.key === failGateKey).status === "candidate", "H9 gate-failing candidate never promotes");
  const failDisc = runPromote(failDiscKey);
  check(failDisc.status !== 0, "H9 discriminator-failing candidate exits non-zero");
  check(failDisc.stdout.includes("gate PASS"), "H9 discriminator failure despite a passing gate");
  check((failDisc.stderr || "").includes("discriminator=0"), "H9 discriminator=0 reported as the reason");
  check(candLines(store).find((c) => c.key === failDiscKey).status === "candidate", "H9 discriminator-failing candidate never promotes");
  const failDisc2 = runPromote(failDiscKey);
  check(failDisc2.status !== 0, "H11 second promote of the failing candidate exits non-zero");
  check(failDisc2.stdout === failDisc.stdout, "H11 two promote evaluations produce identical gate output");
  const rej = runCandidates(store, ["--reject", failDiscKey, "--reason", "fact not distinctive"]);
  check(rej.status === 0, "H9 --reject exits 0");
  const rejC = candLines(store).find((c) => c.key === failDiscKey);
  check(rejC.status === "rejected" && rejC.reject_reason === "fact not distinctive" && typeof rejC.rejected_ts === "string", "H9 rejected candidate records reason + ts");
  const rej2 = runCandidates(store, ["--reject", failDiscKey, "--reason", "again"]);
  check(rej2.status !== 0, "H9 second reject of a rejected candidate refuses");
  const cnt = runCandidates(store, ["count"]);
  check(cnt.status === 0 && cnt.stdout.trim() === "2", "H9 count helper reports two pending candidates");
}

{
  const store = join(ROOT, "h11");
  seedNotes(store);
  const now = Date.now();
  const events = [
    missEvent(new Date(now - 3600e3).toISOString(), "what happened to the old blender rigs", "tool", "no-memory", ["n-dec"]),
    missEvent(new Date(now - 1800e3).toISOString(), "which session discussed the marker color", "ask-cli", "candidates", ["n-mark"]),
  ];
  writeFileSync(join(store, "retrievals.jsonl"), events.map((e) => JSON.stringify(e)).join("\n") + "\n");
  const d1 = runCandidates(store, ["harvest"]);
  const firstCandidates = readFileSync(join(store, "candidates.jsonl"));
  const d2 = runCandidates(store, ["harvest"]);
  check(d1.status === 0 && d2.status === 0, "H11 both harvest runs exit 0");
  check(readFileSync(join(store, "candidates.jsonl")).equals(firstCandidates), "H11 two harvest runs on the same log produce identical candidates.jsonl");
  check(reportOf(store).harvest.candidates_added === 0, "H11 second harvest adds nothing");
}

{
  const store = join(ROOT, "h12");
  seedCorpus(store);
  runAsk(store, DECISION_QUERY);
  const [m, t] = await Promise.all([spawnAsk(store, MARKER_QUERY), spawnAsk(store, TRAP_QUERY)]);
  check(m.code === 0 && t.code === 0, "H12 both concurrent asks succeed");
  const lines = logLines(store);
  check(lines.length === 3, "H12 three lines total (prewarm + two concurrent)");
  const queries = lines.map((l) => l.query).sort();
  check(JSON.stringify(queries) === JSON.stringify([DECISION_QUERY, MARKER_QUERY, TRAP_QUERY].sort()), "H12 concurrent lines intact, one per query, no interleaving");
  check(lines.every((l) => typeof l.verdict === "string"), "H12 every concurrent line parses as a full event");
}

console.log(failed ? `FAILED ${failed}` : "ALL PASS");
process.exit(failed ? 1 : 0);
