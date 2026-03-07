import { html } from '../../../lib/html.js';

function shortFingerprint(value) {
    if (!value) return '';
    return value.slice(0, 8);
}

function buildMetaParts(plugin) {
    const current = shortFingerprint(plugin.fingerprint);
    const last = shortFingerprint(plugin.last_built_fingerprint);
    const reason = plugin.rebuild_reason || (plugin.needs_rebuild ? 'Source changed' : 'Up to date');
    const parts = [];
    if (plugin.needs_rebuild && reason) parts.push(reason);
    if (current) parts.push(`fp ${current}`);
    if (last) parts.push(`last ${last}`);
    return parts;
}

export function BuildMeta({ plugin }) {
    if (plugin.status !== 'linked') {
        return html`<span class="plugin-build-meta plugin-build-meta-placeholder" aria-hidden="true">_</span>`;
    }
    if (!plugin.supports_platform) {
        return html`<span class="plugin-build-meta muted">${plugin.rebuild_reason || 'Unsupported platform'}</span>`;
    }
    if (!plugin.has_cargo) {
        return html`<span class="plugin-build-meta muted">Not buildable: Cargo.toml missing</span>`;
    }
    return html`<span class="plugin-build-meta">${buildMetaParts(plugin).join(' • ')}</span>`;
}
