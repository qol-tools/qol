import { readFileSync } from 'node:fs';

const compact = process.argv.includes('--compact');
const input = process.argv.slice(2).find((argument) => argument !== '--compact');
if (!input) throw new Error('usage: render-contextual-metrics.mjs [--compact] <report.json>');
const report = JSON.parse(readFileSync(input, 'utf8'));
const rows = Array.isArray(report) ? report : report.metrics;
if (!Array.isArray(rows)) throw new Error('input must be a metrics array or a report with metrics');

function cell(value) {
    return String(value ?? '').replaceAll('|', '\\|').replaceAll('\n', ' ');
}

if (!compact) {
    console.log('| Improvement Vector | Scenario | Context | Metric | Before | After | Delta | Correctness | Evidence |');
    console.log('| --- | --- | --- | --- | --- | --- | --- | --- | --- |');
    for (const row of rows) {
        console.log(`| ${cell(row.improvement_vector)} | ${cell(row.scenario)} | ${cell(row.context)} | ${cell(row.metric)} | ${cell(row.before)} | ${cell(row.after)} | ${cell(row.delta)} | ${cell(row.correctness)} | ${cell(row.evidence)} |`);
    }
    process.exit(0);
}

const groups = new Map();
for (const row of rows) {
    const key = JSON.stringify([row.improvement_vector, row.context, row.evidence]);
    const group = groups.get(key) || { row, rows: [] };
    group.rows.push(row);
    groups.set(key, group);
}
for (const { row, rows: groupedRows } of groups.values()) {
    console.log(`### ${cell(row.improvement_vector)}`);
    console.log('');
    console.log(`${cell(row.context)} · Evidence: ${cell(row.evidence)}`);
    console.log('');
    console.log('| Scenario | Metric | Before | After | Delta | Correctness |');
    console.log('| --- | --- | --- | --- | --- | --- |');
    for (const metric of groupedRows) {
        console.log(`| ${cell(metric.scenario)} | ${cell(metric.metric)} | ${cell(metric.before)} | ${cell(metric.after)} | ${cell(metric.delta)} | ${cell(metric.correctness)} |`);
    }
    console.log('');
}
