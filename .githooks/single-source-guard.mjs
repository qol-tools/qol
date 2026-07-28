#!/usr/bin/env node
import { execFileSync } from 'node:child_process';
import { existsSync, readdirSync } from 'node:fs';
import { resolve } from 'node:path';

const TEMPLATE_WORKFLOW_PREFIX = 'plugins/template/.github/workflows/';

function repoRoot() {
    try {
        return execFileSync('git', ['rev-parse', '--show-toplevel'], { encoding: 'utf8' }).trim();
    } catch {
        return '';
    }
}

function grep(cwd, pattern, pathspec) {
    try {
        return execFileSync('git', ['grep', '-nE', pattern, '--', ...pathspec], {
            cwd,
            encoding: 'utf8',
        });
    } catch (error) {
        if (error.status === 1) return '';
        throw error;
    }
}

function manifestSocketFallbackHits(cwd) {
    const manifests = grep(cwd, '^socket[[:space:]]*=', ['plugins/']);
    const dirs = [];
    for (const line of manifests.split('\n')) {
        const match = line.match(/^(plugins\/[^/]+)\/plugin\.toml:/);
        if (match) dirs.push(match[1]);
    }
    const hits = [];
    for (const dir of dirs) {
        const found = grep(cwd, 'default_socket_name|SocketSource::Fallback', [`${dir}/src`]).trim();
        if (found) hits.push(found);
    }
    return hits.join('\n');
}

function contractDefaultDriftHits(cwd) {
    const copiedHelpers = grep(
        cwd,
        'contract_defaults_json|load_plugin_config_from_env_with_defaults|parse_spec_str\\(include_str!\\([^)]*qol-config\\.toml',
        ['plugins/'],
    ).trim();

    const defaults = grep(cwd, '^default[[:space:]]*=', ['plugins/*/qol-config.toml']);
    const dirs = [];
    for (const line of defaults.split('\n')) {
        const match = line.match(/^(plugins\/[^/]+)\/qol-config\.toml:/);
        if (match && !dirs.includes(match[1])) dirs.push(match[1]);
    }

    const oldLoaders = [];
    for (const dir of dirs) {
        const found = grep(
            cwd,
            'load_plugin_config_from_env\\(|qol_runtime::plugin_config::load\\(',
            [`${dir}/src`],
        ).trim();
        if (found) oldLoaders.push(found);
    }

    return [copiedHelpers, oldLoaders.join('\n')].filter(Boolean).join('\n');
}

function launcherIdentityHits(cwd) {
    return grep(cwd, 'qol-tray-launcher|qol-launcher', ['plugins/*/src']).trim();
}

function nestedWorkflowHits(cwd) {
    const files = execFileSync(
        'git',
        ['ls-files', '--cached', '--others', '--exclude-standard', '-z'],
        { cwd, encoding: 'utf8' },
    );
    return files
        .split('\0')
        .filter(Boolean)
        .filter((path) => path.includes('/.github/workflows/'))
        .filter((path) => !path.startsWith(TEMPLATE_WORKFLOW_PREFIX))
        .filter((path) => existsSync(resolve(cwd, path)))
        .map((path) => `${path}:1`)
        .join('\n');
}

function prefixedPluginDirectoryHits(cwd) {
    const pluginsDir = resolve(cwd, 'plugins');
    if (!existsSync(pluginsDir)) return '';
    return readdirSync(pluginsDir, { withFileTypes: true })
        .filter((entry) => entry.isDirectory() && entry.name.startsWith('plugin-'))
        .filter((entry) => existsSync(resolve(pluginsDir, entry.name, 'plugin.toml')))
        .map((entry) => `plugins/${entry.name}/plugin.toml:1`)
        .join('\n');
}

const root = repoRoot();
if (!root) process.exit(0);

const conventionConstants = [
    '42700',
    'qol-tray-state\\.sock',
    'qol-altmon\\.log',
    'QOL_TRAY_STATE_SOCKET',
    'QOL_TRAY_PLUGIN_ID',
    'QOL_TRAY_DAEMON_SOCKET',
    'QOL_TRAY_DAEMON_REPLACE_EXISTING',
    'QOL_DEV_GENERATION_MODE',
    'QOL_DEV_GENERATION_ID',
    'QOL_DEV_READY_FILE',
    'QOL_DEV_UI_PORT',
    'QOL_DEV_ROLLING_RESTART',
    '/dev/restart-prebuilt',
    '/dev/promote-generation',
    'RESERVED_PLUGIN_IDS',
].join('|');
const pluginPorts = '42710|42720';

