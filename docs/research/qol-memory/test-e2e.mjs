#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { parseUnitsText } from "./lib/seal.js";

const BASE = dirname(fileURLToPath(import.meta.url));
const SANDBOX = join(tmpdir(), "qol-memory-e2e-" + createHash("sha256").update(String(process.pid) + Date.now()).digest("hex").slice(0, 8));
const STORE = join(SANDBOX, "store");
const SESSIONS = join(SANDBOX, "sessions");
const PI_DIR = join(SESSIONS, "pi");
const CLAUDE_DIR = join(SESSIONS, "claude");
const PIN_RUN = "2026-08-10T21-38-02-273Z";
const HELDOUT = join(SANDBOX, "heldout-e2e.json");

const DECISION_QUERY = "what did we set for the release naming convention";
const TRAP_QUERY = "did we reject the build date tag scheme";
const MARKER_QUERY = "what is the sandbox fixture marker";
const SESSION_A = "e2e-session-a";
const SESSION_B = "e2e-session-b";
const SESSION_C = "e2e-session-c";

const DECISION_UNIT = "The release naming convention is set: vMAJOR.MINOR-PATCH for feature releases.";
const CONSTRAINT_UNIT = "Never edit generated files directly; always regenerate through the tool that owns them.";
const TRAP_UNIT = "The release naming convention is not set: the team keeps the rejected build date tag scheme, and the vMAJOR.MINOR-PATCH proposal failed the review. The pipeline is unchanged while the team reworks the tooling contract, the migration plan, and the rollout checklist of the registry, and the release cadence keeps the old naming until the rework lands.";
const MARKER_UNIT = "The sandbox fixture marker is teal falcon.";
const COMPACTION_A = "## Key Decisions\n- The release naming convention is set: vMAJOR.MINOR-PATCH for feature releases.\n## Progress\n- the naming doc merged into the guide.";
const COMPACTION_B = "## Constraints & Preferences\n- Never edit generated files directly.";

