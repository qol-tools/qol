import { html } from '../../../lib/html.js';
import { Badge } from '../../../lib/components/StatusIndicators.js';

function StatusBadge({ statusToken }) {
    if (statusToken === 'linked') return html`<${Badge} className="badge-linked">Linked<//>`;
    if (statusToken === 'installed') return html`<${Badge} className="badge-installed">Installed<//>`;
    if (statusToken === 'local') return html`<${Badge} className="badge-local">Local Clone<//>`;
    return null;
}

function BuildBadge({ plugin, statusToken }) {
    if (statusToken === 'linked' && !plugin.supports_platform) {
        return html`<${Badge} className="badge-build-skip">Unsupported<//>`;
    }
    if (statusToken === 'linked' && plugin.supports_platform && !plugin.has_cargo) {
        return html`<${Badge} className="badge-build-skip">No Cargo<//>`;
    }
    return null;
}

export function StatusBadges({ plugin, statusToken }) {
    return html`
        <div class="plugin-status-badges">
            <${StatusBadge} statusToken=${statusToken} />
            <${BuildBadge} plugin=${plugin} statusToken=${statusToken} />
            ${plugin.hasStoreInstall && html`<${Badge} className="badge-installed-dim">+Store<//>`}
        </div>
    `;
}
