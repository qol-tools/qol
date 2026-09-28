import { readFile, writeFile, mkdir, access, realpath, stat } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { createServer } from 'node:http';
import { createHash } from 'node:crypto';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '../../../..');
const design = path.join(root, 'docs/specs/2026-09-27-linked-devices-design.md');
const output = path.resolve(here, '../linked-devices.html');
const reportPath = path.join(root, 'target/linked-devices-page/report.json');
const args = process.argv.slice(2);
const check = args.includes('--check');
const serve = args.includes('--serve');
const unknown = args.filter(arg => !['--check', '--serve'].includes(arg));
if (unknown.length || (check && serve)) throw new Error('Usage: node build.mjs [--check | --serve]');

const markdown = await readFile(design, 'utf8');
const metadata = JSON.parse(execFileSync('cargo', ['metadata', '--no-deps', '--format-version', '1'], { cwd: root, encoding: 'utf8', maxBuffer: 16 * 1024 * 1024 }));
const members = new Set(metadata.workspace_members);
const packages = metadata.packages.filter(item => members.has(item.id)).map(item => ({
  name: item.name,
  directory: path.relative(root, path.dirname(item.manifest_path)).split(path.sep).join('/'),
  dependencies: [...new Set(item.dependencies.filter(dep => dep.path).map(dep => dep.name))],
})).filter(item => /^(apps|plugins|libs|tools)\//.test(item.directory));

function section(title) {
  const lines = markdown.split('\n');
  const start = lines.findIndex(line => /^#{2,3} /.test(line) && line.replace(/^#+ /, '') === title);
  if (start < 0) throw new Error(`Missing design section: ${title}`);
  const depth = lines[start].match(/^#+/)[0].length;
  let end = start + 1;
  while (end < lines.length && !(new RegExp(`^#{1,${depth}} `)).test(lines[end])) end++;
  return lines.slice(start + 1, end).join('\n');
}

const plain = value => value.replace(/\[([^\]]+)\]\([^)]+\)/g, '$1').replace(/[*`]/g, '');
const slug = value => plain(value).toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');
const links = new Map();

function sourceUrl(target) {
  if (!target.startsWith('../')) throw new Error(`Unexpected source reference: ${target}`);
  const absolute = path.resolve(path.dirname(design), target.split('#')[0]);
  const relative = path.relative(root, absolute);
  if (relative.startsWith('..') || path.isAbsolute(relative)) throw new Error(`Source outside workspace: ${target}`);
  links.set(relative, absolute);
  return path.relative(path.dirname(output), absolute).split(path.sep).map(part => encodeURIComponent(part)).join('/');
}

function cells(value) {
  return value.replace(/\[([^\]]+)\]\(([^)]+)\)/g, (_, label, target) => `[${label}](${sourceUrl(target)})`);
}

function table(title, category) {
  const lines = section(title).split('\n').filter(line => line.startsWith('|'));
  if (lines.length < 3) throw new Error(`Empty design table: ${title}`);
  const split = line => line.slice(1, -1).split('|').map(cell => cell.trim());
  const headers = split(lines[0]);
  return lines.slice(2).map(line => {
    const values = split(line);
    if (values.length !== headers.length) throw new Error(`Invalid row in ${title}`);
    const references = [...values[0].matchAll(/\[([^\]]+)\]\(([^)]+)\)/g)].map(match => ({ label: match[1], target: match[2] }));
    const reference = references[0]?.target;
    const packageRoot = reference && path.resolve(path.dirname(design), reference).slice(root.length + 1);
    const pkg = packageRoot && packages.find(item => packageRoot.startsWith(`${item.directory}/`));
    return {
      id: `${category}-${slug(values[0])}`,
      category,
      name: plain(values[0]),
      proposed: values.some(value => value.includes('**Proposed:**')),
      fields: headers.slice(1).map((label, index) => ({ label, text: cells(values[index + 1]) })),
      source: reference ? sourceUrl(reference) : null,
      sources: references.map(item => ({ label: item.label, url: sourceUrl(item.target) })),
      package: pkg?.name ?? null,
      dependencies: pkg?.dependencies ?? [],
      consumers: pkg ? packages.filter(item => item.dependencies.includes(pkg.name)).map(item => item.name) : [],
    };
  });
}

function list(title, ordered) {
  const result = [];
  let active = false;
  for (const line of section(title).split('\n')) {
    const match = line.match(ordered ? /^\d+\. (.*)$/ : /^- (.*)$/);
    if (match) {
      result.push(match[1]);
      active = true;
    } else if (active && /^\s+\S/.test(line)) {
      result[result.length - 1] += ` ${line.trim()}`;
    } else if (line.trim()) {
      active = false;
    }
  }
  return result.map(cells);
}

const rows = [
  ...table('Core facts and their authoritative owners', 'core'),
  ...table("Every plugin's domain", 'plugins'),
  ...table("Every shared crate's ownership boundary", 'libraries'),
  ...table('Applications, tooling, and generated surfaces', 'surfaces'),
];
const covered = new Set(rows.map(row => row.package).filter(Boolean));
const missing = packages.filter(pkg => !covered.has(pkg.name));
if (missing.length) throw new Error(`Ownership map omits packages: ${missing.map(pkg => pkg.name).join(', ')}`);

const implementation = table('Implementation status', 'status');
const statusColumns = ['Before this work', 'Current worktree', 'Verification and remaining work'];
if (implementation.some(row => row.fields.length !== statusColumns.length || row.fields.some((field, index) => field.label !== statusColumns[index]))) {
  throw new Error('Implementation status columns changed; review the presentation');
}
const delivery = implementation.find(row => row.name === 'Deliver the complete feature');
if (!delivery) throw new Error('Missing implementation delivery status');

function find(category, prefix) {
  const row = rows.find(item => item.category === category && item.name.startsWith(prefix));
  if (!row) throw new Error(`Missing diagram owner: ${category}/${prefix}`);
  return row.id;
}

const data = {
  rows,
  implementation: { rows: implementation, delivery: delivery.id },
  counts: Object.fromEntries(['core', 'plugins', 'libraries', 'surfaces'].map(category => [category, rows.filter(row => row.category === category).length])),
  design: path.relative(path.dirname(output), design).split(path.sep).join('/'),
  documentHash: createHash('sha256').update(markdown).digest('hex'),
  nodes: {
    entry: find('core', 'Activation and invocation'),
    core: find('core', 'Device identity'),
    store: find('core', 'Device identity'),
    catalog: find('core', 'Parameterized API operations'),
    bluetooth: find('plugins', 'Bluetooth'),
    hardware: find('core', 'Real device/application state'),
    pointz: find('plugins', 'PointZ'),
    controllers: find('plugins', 'Controllers'),
  },
  handoff: list('Bluetooth handoff', true),
  gaps: list('Gaps that the implementation must close', true),
  mission: list('Mission constraints', false),
};
if (data.handoff.length !== 4 || data.gaps.length !== 5) throw new Error('Walkthrough sections changed; review the presentation');
for (const absolute of links.values()) await access(absolute);
const [template, css, js] = await Promise.all(['page.html', 'styles.css', 'app.js'].map(name => readFile(path.join(here, name), 'utf8')));
const html = template.replace('@@STYLES@@', () => css).replace('@@DATA@@', () => JSON.stringify(data).replaceAll('<', '\\u003c')).replace('@@APP@@', () => js);
if (check) {
  if (await readFile(output, 'utf8') !== html) throw new Error('Page is stale. Run node apps/tray/diagram/linked-devices/build.mjs');
} else {
  await writeFile(output, html);
}
await mkdir(path.dirname(reportPath), { recursive: true });
await writeFile(reportPath, `${JSON.stringify({ status: 'passed', output: path.relative(root, output), packages: packages.length, owners: rows.length, sourceLinks: links.size, documentHash: data.documentHash, htmlHash: createHash('sha256').update(html).digest('hex'), check }, null, 2)}\n`);
console.log(`${check ? 'Verified' : 'Built'} linked-devices.html: ${data.counts.plugins} plugins, ${data.counts.libraries} libraries, ${rows.length} ownership entries`);
console.log(`Report: ${reportPath}`);

if (serve) {
  const rootReal = await realpath(root);
  const allowed = new Set([output, design, ...links.values()]);
  const server = createServer(async (request, response) => {
    try {
      if (!['GET', 'HEAD'].includes(request.method)) {
        response.writeHead(405).end();
        return;
      }
      const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname);
      if (pathname === '/') {
        response.writeHead(302, { Location: `/${path.relative(root, output).split(path.sep).join('/')}`, 'Cache-Control': 'no-store' }).end();
        return;
      }
      const requested = path.resolve(root, `.${pathname}`);
      if (!allowed.has(requested)) {
        response.writeHead(404, { 'Content-Type': 'text/plain' }).end('Not found');
        return;
      }
      const actual = await realpath(requested);
      if (!actual.startsWith(`${rootReal}${path.sep}`)) throw new Error('Path outside workspace');
      if ((await stat(actual)).isDirectory()) {
        response.writeHead(200, { 'Content-Type': 'text/plain; charset=utf-8' }).end(`Source directory: ${path.relative(root, actual)}\nOpen this directory in your checkout to explore its implementation.`);
        return;
      }
      response.writeHead(200, { 'Content-Type': actual === output ? 'text/html; charset=utf-8' : 'text/plain; charset=utf-8', 'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff' });
      response.end(request.method === 'HEAD' ? undefined : await readFile(actual));
    } catch {
      response.writeHead(404, { 'Content-Type': 'text/plain' }).end('Not found');
    }
  });
  server.listen(0, '127.0.0.1', () => console.log(`Preview: http://127.0.0.1:${server.address().port}/`));
}
