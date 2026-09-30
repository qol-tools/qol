import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, readFileSync, renameSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const script = fileURLToPath(new URL('./repository-inventory.mjs', import.meta.url));
const repository = resolve(dirname(script), '..');

function fixture(t) {
    const root = mkdtempSync(join(tmpdir(), 'qol inventory '));
    t.after(() => rmSync(root, { recursive: true, force: true }));
    const checkout = join(root, 'checkout');
    execFileSync('git', ['clone', '--quiet', '--shared', '--no-checkout', repository, checkout]);
    const write = (path, content) => {
        mkdirSync(dirname(join(checkout, path)), { recursive: true });
        writeFileSync(join(checkout, path), content);
    };
    write('.gitignore', '/ignored/\n/reports/\n');
    write('Cargo.toml', '[workspace]\nmembers = ["libs/example"]\nresolver = "2"\n');
    write('libs/example/Cargo.toml', '[package]\nname = "inventory-example"\nversion = "0.1.0"\nedition = "2021"\n');
    const body = Array.from({ length: 16 }, (_, i) => `    let repeated_value_${i} = ${i};`).join('\n');
    write('libs/example/src/lib.rs', `pub fn first() {\n${body}\n}\n`);
    write('libs/example/src/other.rs', `pub fn second() {\n${body}\n}\n`);
    write('one file.txt', 'identical content\n');
    write('second\nfile.txt', 'identical content\n');
    const core = Buffer.alloc(18);
    core.set([127, 69, 76, 70, 2, 1]);
    core.writeUInt16LE(4, 16);
    write('accidental-core', core);
    write('ignored/large-build-output', 'excluded\n');
    execFileSync('cargo', ['generate-lockfile', '--offline'], { cwd: checkout, stdio: 'pipe' });
    execFileSync('git', ['add', '--all'], { cwd: checkout });
    write('untracked.txt', 'not staged\n');
    return { checkout, root, write };
}

function inventory(checkout, out = 'reports/inventory') {
    const result = spawnSync(process.execPath, [script, '--root', checkout, '--out', out], { encoding: 'utf8' });
    return { ...result, reportPath: resolve(checkout, out, 'report.json') };
}

test('inventories ownership, duplicates, ignored boundaries and filenames without shell parsing', t => {
    const { checkout } = fixture(t);
    const run = inventory(checkout);
    assert.equal(run.status, 0, run.stderr);
    const report = JSON.parse(readFileSync(run.reportPath, 'utf8'));
    const names = report.files.map(file => file.path);
    assert.deepEqual(names, ['.gitignore', 'Cargo.lock', 'Cargo.toml', 'accidental-core',
        'libs/example/Cargo.toml', 'libs/example/src/lib.rs', 'libs/example/src/other.rs',
        'one file.txt', 'second\nfile.txt', 'untracked.txt']);
    assert.equal(report.packages[0].name, 'inventory-example');
    assert.equal(report.files.find(file => file.path.endsWith('/lib.rs')).package, 'inventory-example');
    assert.equal(report.files.find(file => file.path === 'untracked.txt').tracking, 'untracked');
    assert.deepEqual(report.exact_duplicates[0].paths, ['one file.txt', 'second\nfile.txt']);
    assert.deepEqual(report.core_dumps, ['accidental-core']);
    assert.ok(report.ignored_boundaries.includes('ignored/'));
    assert.equal(report.repeated_blocks[0].significant_lines, 17);
    assert.deepEqual(report.repeated_blocks[0].occurrences.map(row => row.start_line), [2, 2]);
    const table = readFileSync(join(dirname(run.reportPath), 'files.tsv'), 'utf8');
    assert.equal(table.trimEnd().split('\n').length, names.length + 1);
    assert.ok(table.includes('second\\nfile.txt'));
    assert.equal(inventory(checkout).status, 0);
    assert.deepEqual(JSON.parse(readFileSync(run.reportPath, 'utf8')), report);
});

test('lists missing tracked files as problems and exits unsuccessfully', t => {
    const { checkout } = fixture(t);
    rmSync(join(checkout, 'one file.txt'));
    const run = inventory(checkout);
    assert.equal(run.status, 1);
    const report = JSON.parse(readFileSync(run.reportPath, 'utf8'));
    assert.deepEqual(report.problems.map(problem => problem.path), ['one file.txt']);
    assert.equal(report.files.find(file => file.path === 'one file.txt').type, 'unreadable');
});

test('refuses an output directory among source files', t => {
    const { checkout } = fixture(t);
    const run = inventory(checkout, 'libs/example');
    assert.notEqual(run.status, 0);
    assert.match(run.stderr, /must be Git-ignored/);
});

test('records symlink text without following targets and refuses output symlinks', { skip: process.platform === 'win32' }, t => {
    const { checkout, root, write } = fixture(t);
    const outside = join(root, 'outside.txt');
    writeFileSync(outside, 'do not read or overwrite\n');
    symlinkSync(outside, join(checkout, 'external-link'));
    symlinkSync('does-not-exist', join(checkout, 'dangling-link'));
    const run = inventory(checkout);
    assert.equal(run.status, 0, run.stderr);
    const report = JSON.parse(readFileSync(run.reportPath, 'utf8'));
    assert.equal(report.files.find(file => file.path === 'external-link').bytes, Buffer.byteLength(outside));
    assert.equal(report.files.find(file => file.path === 'dangling-link').type, 'symlink');
    write('reports/unsafe/.keep', '');
    symlinkSync(outside, join(checkout, 'reports/unsafe/report.json'));
    assert.notEqual(inventory(checkout, 'reports/unsafe').status, 0);
    assert.equal(readFileSync(outside, 'utf8'), 'do not read or overwrite\n');
    rmSync(join(checkout, 'reports/unsafe/report.json'));
    symlinkSync(join(root, 'missing-output'), join(checkout, 'reports/unsafe/report.json'));
    assert.notEqual(inventory(checkout, 'reports/unsafe').status, 0);
});

test('rejects tracked files beneath symlinked directories without reading external content', { skip: process.platform === 'win32' }, t => {
    const { checkout, root, write } = fixture(t);
    const paths = ['linked/first.js', 'linked/second.js'];
    for (const path of paths) write(path, 'repository content\n');
    execFileSync('git', ['add', '--', 'linked'], { cwd: checkout });
    const outside = join(root, 'outside');
    renameSync(join(checkout, 'linked'), outside);
    const body = Array.from({ length: 16 }, (_, i) => `const private_value_${i} = "external-content-${i}";`).join('\n');
    writeFileSync(join(outside, 'first.js'), `function first() {\n${body}\n}\n`);
    writeFileSync(join(outside, 'second.js'), `function second() {\n${body}\n}\n`);
    symlinkSync(outside, join(checkout, 'linked'));

    const run = inventory(checkout);
    assert.equal(run.status, 1, run.stderr);
    const encoded = readFileSync(run.reportPath, 'utf8');
    const report = JSON.parse(encoded);
    assert.deepEqual(report.problems.map(problem => problem.path), paths);
    for (const path of paths) {
        const file = report.files.find(file => file.path === path);
        assert.equal(file.type, 'unreadable', path);
        assert.equal(file.sha256, null, path);
        assert.equal(file.bytes, 0, path);
    }
    assert.ok(!encoded.includes('external-content-'));
    assert.ok(!report.repeated_blocks.some(block => block.occurrences.some(row => paths.includes(row.path))));
});
