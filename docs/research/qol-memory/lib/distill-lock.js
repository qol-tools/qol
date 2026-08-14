import { openSync, closeSync, readFileSync, unlinkSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const STALE_MS = 10 * 60 * 1000;

export function acquireDistillLock(storeRoot, mode) {
  const path = join(storeRoot, ".distill.lock");
  let released = false;
  for (let attempt = 0; attempt < 2; attempt++) {
    try {
      const fd = openSync(path, "wx");
      closeSync(fd);
      writeFileSync(path, JSON.stringify({ pid: process.pid, started_at: new Date().toISOString(), mode }) + "\n");
      return {
        release: () => {
          if (released) return;
          released = true;
          try {
            unlinkSync(path);
          } catch {}
        },
      };
    } catch (e) {
      if (e.code !== "EEXIST") throw e;
      let stale = false;
      try {
        const lock = JSON.parse(readFileSync(path, "utf8"));
        stale = lock && typeof lock.started_at === "string" && Date.now() - new Date(lock.started_at).getTime() > STALE_MS;
      } catch {}
      if (stale) {
        try {
          unlinkSync(path);
        } catch {}
        continue;
      }
      return null;
    }
  }
  return null;
}
