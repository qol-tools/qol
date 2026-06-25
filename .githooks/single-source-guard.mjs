#!/usr/bin/env node
// Single-source-of-truth guard for cross-process constants.
//
// The dev-server port, the platform state-socket path, and the env-var names the
// host injects into plugins live in ONE place: libs/qol-conventions. Per-plugin
// daemon ports live in ONE place: each plugin's plugin.toml [daemon] port. This
// guard blocks the raw literals from reappearing in code, so a value a
// plugin/CLI/tray/UI process uses can never drift from its single source.
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

const root = repoRoot();
if (!root) process.exit(0);

const conventionConstants = '42700|qol-tray-state\\.sock|QOL_TRAY_STATE_SOCKET|QOL_TRAY_PLUGIN_ID';
const pluginPorts = '42710|42720';

const hits = [
    grep(root, conventionConstants, ['*.rs', ':!libs/qol-conventions/']),
    grep(root, pluginPorts, ['*.js', '*.py']),
]
    .join('')
    .trim();

if (hits) {
    const out = process.stderr;
    out.write('\n  single-source guard rejected: cross-process constants must come from their single source\n');
    out.write('  offending occurrences:\n');
    for (const line of hits.split('\n')) out.write(`    ${line}\n`);
    out.write('\n  fix:\n');
    out.write('    - host constants: qol_conventions::{DEFAULT_PORT, STATE_SOCKET_PATH, ENV_STATE_SOCKET, ENV_PLUGIN_ID, settings_url}\n');
    out.write('    - plugin id     : qol_conventions::build::emit_plugin_id (from plugin.toml)\n');
    out.write('    - daemon port   : [daemon] port in plugin.toml (emit_daemon_port in Rust; host-served daemonPort in JS)\n\n');
    process.exit(1);
}

process.exit(0);