const SAFE = ["blender", "kcd2", "weapon", "rig", "lattice", "geometry", "node", "mesh", "topology", "vertex", "weight", "paint", "bone", "armature", "pose", "driver", "modifier", "texture", "shader", "light", "camera", "viewport", "render", "engine", "curve", "bevel", "mirror", "array", "instance", "collection", "scene", "keyframe", "timeline", "action", "clip", "audio", "mixer", "track", "gain", "compressor", "equalizer", "reverb", "delay", "chorus", "oscillator", "filter", "envelope", "sequencer", "pattern", "drum", "kick", "snare", "bass", "lead", "pad", "chord", "melody", "tempo", "meter", "scale", "phase", "waveform", "spectrum", "harmonic", "resonance", "decay", "attack", "loop", "layer", "strip", "monitor", "console", "panel", "dial", "knob", "fader", "bus", "route", "stem", "master", "bounce", "export", "import", "archive", "backup", "restore", "canvas", "brush", "stroke", "dodge", "burn", "smudge", "clone", "heal", "grain", "noise", "vignette", "gradient", "opacity", "blend", "mask", "alpha", "depth", "normal", "roughness", "metallic", "emission", "subsurface", "translucent", "refraction", "dispersion", "lens", "focal", "aperture", "shutter", "white", "balance", "histogram", "curves", "levels", "hue", "saturation", "vibrance", "warmth", "tint", "contrast", "shadow", "highlight", "midtone", "tonemap", "exposure", "dynamic", "range", "bitrate", "codec", "buffer", "latency", "jitter", "throughput", "bandwidth", "packet", "frame", "hertz", "decibel", "limiter", "expander", "deesser", "sidechain", "stereo", "mono", "width", "room", "hall", "plate", "spring", "impulse", "convolution", "flanger", "phaser", "ring", "modulator", "pitch", "formant", "vowel", "grit", "drive", "tube", "tape", "digital", "analog", "hybrid", "modular", "cable", "rack", "pedal", "amp", "cabinet", "speaker", "pair", "signal", "chain", "insert", "return", "send", "aux", "group", "folder", "snapshot", "preserve", "template", "macro", "mapping", "assign", "layout", "workspace", "sidebar", "toolbar", "menu", "shortcut", "gesture", "drag", "hover", "click", "scroll", "zoom", "pan", "rotate", "orbit", "fly", "walk", "teleport", "cursor", "pointer", "selection", "marquee", "lasso", "wand", "eyedropper", "ruler", "grid", "snap", "align", "distribute", "center", "average", "symmetry", "radial", "circular", "linear", "spiral", "helix", "toroid", "sphere", "cube", "plane", "disc", "cylinder", "cone", "pyramid", "prism", "wedge", "chamfer", "fillet", "hollow", "shell", "thickness", "wall", "strut", "brace", "girder", "truss", "chassis", "hull", "deck", "bulkhead", "keel", "mast", "boom", "sail", "rudder", "propeller", "rotor", "blade", "turbine", "fan", "duct", "nozzle", "combustor", "plenum", "manifold", "header", "riser", "stack", "flue", "damper", "louver", "grille", "vent", "port", "orifice", "throttle", "carburetor", "injector", "piston", "crank", "camshaft", "valve", "hinge", "pivot", "bearing", "bushing", "seal", "gasket", "flange", "coupler", "spline", "keyway", "broach", "reamer", "tap", "die", "mold", "cast", "forge", "weld", "braze", "solder", "rivet", "bolt", "screw", "stud", "pin", "dowel", "wedge", "shim", "spacer", "washer", "nut", "bracket", "mount", "pedestal", "base", "plinth", "footer", "trim", "molding", "cornice", "baseboard", "wainscot", "chair", "sofa", "table", "desk", "shelf", "cabinet", "dresser", "wardrobe", "nightstand", "lamp", "chandelier", "sconce", "pendant", "spot", "flood", "wash", "uplight", "downlight", "accent", "ambient", "task", "mood", "dimmer", "switch", "outlet", "breaker", "conduit", "raceway", "junction", "box", "gang", "device", "lampholder", "socket", "plug", "cord", "wire", "conductor", "insulation", "sheath", "armor", "jacket", "wrap", "lacing", "tie", "label", "clamp", "binder", "notebook", "journal", "ledger", "logbook", "vault", "vessel", "chamber", "cavern", "grotto", "alcove", "niche", "ledge", "pocket", "pouch", "satchel", "haversack", "knapsack", "backpack", "rucksack", "carrier", "tote", "handbag", "purse", "clutch", "wallet", "billfold", "cardholder", "keychain", "lanyard", "badge", "emblem", "insignia", "crest", "coat", "mantle", "shroud", "cloak", "robe", "gown", "tunic", "blouse", "shirt", "sweater", "cardigan", "parka", "anorak", "poncho", "cape", "hood", "cowl", "bonnet", "cap", "beret", "beanie", "tam", "toque", "fedora", "bowler", "trilby", "homburg", "panama", "straw", "boater", "derby", "cloche", "pillbox", "wimple", "veil", "mantilla", "fascinator", "tiara", "diadem", "coronet", "crown", "scepter", "orb", "mace", "halberd", "pike", "lance", "javelin", "spear", "trident", "glaive", "partisan", "spontoon", "billhook", "fauchard", "voulge", "bec", "corseque", "ranseur", "spetum", "plancon", "gisarme", "guisarme", "couteau", "dagger", "dirk", "stiletto", "misericorde", "bowie", "kukri", "machete", "falchion", "scimitar", "sabre", "rapier", "foil", "epee", "estoc", "smallsword", "broadsword", "claymore", "zweihander", "flamberge", "espada", "tulwar", "shamshir", "kilij", "palash", "khanda", "firangi", "pata", "katar", "bagh", "kama", "sai", "tonfa", "nunchaku", "bo", "jo", "hanbo", "eku", "kusarigama", "manriki", "kusari", "fundow", "jitte", "sasumata", "tsukubo", "sodegarami", "torimono", "tekkan", "jutte"];

