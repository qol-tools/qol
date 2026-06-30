#!/usr/bin/env node
// Single-source-of-truth guard for cross-process constants.
//
// The dev-server port, the platform state-socket path, and the env-var names the
// host injects into plugins live in ONE place: libs/qol-conventions. Per-plugin
// daemon ports live in ONE place: each plugin's plugin.toml [daemon] port. A
// plugin's daemon socket likewise lives in ONE place: plugin.toml [daemon].socket,
// which the host injects via QOL_TRAY_DAEMON_SOCKET - so such a plugin must not
// also hardcode a fallback socket name in Rust. This guard blocks the raw literals
// from reappearing in code, so a value a plugin/CLI/tray/UI process uses can never
// drift from its single source.
// Run by the pre-commit hook and by the CI "Single source guard" step.
import { execFileSync } from 'node:child_process';

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

const root = repoRoot();
if (!root) process.exit(0);

const conventionConstants = '42700|qol-tray-state\\.sock|QOL_TRAY_STATE_SOCKET|QOL_TRAY_PLUGIN_ID|RESERVED_PLUGIN_IDS|qol-altmon\\.log|QOL_TRAY_DAEMON_SOCKET|QOL_TRAY_DAEMON_REPLACE_EXISTING';
const pluginPorts = '42710|42720';

const constantHits = [
    grep(root, conventionConstants, ['*.rs', ':!libs/qol-conventions/']),
    grep(root, pluginPorts, ['*.js', '*.py']),
]
    .join('')
    .trim();

const socketHits = manifestSocketFallbackHits(root);

if (constantHits || socketHits) {
    const out = process.stderr;
    out.write('\n  single-source guard rejected\n');
    if (constantHits) {
        out.write('\n  cross-process constants must come from their single source\n');
        out.write('  offending occurrences:\n');
        for (const line of constantHits.split('\n')) out.write(`    ${line}\n`);
        out.write('\n  fix:\n');
        out.write('    - host constants: qol_conventions::{DEFAULT_PORT, STATE_SOCKET_PATH, ENV_STATE_SOCKET, ENV_PLUGIN_ID, ENV_DAEMON_SOCKET, ENV_DAEMON_REPLACE_EXISTING, settings_url}\n');
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
    out.write('\n');
    process.exit(1);
}

process.exit(0);
