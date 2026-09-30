import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { lstatSync, mkdirSync, readFileSync, readlinkSync, realpathSync, writeFileSync } from 'node:fs';
import { dirname, extname, isAbsolute, relative, resolve, sep } from 'node:path';
import { parseArgs } from 'node:util';

const { values } = parseArgs({ options: {
    root: { type: 'string', default: process.cwd() },
    out: { type: 'string', default: 'verify/reports/repository-inventory' },
    help: { type: 'boolean', default: false },
} });

if (values.help) {
    process.stdout.write('node verify/repository-inventory.mjs [--root <checkout>] [--out <directory>]\n');
    process.exit(0);
}

function run(program, args, cwd) {
    return execFileSync(program, args, { cwd, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 });
}

const root = run('git', ['rev-parse', '--show-toplevel'], resolve(values.root)).trim();
const output = resolve(root, values.out);
const git = (...args) => run('git', args, root);
const posix = path => path.split(sep).join('/');
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const compare = (a, b) => a < b ? -1 : a > b ? 1 : 0;
const nulList = value => value.split('\0').filter(Boolean);
const outputRelative = relative(root, output);
const artifacts = ['report.json', 'files.tsv', 'directories.tsv', 'packages.tsv'];
if (!outputRelative || (!isAbsolute(outputRelative) && outputRelative !== '..' && !outputRelative.startsWith(`..${sep}`))) {
    if (!outputRelative) throw new Error('Output must be a separate directory.');
    try {
        git('check-ignore', '-q', '--', posix(outputRelative));
    } catch {
        throw new Error('Output inside the repository must be Git-ignored.');
    }
}
mkdirSync(output, { recursive: true });
if (realpathSync(output) !== output) throw new Error('Output directory must not traverse a symlink.');
for (const name of artifacts) {
    const path = resolve(output, name);
    const stat = lstatSync(path, { throwIfNoEntry: false });
    if (stat && !stat.isFile()) throw new Error(`Output must be a regular file: ${path}`);
}

function role(path) {
    if (path.startsWith('vendor/')) return 'vendor';
    if (path.startsWith('docs/')) return 'documentation-or-research';
    if (/(^|\/)(tests?|fixtures|examples|proptest-regressions)(\/|\.)/.test(path)) return 'test-or-example';
    if (path.startsWith('flows/')) return 'guest-environment';
    if (path.startsWith('.github/') || path.startsWith('.githooks/') || path.startsWith('verify/')) return 'automation';
    if (/(^|\/)(LICENSE[^/]*|README\.md)$/.test(path) || extname(path) === '.md') return 'documentation';
    if (['.rs', '.js', '.mjs', '.ts', '.py', '.sh'].includes(extname(path))) return 'source';
    if (['.toml', '.json', '.yml', '.yaml'].includes(extname(path))) return 'configuration-or-data';
    return 'asset-or-other';
}

function entries() {
    const tracked = nulList(git('ls-files', '--stage', '-z')).map(record => {
        const tab = record.indexOf('\t');
        const [mode, object, stage] = record.slice(0, tab).split(' ');
        if (stage !== '0') throw new Error('Resolve index conflicts before inventorying.');
        return { path: record.slice(tab + 1), tracking: 'tracked', mode, object };
    });
    const untracked = nulList(git('ls-files', '--others', '--exclude-standard', '-z'))
        .map(path => ({ path, tracking: 'untracked' }));
    return [...tracked, ...untracked].sort((a, b) => compare(a.path, b.path));
}

const metadata = JSON.parse(run('cargo', ['metadata', '--no-deps', '--format-version', '1', '--locked', '--offline'], root));
const members = new Set(metadata.workspace_members);
const packageNames = new Set(metadata.packages.filter(pkg => members.has(pkg.id)).map(pkg => pkg.name));
const packages = metadata.packages.filter(pkg => members.has(pkg.id)).map(pkg => ({
    name: pkg.name,
    path: posix(relative(root, dirname(pkg.manifest_path))),
    manifest: posix(relative(root, pkg.manifest_path)),
    description: pkg.description,
    targets: pkg.targets.map(target => ({ name: target.name, kind: target.kind, path: posix(relative(root, target.src_path)) })),
    workspace_dependencies: pkg.dependencies.filter(dep => packageNames.has(dep.name)).map(dep => ({
        name: dep.name, kind: dep.kind ?? 'normal', target: dep.target, optional: dep.optional,
    })),
})).sort((a, b) => compare(a.path, b.path));
const owners = [...packages].sort((a, b) => b.path.length - a.path.length);
const sourceLines = new Map();
const problems = [];

