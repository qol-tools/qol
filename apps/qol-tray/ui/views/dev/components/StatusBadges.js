import { html } from '../../../lib/html.js';

function StatusBadge({ statusToken }) {
    if (statusToken === 'linked') return html`<span class="badge badge-linked">Linked</span>`;
    if (statusToken === 'installed') return html`<span class="badge badge-installed">Installed</span>`;
    if (statusToken === 'local') return html`<span class="badge badge-local">Local Clone</span>`;
    return null;
}

function BuildBadge({ plugin, statusToken }) {
    if (statusToken === 'linked' && !plugin.supports_platform) {
        return html`<span class="badge badge-build-skip">Unsupported</span>`;
    }
    if (statusToken === 'linked' && plugin.supports_platform && !plugin.has_cargo) {
        return html`<span class="badge badge-build-skip">No Cargo</span>`;
    }
    return null;
}

export function StatusBadges({ plugin, statusToken }) {
    return html`
        <div class="plugin-status-badges">
            <${StatusBadge} statusToken=${statusToken} />
            <${BuildBadge} plugin=${plugin} statusToken=${statusToken} />
            ${plugin.hasStoreInstall && html`<span class="badge badge-installed-dim">+Store</span>`}
        </div>
    `;
}
