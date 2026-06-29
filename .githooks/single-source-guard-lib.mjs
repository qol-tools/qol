// Pure helpers for single-source-guard.mjs, split out so the cross-language
// agreement logic can be unit-tested (see single-source-guard.test.mjs).

export const RUST_REPLACE_ENV_RE =
    /ENV_DAEMON_REPLACE_EXISTING:\s*&str\s*=\s*"([^"]+)"/;
export const PY_REPLACE_ENV_RE = /REPLACE_EXISTING_ENV\s*=\s*['"]([^'"]+)['"]/;

export function extractLiteral(text, re) {
    if (text == null) return null;
    const match = text.match(re);
    return match ? match[1] : null;
}

// Returns an error string when the replace-existing env name cannot be proven
// equal across its Rust single source and the Python ide-checkout daemon, or
// null when they provably agree. Fails closed: a value that is missing or no
// longer matches its anchor is an error, never a silent pass.
export function daemonEnvDrift(rustLibSource, pyServerSource) {
    const rust = extractLiteral(rustLibSource, RUST_REPLACE_ENV_RE);
    const py = extractLiteral(pyServerSource, PY_REPLACE_ENV_RE);
    if (rust === null || py === null) {
        return (
            `cannot verify the daemon replace-existing env name: ` +
            `rust=${rust ?? 'NONE'} python=${py ?? 'NONE'} ` +
            `(a source moved or changed format; update single-source-guard-lib.mjs)`
        );
    }
    if (rust !== py) {
        return (
            `cross-language drift: plugin-ide-checkout/server.py ` +
            `REPLACE_EXISTING_ENV='${py}' must equal ` +
            `qol_conventions::ENV_DAEMON_REPLACE_EXISTING='${rust}'`
        );
    }
    return null;
}
