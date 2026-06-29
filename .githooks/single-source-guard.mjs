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
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { daemonEnvDrift } from './single-source-guard-lib.mjs';

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

function readOrNull(repoRootDir, relPath) {
    try {
        return readFileSync(join(repoRootDir, relPath), 'utf8');
    } catch {
        return null;
    }
}

const root = repoRoot();
if (!root) process.exit(0);

const conventionConstants = '42700|qol-tray-state\\.sock|QOL_TRAY_STATE_SOCKET|QOL_TRAY_PLUGIN_ID|RESERVED_PLUGIN_IDS|qol-altmon\\.log|QOL_TRAY_DAEMON_SOCKET|QOL_TRAY_DAEMON_REPLACE_EXISTING';
const pluginPorts = '42710|42720';

const hits = [
    grep(root, conventionConstants, ['*.rs', ':!libs/qol-conventions/']),
    grep(root, pluginPorts, ['*.js', '*.py']),
]
    .join('')
    .trim();

// The Python ide-checkout daemon ships without the Rust source, so it hardcodes
// the replace-existing env name rather than importing the const. Assert the two
// agree; fail closed if either value can't be read (rule in the lib).
const crossLangError = daemonEnvDrift(
    readOrNull(root, 'libs/qol-conventions/src/lib.rs'),
    readOrNull(root, 'plugins/plugin-ide-checkout/server.py'),
);

if (hits || crossLangError) {
    const out = process.stderr;
    out.write('\n  single-source guard rejected: cross-process constants must come from their single source\n');
    if (hits) {
        out.write('  offending occurrences:\n');
        for (const line of hits.split('\n')) out.write(`    ${line}\n`);
    }
    if (crossLangError) {
        out.write(`  ${crossLangError}\n`);
    }
    out.write('\n  fix:\n');
    out.write('    - host constants: qol_conventions::{DEFAULT_PORT, STATE_SOCKET_PATH, ENV_STATE_SOCKET, ENV_PLUGIN_ID, ENV_DAEMON_SOCKET, ENV_DAEMON_REPLACE_EXISTING, settings_url}\n');
    out.write('    - reserved ids  : qol_conventions::is_reserved_plugin_id\n');
    out.write('    - trace log     : qol_conventions::TRACE_LOG_PATH\n');
    out.write('    - plugin id     : qol_conventions::build::emit_plugin_id (from plugin.toml)\n');
    out.write('    - daemon port   : [daemon] port in plugin.toml (emit_daemon_port in Rust; host-served daemonPort in JS)\n\n');
    process.exit(1);
}

process.exit(0);
