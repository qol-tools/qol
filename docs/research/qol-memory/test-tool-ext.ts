import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";
import { spawnSync } from "node:child_process";
import { join } from "node:path";
import { appendFileSync, readFileSync } from "node:fs";

export default function (pi: ExtensionAPI) {
  pi.registerTool({
    name: "qol_memory_retrieve",
    label: "QoL Memory Retrieve",
    description:
      "Retrieve past-work memory: what we decided, how we fixed things, corpus facts. Ask when the user references past sessions, decisions, or fixes you don't remember. Returns a verdict (answered/candidates/no-memory) with provenance. This is a recall tool - the answer is a fact from past transcripts, not a suggestion.",
    parameters: Type.Object({
      query: Type.String({ description: "What to recall, e.g. 'how did we fix the m4a1 anchoring'" }),
    }),
    async execute(toolCallId, params, signal, onUpdate, ctx) {
      try { appendFileSync("/tmp/qol-memory-tool-calls.log", new Date().toISOString() + " TOOL " + String(params.query || "").slice(0, 80) + "\n"); } catch {}
      const store = process.env.QOL_MEMORY_STORE || join(process.env.HOME || "", ".local", "share", "qol-tray", "plugins", "qol-memory");
      const manifest = join(store, "manifest.json");
      let askPath = "";
      try {
        const m = JSON.parse(readFileSync(manifest, "utf8"));
        if (m && m.ask_mjs) askPath = m.ask_mjs;
      } catch {}
      if (!askPath && process.env.QOL_MEMORY_ASK) askPath = process.env.QOL_MEMORY_ASK;
      if (!askPath) {
        return { content: [{ type: "text", text: "memory unavailable: no qol-memory manifest at " + manifest }], details: {} };
      }
      const r = spawnSync("node", [askPath, String(params.query || ""), "--brief"], {
        encoding: "utf8", timeout: 6000,
      });
      if (r.status !== 0 || !r.stdout) {
        return { content: [{ type: "text", text: "memory error: ask.mjs failed" }], details: {} };
      }
      let d;
      try { d = JSON.parse(r.stdout); } catch {
        return { content: [{ type: "text", text: "memory error: bad ask.mjs output" }], details: {} };
      }
      const lines: string[] = [];
      if (d.verdict === "answered" && d.answer) {
        const a = d.answer;
        const text = a.text || "";
        const cap = a.source_kind === "decision" ? 280 : 240;
        const truncated = text.length >= cap && !/[.!?\u2026"]\s*$/.test(text.trim());
        lines.push(`VERDICT: answered (${d.confidence})${truncated ? " - FACT IS TRUNCATED, retrieve again for the full note" : ""}`);
        lines.push(`FACT: ${text}${truncated ? " [truncated]" : ""}`);
        lines.push(`PROVENANCE: key ${a.key}, ${a.source_kind}, ${a.source_ts || "?"}`);
        if (a.superseded && a.superseded.length) {
          lines.push(`SUPERSEDES: ${a.superseded.map((s: any) => s.text).join(" | ")}`);
        }
      } else if (d.verdict === "candidates") {
        lines.push(`VERDICT: candidates (${d.confidence}) - no decisive fact, hints only`);
        for (const c of (d.recalled || []).slice(0, 3)) {
          lines.push(`HINT: key ${c.key}, ${c.source_kind}, ${c.source_ts || "?"}`);
        }
      } else {
        lines.push(`VERDICT: no-memory - nothing in the corpus answers this. Be honest about not knowing.`);
      }
      if (d.non_default_gates) lines.push("WARNING: non-default gates applied");
      return { content: [{ type: "text", text: lines.join("\n") }], details: {} };
    },
  });
}
