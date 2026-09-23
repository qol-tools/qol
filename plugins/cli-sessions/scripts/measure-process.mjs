import { readFile, readlink, mkdir, writeFile } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { dirname } from 'node:path';
import { performance } from 'node:perf_hooks';
import { setTimeout } from 'node:timers/promises';

const [pidText, secondsText = '60', output = 'target/cli-sessions-performance/process.json'] = process.argv.slice(2);
const pid = Number(pidText);
const seconds = Number(secondsText);
if (!Number.isInteger(pid) || pid <= 0 || !Number.isInteger(seconds) || seconds < 1 || seconds > 3600) {
    throw new Error('Usage: node measure-process.mjs PID [SECONDS=60] [REPORT_PATH]');
}
const root = `/proc/${pid}`;
const ticksPerSecond = Number(execFileSync('getconf', ['CLK_TCK'], { encoding: 'utf8' }).trim());
const executable = await readlink(`${root}/exe`);
const sha256 = createHash('sha256').update(await readFile(`${root}/exe`)).digest('hex');
async function sample() {
    const [rawStat, rawStatus] = await Promise.all([readFile(`${root}/stat`, 'utf8'), readFile(`${root}/status`, 'utf8')]);
    const stat = rawStat.slice(rawStat.lastIndexOf(')') + 2).trim().split(/\s+/);
    const status = Object.fromEntries(rawStatus.trim().split('\n').map(line => {
        const colon = line.indexOf(':');
        return [line.slice(0, colon), line.slice(colon + 1).trim()];
    }));
    return {
        timeMs: performance.now(),
        cpuTicks: Number(stat[11]) + Number(stat[12]),
        childTicks: Number(stat[13]) + Number(stat[14]),
        startTicks: stat[19],
        rssMiB: Number.parseInt(status.VmRSS, 10) / 1024,
    };
}
const samples = [await sample()];
for (let i = 0; i < seconds; i++) {
    await setTimeout(1000);
    const next = await sample();
    if (next.startTicks !== samples[0].startTicks) throw new Error('Process restarted during measurement');
    samples.push(next);
}
const finalHash = createHash('sha256').update(await readFile(`${root}/exe`)).digest('hex');
if (finalHash !== sha256) throw new Error('Executable changed during measurement');
const first = samples[0];
const last = samples.at(-1);
const durationSeconds = (last.timeMs - first.timeMs) / 1000;
const cpuPercent = (a, b) => 100 * (b.cpuTicks - a.cpuTicks) / ticksPerSecond / ((b.timeMs - a.timeMs) / 1000);
const report = {
    pid, executable, sha256, durationSeconds,
    cpuPercentOneCore: cpuPercent(first, last),
    reapedChildrenCpuPercentOneCore: 100 * (last.childTicks - first.childTicks) / ticksPerSecond / durationSeconds,
    peakOneSecondCpuPercent: Math.max(...samples.slice(1).map((sample, i) => cpuPercent(samples[i], sample))),
    rssStartMiB: first.rssMiB, rssEndMiB: last.rssMiB,
    rssPeakMiB: Math.max(...samples.map(sample => sample.rssMiB)),
};
await mkdir(dirname(output), { recursive: true });
await writeFile(output, JSON.stringify({ ...report, samples }, null, 2) + '\n');
console.log(JSON.stringify(report, null, 2));