function decode(bytes) {
    if (bytes.includes(0)) return null;
    try {
        return new TextDecoder('utf-8', { fatal: true }).decode(bytes);
    } catch {
        return null;
    }
}

function binaryHint(bytes) {
    if (bytes.length < 18 || !bytes.subarray(0, 4).equals(Buffer.from([127, 69, 76, 70]))) return null;
    const type = bytes[5] === 1 ? bytes.readUInt16LE(16) : bytes.readUInt16BE(16);
    return type === 4 ? 'elf-core-dump' : 'elf-binary';
}

function inspect(entry) {
    const path = resolve(root, entry.path);
    const base = { ...entry, role: role(entry.path), package: owners.find(pkg => entry.path.startsWith(`${pkg.path}/`))?.name ?? null };
    if (entry.mode === '160000') return { ...base, type: 'gitlink', bytes: 0, lines: null, sha256: null };
    try {
        const stat = lstatSync(path);
        const type = stat.isSymbolicLink() ? 'symlink' : 'file';
        if (!stat.isSymbolicLink() && !stat.isFile()) throw new Error('Expected a file or symlink');
        const bytes = type === 'symlink' ? Buffer.from(readlinkSync(path)) : readFileSync(path);
        const text = decode(bytes);
        const lines = text === null ? null : text.split('\n').length - Number(text.endsWith('\n') || !text);
        if (type === 'file' && text !== null && !entry.path.startsWith('vendor/') && /\.(rs|js|mjs|ts|py|sh)$/.test(entry.path)) {
            const significant = text.split('\n').map((line, index) => ({ text: line.trim(), line: index + 1 }))
                .filter(line => line.text && !line.text.startsWith('//') && !line.text.startsWith('# '));
            if (significant.every(line => line.text.length < 1000)) sourceLines.set(entry.path, significant);
        }
        return { ...base, type, bytes: bytes.length, lines, sha256: hash(bytes), binary_hint: binaryHint(bytes) };
    } catch (error) {
        problems.push({ path: entry.path, error: error.message });
        return { ...base, type: 'unreadable', bytes: 0, lines: null, sha256: null };
    }
}

function groupBy(rows, key) {
    const groups = new Map();
    for (const row of rows) {
        const id = key(row);
        if (!groups.has(id)) groups.set(id, []);
        groups.get(id).push(row);
    }
    return groups;
}

