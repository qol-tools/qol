import { spawn } from 'node:child_process';
import { existsSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { createConnection } from 'node:net';
import { tmpdir } from 'node:os';
import { basename, dirname, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const pluginDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repoDir = resolve(pluginDir, '../..');
const binary = resolve(process.argv[2] || resolve(repoDir, 'target/debug/plugin-bluetooth'));
const runId = new Date().toISOString().replaceAll(':', '-').replaceAll('.', '-');
const reportDir = resolve(pluginDir, 'reports/bluetooth-search', runId);
const reportPath = resolve(reportDir, 'report.json');
const socketPath = resolve(tmpdir(), `qol-bt-smoke-${process.pid}.sock`);
const startedAt = new Date().toISOString();
const delay = (milliseconds) => new Promise((resolveDelay) => setTimeout(resolveDelay, milliseconds));

function action(socket, name) {
    return new Promise((resolveAction, rejectAction) => {
        const connection = createConnection(socket);
        let response = '';
        const timeout = setTimeout(() => {
            connection.destroy();
            rejectAction(new Error(`daemon action timed out: ${name}`));
        }, 2000);
        connection.setEncoding('utf8');
        connection.on('data', (chunk) => {
            response += chunk;
        });
        connection.once('error', (error) => {
            clearTimeout(timeout);
            rejectAction(error);
        });
        connection.once('end', () => {
            clearTimeout(timeout);
            try {
                const parsed = JSON.parse(response.trim());
                if (parsed.status !== 'handled') {
                    rejectAction(new Error(`daemon rejected ${name}: ${parsed.message || parsed.status}`));
                    return;
                }
                resolveAction(parsed.data ?? null);
            } catch (error) {
                rejectAction(error);
            }
        });
        connection.once('connect', () => {
            connection.end(`${JSON.stringify({ action: name })}\n`);
        });
    });
}

async function waitFor(check, timeoutMs) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
        const value = await check().catch(() => null);
        if (value) return value;
        await delay(50);
    }
    return null;
}

async function stopChild(child) {
    if (child.exitCode !== null) return;
    child.kill('SIGTERM');
    await Promise.race([
        new Promise((resolveExit) => child.once('exit', resolveExit)),
        delay(500),
    ]);
    if (child.exitCode !== null) return;
    child.kill('SIGKILL');
}

function numericDelta(before, after) {
    const amount = after - before;
    const percent = ((amount / before) * 100).toFixed(1);
    return `${amount} ms (${percent}%)`;
}

if (process.platform !== 'linux') {
    throw new Error('Bluetooth daemon smoke requires Linux and BlueZ');
}
if (!existsSync(binary)) {
    throw new Error(`binary not found: ${binary}`);
}

mkdirSync(reportDir, { recursive: true });
rmSync(socketPath, { force: true });
const child = spawn(binary, [], {
    cwd: pluginDir,
    env: {
        ...process.env,
        QOL_TRAY_DAEMON_REPLACE_EXISTING: '1',
        QOL_TRAY_DAEMON_SOCKET: socketPath,
    },
    stdio: ['ignore', 'ignore', 'pipe'],
});
let stderr = '';
child.stderr.setEncoding('utf8');
child.stderr.on('data', (chunk) => {
    stderr += chunk;
});

let status = 'failed';
let failure = null;
let startAckMs = null;
let activeAfterDeadline = false;
let stoppedAfterCancel = false;
let inventory = null;