const GUARD = ["set", "releas", "naming", "convention", "reject", "build", "date", "tag", "scheme", "never", "edit", "generat", "file", "sandbox", "fixture", "marker", "teal", "falcon", "swallow", "airspeed", "unladen", "velocity", "decid"];

function fillerSentence(g) {
  const w = (k) => SAFE[(g * 31 + k * 17) % SAFE.length];
  return `The ${w(0)} ${w(1)} ${w(2)} ${w(3)} ${w(4)} while the ${w(5)} ${w(6)} ${w(7)} ${w(8)} ${w(9)} stays on the ${w(10)} lane, and the ${w(11)} ${w(12)} ${w(13)} ${w(14)} ${w(15)} holds the ${w(16)} ${w(17)} ${w(18)} line.`;
}

function fillers(n, offset) {
  const out = [];
  for (let i = 0; i < n; i++) out.push(fillerSentence(offset + i));
  return out;
}

function sessionLines(sessionId, units, compaction, baseTs) {
  const lines = [{ type: "session", id: sessionId, cwd: "/tmp/qol-memory-e2e" }];
  units.forEach((text, i) => {
    lines.push({ type: "message", message: { role: "user", content: [{ type: "text", text }], timestamp: new Date(baseTs + i * 1000).toISOString() } });
  });
  if (compaction) {
    lines.push({ type: "compaction", summary: compaction, timestamp: new Date(baseTs + units.length * 1000).toISOString(), details: { readFiles: [], modifiedFiles: [] } });
  }
  return lines.map((l) => JSON.stringify(l)).join("\n") + "\n";
}

let failed = 0;
function check(cond, label) {
  if (!cond) {
    failed++;
    console.error(`FAIL ${label}`);
  } else {
    console.log(`pass ${label}`);
  }
}

function env() {
  return { ...process.env, QOL_MEMORY_STORE: STORE, QOL_MEMORY_PI_DIR: PI_DIR, QOL_MEMORY_CLAUDE_DIR: CLAUDE_DIR, QOL_MEMORY_MODEL_DISABLE: "1" };
}

function run(cmd, args, e = env()) {
  const t = Date.now();
  const r = spawnSync(cmd, args, { encoding: "utf8", timeout: 600000, maxBuffer: 64 * 1024 * 1024, env: e });
  return { status: r.status, stdout: r.stdout || "", stderr: r.stderr || "", ms: Date.now() - t };
}

function scrub(s) {
  return s
    .split(SANDBOX)
    .join("<sandbox>")
    .replace(/\d{4}-\d{2}-\d{2}T[\dZ.:\-]*Z/g, "<ts>")
    .replace(/\d+ms/g, "Nms");
}

function latestRun(root) {
  return readdirSync(root).filter((n) => /^\d{4}-\d{2}-\d{2}T/.test(n)).sort().reverse()[0];
}

function readUnitsFile() {
  return parseUnitsText(readFileSync(join(STORE, "units.jsonl"), "utf8"));
}

const started = Date.now();
process.on("exit", () => {
  if (process.env.QOL_MEMORY_E2E_KEEP === "1") return;
  try {
    rmSync(SANDBOX, { recursive: true, force: true });
  } catch {}
});

mkdirSync(join(STORE, "snapshot", PIN_RUN), { recursive: true });
mkdirSync(PI_DIR, { recursive: true });
mkdirSync(CLAUDE_DIR, { recursive: true });

const aFill = fillers(19, 0);
const bFill = fillers(18, 100);
const cFill = fillers(19, 200);
writeFileSync(join(STORE, "snapshot", PIN_RUN, "snapshot.jsonl"), JSON.stringify({ key: "e2e-pin-seed", source: "test", session: "e2e-pin", cwd: "/tmp", kind: "user", ts: "2026-08-10T00:00:00.000Z", text: aFill[0] }) + "\n");