function duplicateKind(paths) {
    if (paths.every(path => /(^|\/)LICENSE/.test(path))) return 'licenses';
    if (paths.every(path => /(^|\/)(tests|fixtures)\//.test(path))) return 'test-fixtures';
    if (paths.every(path => path.includes('/platform/'))) return 'platform-adapters';
    if (paths.every(path => path.endsWith('/build.rs'))) return 'build-entrypoints';
    if (paths.every(path => path.includes('/ui/lib/'))) return 'bundled-web-libraries';
    if (paths.every(path => path.endsWith('.md'))) return 'documentation';
    return 'review';
}

const files = entries().map(inspect);
const exactDuplicates = [...groupBy(files.filter(file => file.sha256), file => `${file.type}:${file.sha256}`).values()]
    .filter(group => group.length > 1).map(group => ({
        sha256: group[0].sha256,
        bytes_each: group[0].bytes,
        repeated_bytes: group[0].bytes * (group.length - 1),
        kind_hint: duplicateKind(group.map(file => file.path)),
        paths: group.map(file => file.path),
    })).sort((a, b) => b.repeated_bytes - a.repeated_bytes || compare(a.paths[0], b.paths[0]));

function repeatedBlocks() {
    const width = 12;
    const windows = new Map();
    for (const [path, lines] of sourceLines) {
        for (let index = 0; index + width <= lines.length; index++) {
            const block = lines.slice(index, index + width).map(line => line.text);
            const text = block.join('\n');
            if (text.length < 240 || block.filter(line => /[a-zA-Z0-9]/.test(line)).length < 8) continue;
            const id = hash(text);
            if (!windows.has(id)) windows.set(id, []);
            windows.get(id).push({ path, index });
        }
    }
    const hashes = new Map(files.map(file => [file.path, file.sha256]));
    return [...windows.values()].filter(rows => new Set(rows.map(row => hashes.get(row.path))).size > 1)
        .filter(rows => !extendsPreviousWindow(rows, width, windows))
        .map(rows => extendBlock(rows, width))
        .sort((a, b) => b.significant_lines * b.occurrences.length - a.significant_lines * a.occurrences.length || compare(a.occurrences[0].path, b.occurrences[0].path));
}

function extendsPreviousWindow(rows, width, windows) {
    if (rows.some(row => row.index === 0)) return false;
    const first = rows[0];
    const previousText = sourceLines.get(first.path).slice(first.index - 1, first.index + width - 1).map(line => line.text).join('\n');
    const previous = windows.get(hash(previousText));
    return previous?.length === rows.length && previous.every((row, i) => row.path === rows[i].path && row.index + 1 === rows[i].index);
}

function extendBlock(rows, width) {
    let length = width;
    while (rows.every(row => sourceLines.get(row.path)[row.index + length]) &&
        new Set(rows.map(row => sourceLines.get(row.path)[row.index + length].text)).size === 1) length++;
    const first = rows[0];
    return {
        significant_lines: length,
        sample: sourceLines.get(first.path).slice(first.index, first.index + Math.min(length, 3)).map(line => line.text).join('\n'),
        occurrences: rows.map(row => ({
            path: row.path,
            start_line: sourceLines.get(row.path)[row.index].line,
            end_line: sourceLines.get(row.path)[row.index + length - 1].line,
        })),
    };
}

function directories() {
    const rows = new Map([['.', { path: '.', files: 0, bytes: 0 }]]);
    for (const file of files) {
        for (let path = posix(dirname(file.path));; path = posix(dirname(path))) {
            if (!rows.has(path)) rows.set(path, { path, files: 0, bytes: 0 });
            rows.get(path).files++;
            rows.get(path).bytes += file.bytes;
            if (path === '.') break;
        }
    }
    return [...rows.values()].sort((a, b) => compare(a.path, b.path));
}

const report = {
    schema_version: 1,
    commit: git('rev-parse', 'HEAD').trim(),
    status_porcelain: git('status', '--porcelain=v1', '--untracked-files=all'),
    scope: {
        files: 'Git tracked plus nonignored untracked files in this worktree; symlinks hash link text without following targets.',
        ignored: 'Ignored paths are listed as boundaries, not hashed or recursively inventoried; Git internals are excluded.',
        blocks: 'Heuristic cross-file matches of 12+ consecutive nonblank lines, indentation trimmed, whole-line // and # comments skipped. Identifiers and literals are retained. Vendor and minified code are excluded. Matches may overlap or belong to tests; they are not proof of shared ownership.',
    },
    summary: { files: files.length, tracked_files: files.filter(file => file.tracking === 'tracked').length, bytes: files.reduce((total, file) => total + file.bytes, 0), packages: packages.length, exact_duplicate_groups: exactDuplicates.length },
    packages,
    other_manifests: files.filter(file => file.path.endsWith('Cargo.toml') && !packages.some(pkg => pkg.manifest === file.path)).map(file => file.path),
    files,
    directories: directories(),
    exact_duplicates: exactDuplicates,
    core_dumps: files.filter(file => file.binary_hint === 'elf-core-dump').map(file => file.path),
    repeated_blocks: repeatedBlocks(),
    ignored_boundaries: nulList(git('ls-files', '--others', '--ignored', '--exclude-standard', '--directory', '-z')).filter(path => !path.startsWith(`${posix(outputRelative)}/`)),
    problems,
};

function tsv(rows, columns) {
    const cell = value => String(value ?? '').replaceAll('\\', '\\\\').replaceAll('\t', '\\t').replaceAll('\r', '\\r').replaceAll('\n', '\\n');
    return [columns.join('\t'), ...rows.map(row => columns.map(column => cell(row[column])).join('\t'))].join('\n') + '\n';
}

writeFileSync(resolve(output, 'report.json'), JSON.stringify(report, null, 2) + '\n');
writeFileSync(resolve(output, 'files.tsv'), tsv(files, ['path', 'tracking', 'type', 'package', 'role', 'bytes', 'lines', 'sha256', 'binary_hint']));
writeFileSync(resolve(output, 'directories.tsv'), tsv(report.directories, ['path', 'files', 'bytes']));
writeFileSync(resolve(output, 'packages.tsv'), tsv(packages, ['path', 'name', 'description']));
process.stdout.write(JSON.stringify({ ...report.summary, repeated_block_candidates: report.repeated_blocks.length, problems: problems.length, report: resolve(output, 'report.json') }) + '\n');
if (problems.length) process.exitCode = 1;