const constantHits = [
    grep(root, conventionConstants, ['*.rs', ':!libs/qol-conventions/']),
    grep(root, pluginPorts, ['*.js', '*.py']),
]
    .join('')
    .trim();

const socketHits = manifestSocketFallbackHits(root);
const contractDefaultHits = contractDefaultDriftHits(root);
const launcherHits = launcherIdentityHits(root);
const nestedWorkflows = nestedWorkflowHits(root);
const prefixedPluginDirectories = prefixedPluginDirectoryHits(root);

if (
    constantHits ||
    socketHits ||
    contractDefaultHits ||
    launcherHits ||
    nestedWorkflows ||
    prefixedPluginDirectories
) {
    const out = process.stderr;
    out.write('\n  single-source guard rejected\n');
    if (constantHits) {
        out.write('\n  cross-process constants must come from their single source\n');
        out.write('  offending occurrences:\n');
        for (const line of constantHits.split('\n')) out.write(`    ${line}\n`);
        out.write('\n  fix:\n');
        out.write('    - host constants: qol_conventions::{DEFAULT_PORT, STATE_SOCKET_PATH, ENV_STATE_SOCKET, ENV_PLUGIN_ID, ENV_DAEMON_SOCKET, ENV_DAEMON_REPLACE_EXISTING, ENV_DEV_*, DEV_*_ROUTE, settings_url}\n');
        out.write('    - reserved ids  : qol_conventions::is_reserved_plugin_id\n');
        out.write('    - trace log     : qol_conventions::TRACE_LOG_PATH\n');
        out.write('    - plugin id     : qol_conventions::build::emit_plugin_id (from plugin.toml)\n');
        out.write('    - daemon port   : [daemon] port in plugin.toml (emit_daemon_port in Rust; host-served daemonPort in JS)\n');
    }
    if (socketHits) {
        out.write('\n  a plugin that declares [daemon].socket in plugin.toml must not also hardcode\n');
        out.write('  a fallback socket name in Rust: the manifest is the single source and the host\n');
        out.write('  injects it via QOL_TRAY_DAEMON_SOCKET into the daemon and every action client.\n');
        out.write('  offending occurrences:\n');
        for (const line of socketHits.split('\n')) out.write(`    ${line}\n`);
        out.write('\n  fix: use SocketSource::EnvRequired (see plugin-launcher/src/daemon.rs); drop default_socket_name.\n');
    }
    if (contractDefaultHits) {
        out.write('\n  plugin config defaults must come from qol-config.toml through qol-config\n');
        out.write('  offending occurrences:\n');
        for (const line of contractDefaultHits.split('\n')) out.write(`    ${line}\n`);
        out.write('\n  fix:\n');
        out.write('    - embed the contract with qol_config::plugin_config_contract!()\n');
        out.write('    - load with qol_config::load_plugin_config_from_env_with_contract(...)\n');
        out.write('    - validate tests with qol_config::validate_contract_defaults_match_type::<T>(...)\n');
        out.write('    - for runtime-socket config, merge raw JSON with qol_config::deserialize_with_contract_defaults(...)\n');
    }
    if (launcherHits) {
        out.write('\n  launcher window identity markers must come from qol_conventions::launcher\n');
        out.write('  offending occurrences:\n');
        for (const line of launcherHits.split('\n')) out.write(`    ${line}\n`);
    }
    if (nestedWorkflows) {
        out.write('\n  nested GitHub workflows are inert inside the monorepo and duplicate automation\n');
        out.write('  offending occurrences:\n');
        for (const line of nestedWorkflows.split('\n')) out.write(`    ${line}\n`);
        out.write('\n  fix: keep executable workflows at root; only the plugin template may embed scaffold workflows.\n');
    }
    if (prefixedPluginDirectories) {
        out.write('\n  plugin source directories must not repeat the plugin- namespace\n');
        out.write('  offending occurrences:\n');
        for (const line of prefixedPluginDirectories.split('\n')) out.write(`    ${line}\n`);
        out.write('\n  fix: name source directories by capability; plugin.toml owns stable identity.\n');
    }
    out.write('\n');
    process.exit(1);
}

process.exit(0);