writeFileSync(join(PI_DIR, "e2e-a.jsonl"), sessionLines(SESSION_A, [DECISION_UNIT, ...aFill], COMPACTION_A, Date.UTC(2026, 7, 13, 9)));
writeFileSync(join(PI_DIR, "e2e-b.jsonl"), sessionLines(SESSION_B, [CONSTRAINT_UNIT, MARKER_UNIT, ...bFill], COMPACTION_B, Date.UTC(2026, 7, 13, 10)));
writeFileSync(join(PI_DIR, "e2e-c.jsonl"), sessionLines(SESSION_C, [TRAP_UNIT, ...cFill], null, Date.UTC(2026, 7, 13, 11)));

writeFileSync(
  HELDOUT,
  JSON.stringify({
    name: "sandbox e2e verdict suite",
    questions: [
      { id: "e1", query: DECISION_QUERY, fact: "vMAJOR.MINOR-PATCH" },
      { id: "e2", query: "should I never edit generated files", fact: "never edit generated files" },
    ],
    traps: [{ id: "t1", query: "what is the airspeed velocity of an unladen swallow", expected: "no-memory" }],
  })
);

const allSeeded = [DECISION_UNIT, CONSTRAINT_UNIT, TRAP_UNIT, MARKER_UNIT, ...aFill, ...bFill, ...cFill];
check(allSeeded.length === 60, "E1 seed carries 60 user units across 3 sessions");
check(new Set(allSeeded).size === 60, "E1 seed texts are pairwise distinct");
check([...aFill, ...bFill, ...cFill].every((t) => !GUARD.some((g) => t.toLowerCase().includes(g))), "E1 filler avoids all assertion vocabulary");
check(allSeeded.some((t) => t === DECISION_UNIT) && allSeeded.some((t) => t === CONSTRAINT_UNIT) && allSeeded.some((t) => t === TRAP_UNIT), "E1 decision/constraint/trap units present");

const skillsRun = run("node", [join(BASE, "skills.mjs")]);
check(skillsRun.status === 0, "E1 skills index built in the sandbox");

const ingest1 = run("node", [join(BASE, "ingest.mjs"), "--no-llm"]);
console.log(scrub(ingest1.stdout));
check(ingest1.status === 0, "E2 ingest exits 0 (QOL_MEMORY_MODEL_DISABLE=1)");
const merge1 = (ingest1.stdout.match(/merge done \((\d+) units in store, (\d+) new from run ([0-9TZ.\-]+)\)/) || []).slice(1);
const report1 = (ingest1.stdout.match(/\[ingest\] report: (.+)/) || [])[1];
const snapRun = merge1[2];
const notesRun = latestRun(join(STORE, "notes"));
check(merge1[0] === "62" && merge1[1] === "62", `E2 merge counts correct (${merge1[0]} in store, ${merge1[1]} new)`);
check(!!snapRun, "E2 snapshot run identified");
check(!!report1 && existsSync(report1), "E2 report-*.json written");
const report = JSON.parse(readFileSync(report1, "utf8"));
check(/^2 \(carried 0\)/.test(report.decisions || ""), `E2 decisions added 2 (got: ${report.decisions})`);
check(report.evals && report.evals.units && report.evals.notes && report.evals.skills && report.evals.verdict !== undefined, "E2 report carries evals + verdict fields");
check(report.evals.units.hit1 === "0/30" && /^\d+\/10$/.test(report.evals.notes.hit1 || "") && String(report.evals.skills).includes("| pass") && String(report.evals.verdict).includes("traps 8/8 safe"), "E2 evals fields have the sandbox-run values");
const decisionNotes = parseUnitsText(readFileSync(join(STORE, "notes", notesRun, "notes.jsonl"), "utf8")).filter((n) => n.cls === "decision");
check(decisionNotes.length >= 2, `E2 notes run created with ${decisionNotes.length} decision notes`);
check(decisionNotes.some((n) => n.text.includes("release naming convention is set")), "E2 decision note distilled from session A compaction");
check(decisionNotes.some((n) => n.text.includes("Never edit generated files directly")), "E2 constraint note distilled from session B compaction");
const merged = readUnitsFile();
check(merged.length === 62, "E2 merged store holds 62 units");
check(new Set(merged.map((u) => u.key)).size === 62, "E2 merged units have distinct keys");
check(new Set(merged.map((u) => u.ts)).size === 62, "E2 merged units have distinct ts");
check(merged.some((u) => u.text === DECISION_UNIT) && merged.some((u) => u.text === CONSTRAINT_UNIT) && merged.some((u) => u.text === TRAP_UNIT), "E2 decision/constraint/trap units merged");

