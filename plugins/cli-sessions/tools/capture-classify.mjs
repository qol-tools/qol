#!/usr/bin/env node
import { execFileSync } from "node:child_process";
import { writeFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, "..", "..", "..");
const PLUGIN = resolve(REPO, "plugins", "cli-sessions");
const BIN = resolve(REPO, "target", "debug", "examples", "classify");

function kitten(args) {
  return execFileSync("kitten", args, { encoding: "utf8", maxBuffer: 64 << 20 });
}

function windows() {
  const ls = JSON.parse(kitten(["@", "ls"]));
  const out = [];
  for (const osw of ls)
    for (const tab of osw.tabs)
      for (const w of tab.windows) {
        const fg = (w.foreground_processes || []).map((p) =>
          (p.cmdline?.[0] || "").split("/").pop()
        );
        out.push({
          id: w.id,
          title: w.title || "",
          at_prompt: !!(w.is_at_prompt ?? w.at_prompt),
          foreground_basenames: fg,
        });
      }
  return out;
}

function frameFor(w) {
  const screen = kitten([
    "@", "get-text", "--match", `id:${w.id}`, "--extent", "screen",
  ]);
  return { ...w, screen };
}

function classify(frame) {
  const payload = JSON.stringify({
    title: frame.title,
    at_prompt: frame.at_prompt,
    foreground_basenames: frame.foreground_basenames,
    screen: frame.screen,
  });
  return execFileSync(BIN, [], { input: payload, encoding: "utf8" }).trim();
}

const mode = process.argv[2] || "once";

if (mode === "snapshot") {
  const id = process.argv[3];
  const label = process.argv[4] || `win${id}`;
  const expect = process.argv[5] || null;
  const dir = `${PLUGIN}/tests/fixtures/corpus`;
  mkdirSync(dir, { recursive: true });
  const w = windows().find((x) => String(x.id) === String(id));
  if (!w) throw new Error(`no window ${id}`);
  const frame = frameFor(w);
  writeFileSync(`${dir}/${label}.txt`, frame.screen);
  writeFileSync(
    `${dir}/${label}.meta.json`,
    JSON.stringify({ title: w.title, at_prompt: w.at_prompt, foreground_basenames: w.foreground_basenames, expect }, null, 2)
  );
  console.log(`saved ${label} (${frame.screen.split("\n").length} lines) expect=${expect} -> ${classify(frame)}`);
} else if (mode === "watch") {
  const seconds = Number(process.argv[3] || 600);
  const dir = `${PLUGIN}/tests/fixtures/suspects`;
  mkdirSync(dir, { recursive: true });
  const deadline = Date.now() + seconds * 1000;
  const seen = new Map();
  // Mirror Rust title_working exactly: braille spinner or busy-star 2734..273F.
  // 2733 (parked star) is deliberately excluded - it means done, not busy.
  const titleLooksBusy = (t) => {
    const c = (t.trimStart().codePointAt(0) || 0);
    return (c >= 0x2800 && c <= 0x28ff) || (c >= 0x2734 && c <= 0x273f);
  };
  let saved = 0;
  while (Date.now() < deadline) {
    for (const w of windows()) {
      const fg = w.foreground_basenames;
      if (!fg.includes("claude")) continue;
      const frame = frameFor(w);
      const result = classify(frame);
      const status = /status=(\w+)/.exec(result)?.[1];
      const stamp = new Date().toISOString().slice(11, 19);
      // The smoking gun: any frame that classifies NeedsYou. Save every one so
      // each can be judged real-prompt vs false-positive. Also save a desync
      // (title still a live spinner but classified not-working) - the window in
      // which a working frame can momentarily fall through to a choice read.
      const desync = titleLooksBusy(w.title) && status !== "Working" && status !== "Service";
      const reason = status === "NeedsYou" ? "NEEDSYOU" : desync ? "DESYNC" : null;
      if (reason) {
        const label = `${reason.toLowerCase()}_win${w.id}_${stamp.replace(/:/g, "")}`;
        writeFileSync(`${dir}/${label}.txt`, frame.screen);
        writeFileSync(`${dir}/${label}.meta.json`, JSON.stringify({ title: w.title, at_prompt: w.at_prompt, foreground_basenames: fg, result }, null, 2));
        console.log(`${stamp} win${w.id} ${result} <== ${reason} saved ${label}`);
        saved++;
      } else if (seen.get(w.id) !== status) {
        const flag = status === "YourTurn" ? " <== your turn" : "";
        console.log(`${stamp} win${w.id} ${result}${flag}`);
        seen.set(w.id, status);
      }
    }
    execFileSync("sleep", ["2"]);
  }
  console.log(`watch done; ${saved} suspect frames saved to ${dir}`);
} else {
  for (const w of windows()) {
    const fg = w.foreground_basenames;
    if (!fg.includes("claude")) continue;
    console.log(`win${w.id} title=${JSON.stringify(w.title)} -> ${classify(frameFor(w))}`);
  }
}