try {
    const ready = await waitFor(() => action(socketPath, 'ping').then(() => true), 5000);
    if (!ready) throw new Error('daemon did not become ready');

    const started = performance.now();
    await action(socketPath, 'start_search');
    startAckMs = Math.round(performance.now() - started);
    const searching = await waitFor(async () => {
        const snapshot = await action(socketPath, 'search_status');
        return snapshot?.searching ? snapshot : null;
    }, 15000);
    if (!searching) throw new Error('search did not enter the active state');

    await delay(5500);
    const heldStatus = await action(socketPath, 'search_status');
    activeAfterDeadline = heldStatus?.searching === true;
    if (!activeAfterDeadline) throw new Error('search stopped without an explicit cancellation');

    const devices = await action(socketPath, 'devices');
    const items = Array.isArray(devices?.items) ? devices.items : [];
    const pairedRows = items.filter((item) => item?.paired === true).length;
    inventory = {
        connected_count: devices?.connected_count ?? null,
        count: devices?.count ?? null,
        paired_count: devices?.paired_count ?? null,
        paired_rows: pairedRows,
        searching: devices?.searching ?? null,
    };
    if (inventory.paired_count !== pairedRows) {
        throw new Error('unified inventory omitted one or more paired rows');
    }

    await action(socketPath, 'stop_search');
    const stopped = await waitFor(async () => {
        const snapshot = await action(socketPath, 'search_status');
        return snapshot?.searching === false ? snapshot : null;
    }, 5000);
    stoppedAfterCancel = Boolean(stopped);
    if (!stoppedAfterCancel) throw new Error('search remained active after cancellation');
    status = 'pass';
} catch (error) {
    failure = error instanceof Error ? error.message : String(error);
} finally {
    await action(socketPath, 'kill').catch(() => {});
    await stopChild(child);
    rmSync(socketPath, { force: true });
}

const context = 'Linux/BlueZ, isolated daemon socket, default adapter, 1 run';
const evidence = relative(repoDir, reportPath);
const metrics = [
    {
        improvement_vector: 'continuous Bluetooth discovery',
        scenario: 'Start search from the daemon action surface',
        context,
        metric: 'action acknowledgement latency',
        before: '5026 ms',
        after: startAckMs === null ? 'not measured' : `${startAckMs} ms`,
        delta: startAckMs === null ? 'N/A' : numericDelta(5026, startAckMs),
        correctness: startAckMs !== null && startAckMs < 1000 ? 'passed' : 'failed',
        evidence,
    },
    {
        improvement_vector: 'continuous Bluetooth discovery',
        scenario: 'Leave search running beyond the former deadline',
        context,
        metric: 'active after 5.5 seconds',
        before: 'failed',
        after: activeAfterDeadline ? 'passed' : 'failed',
        delta: 'N/A',
        correctness: activeAfterDeadline ? 'passed' : 'failed',
        evidence,
    },
    {
        improvement_vector: 'explicit Bluetooth discovery cancellation',
        scenario: 'Stop search from the daemon action surface',
        context,
        metric: 'idle state after cancellation',
        before: 'unavailable',
        after: stoppedAfterCancel ? 'passed' : 'failed',
        delta: 'N/A',
        correctness: stoppedAfterCancel ? 'passed' : 'failed',
        evidence,
    },
    {
        improvement_vector: 'paired-device visibility',
        scenario: 'Query the unified device inventory while searching',
        context,
        metric: 'paired rows represented in inventory',
        before: 'split payloads',
        after: inventory === null ? 'not measured' : `${inventory.paired_rows}/${inventory.paired_count} rows`,
        delta: 'N/A',
        correctness: inventory !== null && inventory.paired_rows === inventory.paired_count ? 'passed' : 'failed',
        evidence,
    },
];
const report = {
    name: 'bluetooth-daemon-search-smoke',
    started_at: startedAt,
    finished_at: new Date().toISOString(),
    status,
    inputs: {
        binary: basename(binary),
        platform: process.platform,
        socket: 'isolated temporary socket',
    },
    artifacts: {
        report: evidence,
    },
    commands: ['ping', 'start_search', 'search_status', 'devices', 'stop_search', 'kill'],
    observations: {
        active_after_5500_ms: activeAfterDeadline,
        inventory,
        start_ack_ms: startAckMs,
        stopped_after_cancel: stoppedAfterCancel,
    },
    metrics,
    failure,
    diagnostics: stderr.trim() ? 'daemon wrote diagnostic output; rerun directly to inspect' : null,
    next: status === 'pass' ? [] : ['Inspect daemon diagnostics and rerun make smoke-daemon'],
};
writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
console.log(JSON.stringify({ report: evidence, status, observations: report.observations }, null, 2));
if (status !== 'pass') process.exitCode = 1;
