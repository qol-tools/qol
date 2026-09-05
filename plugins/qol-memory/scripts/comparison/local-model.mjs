import { spawn } from "node:child_process";
import { openSync, closeSync } from "node:fs";
import { createServer } from "node:net";
import { once } from "node:events";
import { setTimeout as delay } from "node:timers/promises";
import { join } from "node:path";

export function localEndpoint(value) {
  const endpoint = new URL(value);
  if (endpoint.protocol !== "http:" || !["127.0.0.1", "[::1]"].includes(endpoint.hostname)
      || endpoint.username || endpoint.password || endpoint.pathname !== "/" || endpoint.search || endpoint.hash) {
    throw new Error("The verifier endpoint must be a loopback HTTP origin");
  }
  return endpoint.origin;
}

export async function api(endpoint, path, body, timeout = 120_000) {
  const response = await fetch(`${endpoint}/api/${path}`, {
    method: body === undefined ? "GET" : "POST", redirect: "error",
    headers: { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(timeout),
  });
  const value = await response.json();
  if (!response.ok || value.error) throw new Error(value.error ?? `Model HTTP ${response.status}`);
  return value;
}

async function freePort() {
  const server = createServer();
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const port = server.address().port;
  await new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
  return port;
}

export async function withLocalModel(settings, out, report, work) {
  let child;
  let closed;
  let log;
  let spawnError;
  const endpoint = localEndpoint(settings.endpoint ?? `http://127.0.0.1:${await freePort()}`);
  try {
    if (!settings.endpoint) {
      log = openSync(join(out, "ollama.log"), "w");
      child = spawn("ollama", ["serve"], {
        env: { ...process.env, OLLAMA_HOST: endpoint, OLLAMA_NO_CLOUD: "1", OLLAMA_NUM_PARALLEL: "1", OLLAMA_MAX_LOADED_MODELS: "1" },
        stdio: ["ignore", log, log],
      });
      child.on("error", error => { spawnError = error; });
      closed = new Promise(resolve => child.once("close", (code, signal) => resolve({ code, signal })));
      const deadline = performance.now() + 10_000;
      for (;;) {
        if (spawnError) throw spawnError;
        if (child.exitCode !== null) throw new Error("Isolated model server exited during startup");
        try { await api(endpoint, "version"); break; } catch (error) {
          if (performance.now() >= deadline) throw error;
          await delay(200);
        }
      }
    }
    let registry = await api(endpoint, "tags");
    if (settings.prepare && !registry.models.some(model => model.name === settings.model && model.digest === settings.digest)) {
      process.stdout.write(`Preparing ${settings.model}\n`);
      await api(endpoint, "pull", {model: settings.model, stream: false}, 1_800_000);
      registry = await api(endpoint, "tags");
    }
    const model = registry.models.find(model => model.name === settings.model);
    if (!model || model.remote_model || model.remote_host || model.name.includes("cloud")) throw new Error("Expected an installed local model");
    if (model.digest !== settings.digest) throw new Error("Installed model differs from the frozen verification profile");
    report.inputs.provider = { endpoint, model, version: await api(endpoint, "version"), owned_server: Boolean(child) };
    await work(endpoint);
    const after = (await api(endpoint, "tags")).models.find(model => model.name === settings.model);
    if (after?.digest !== model.digest) throw new Error("Model changed during verification");
    report.inputs.provider.residency = await api(endpoint, "ps");
  } finally {
    if (child) {
      child.kill("SIGTERM");
      const deadline = setTimeout(() => child.kill("SIGKILL"), 5000);
      report.artifacts.server_exit = await closed;
      clearTimeout(deadline);
    }
    if (log !== undefined) closeSync(log);
  }
}
