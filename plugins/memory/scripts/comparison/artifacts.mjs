import { spawnSync } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { createReadStream, createWriteStream, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync, copyFileSync, chmodSync, renameSync } from "node:fs";
import { basename, join } from "node:path";
import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";

export const sha256 = (text) => createHash("sha256").update(text).digest("hex");
export const readJson = (path) => JSON.parse(readFileSync(path, "utf8"));
export const writeJson = (path, value) => writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);

export async function hashFile(path) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest("hex");
}

export async function prepareModels(registry, cache, offline) {
  const result = {};
  for (const [name, model] of Object.entries(registry)) {
    const directory = join(cache, model.revision);
    mkdirSync(directory, { recursive: true });
    for (const [file, expected] of Object.entries(model.files)) {
      const path = join(directory, file);
      if (existsSync(path) && await hashFile(path) === expected) continue;
      if (offline) throw new Error(`Missing or invalid model file ${path}; rerun without --offline`);
      process.stdout.write(`Downloading ${model.repository}/${file}\n`);
      const response = await fetch(`https://huggingface.co/${model.repository}/resolve/${model.revision}/${file}`);
      if (!response.ok || !response.body) throw new Error(`Model download failed: HTTP ${response.status}`);
      const temporary = `${path}.${randomUUID()}.part`;
      try {
        await pipeline(Readable.fromWeb(response.body), createWriteStream(temporary, { flags: "wx" }));
        if (await hashFile(temporary) !== expected) throw new Error(`Model checksum mismatch: ${file}`);
        renameSync(temporary, path);
      } finally {
        rmSync(temporary, { force: true });
      }
    }
    result[name] = directory;
  }
  return result;
}

export function runCommand(root, out, report, name, argv, input) {
  process.stdout.write(`${name}\n`);
  const start = performance.now();
  const result = spawnSync(argv[0], argv.slice(1), {
    cwd: root, encoding: "utf8", input: input === undefined ? undefined : JSON.stringify(input),
    maxBuffer: 64 * 1024 * 1024,
    env: { ...process.env, TOKENIZERS_PARALLELISM: "false" },
  });
  const log = join(out, `${name}.log`);
  writeFileSync(log, `${result.stdout ?? ""}${result.stderr ?? ""}${result.error ?? ""}`);
  report.commands.push({ name, argv, exit_code: result.status, signal: result.signal, duration_ms: performance.now() - start, log });
  if (result.status !== 0) throw new Error(`${name} failed; see ${log}`);
  return result.stdout;
}

export async function buildWorker(root, out, report, name, args) {
  const stdout = runCommand(root, out, report, `build-${name}`, ["cargo", "build", "--locked", ...args, "--message-format=json"]);
  const artifacts = stdout.split("\n").filter((line) => line.startsWith("{")).map((line) => JSON.parse(line));
  const built = artifacts.findLast((entry) => entry.reason === "compiler-artifact" && entry.target?.name === name && entry.executable);
  if (!built) throw new Error(`Cargo did not report an executable for ${name}`);
  const directory = join(out, "bin");
  mkdirSync(directory, { recursive: true });
  const path = join(directory, basename(built.executable));
  copyFileSync(built.executable, path);
  chmodSync(path, 0o755);
  const digest = await hashFile(path);
  if (digest !== await hashFile(built.executable)) throw new Error("Build artifact changed during snapshot");
  report.artifacts[name] = { path, sha256: digest, compiler_artifact: built };
  return path;
}