const ingest2 = run("node", [join(BASE, "ingest.mjs"), "--no-llm"]);
const merge2 = (ingest2.stdout.match(/merge done \((\d+) units in store, (\d+) new from run ([0-9TZ.\-]+)\)/) || []).slice(1);
const report2 = (ingest2.stdout.match(/\[ingest\] report: (.+)/) || [])[1];
check(ingest2.status === 0, "E3 second ingest exits 0");
check(merge2[0] === "62" && merge2[1] === "0", `E3 idempotent merge adds 0 new (store ${merge2[0]})`);
check(/^0 \(carried 2\)/.test(JSON.parse(readFileSync(report2, "utf8")).decisions || ""), "E3 decisions re-run adds 0, carries 2");

const askRaw = (query, extra) => run("node", [join(BASE, "ask.mjs"), query, "--brief", ...extra]).stdout;
const ask = (query, extra) => JSON.parse(askRaw(query, extra));
const d1 = ask(DECISION_QUERY, []);
check(d1.verdict === "answered" && (d1.answer.text || "").includes("vMAJOR.MINOR-PATCH"), `E4 decision query answered with the correct fact (${d1.verdict}/${d1.answer && d1.answer.layer})`);
const trapFull = run("node", [join(BASE, "ask.mjs"), TRAP_QUERY]).stdout;
const trapAsk = JSON.parse(trapFull);
check(trapAsk.verdict === "no-memory" || trapAsk.verdict === "candidates", `E4 trap query not answered (${trapAsk.verdict})`);
check(trapFull.includes("rejected build date tag scheme"), "E4 trap unit is the top candidate the gate refuses");
const m1ask = ask(MARKER_QUERY, []);
check(m1ask.verdict === "answered" && (m1ask.answer.text || "").includes("teal falcon"), `E4 marker query answered by its session unit (${m1ask.verdict})`);
const mx = ask(MARKER_QUERY, ["--exclude-session", SESSION_B]);
check(mx.verdict === "no-memory" || mx.verdict === "candidates", `E4 --exclude-session keeps the session from answering its own prompt (${mx.verdict})`);

const vd = run("node", [join(BASE, "eval", "verdict-eval.mjs"), "--store", STORE, "--snapshot-run", snapRun, "--notes-run", latestRun(join(STORE, "notes")), "--heldout", HELDOUT, "--floor", "2", "--rebuild"]);
console.log(scrub(vd.stdout));
check(vd.status === 0, "E5 verdict-eval gate exit 0 on the sandbox");
check(vd.stdout.includes("gate PASS"), "E5 gate PASS proven end-to-end");

const sealMarker = join(STORE, "units.seal.json");
const sealBlob = join(STORE, "units.seal.gz");
check(existsSync(sealMarker) && existsSync(sealBlob), "E6 units.seal.json + units.seal.gz exist after ingest");
check(["idx-pool.json", "idx-pool.json.meta", "idx-user.json", "idx-user.json.meta", "idx-notes.json", "idx-notes.json.meta"].every((f) => existsSync(join(STORE, f))), "E6 idx-* caches exist after asks");
const metaPath = join(STORE, "idx-pool.json.meta");
const metaMtime = statSync(metaPath).mtimeMs;
const warm1 = askRaw(DECISION_QUERY, []);
const warm2 = askRaw(DECISION_QUERY, []);
check(statSync(metaPath).mtimeMs === metaMtime, "E6 second ask.mjs run hits the warm M0 cache (idx meta untouched)");
check(warm1 === warm2, "E6 warm asks byte-identical");

console.log(failed ? `FAILED ${failed}` : "ALL PASS");
console.log(`e2e runtime ${((Date.now() - started) / 1000).toFixed(1)}s`);
process.exit(failed ? 1 : 0);
