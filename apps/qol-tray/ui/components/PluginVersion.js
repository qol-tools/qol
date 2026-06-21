import { html } from '../lib/html.js';
import { resolvePluginVersion, formatPluginVersionLabel } from '../utils/plugin-version.js';

export function PluginVersion({ plugin, version, hasUpdate = false, className }) {
    const resolved = version !== undefined ? version : resolvePluginVersion(plugin);
    const label = formatPluginVersionLabel(resolved, hasUpdate);
    if (!label) return null;
    const cls = ['plugin-version', hasUpdate && 'has-update', className].filter(Boolean).join(' ');
    return html`<span class=${cls} data-selected-text="">${label}</span>`;
}
