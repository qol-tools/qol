import { html } from '../../lib/html.js';
import { formatCacheAge } from './reducer.js';
import { PageHeader } from '../../components/PageHeader.js';
import { StoreGrid } from './grid.js';

export function StoreLayout({ ctrl }) {
    return html`
        <div class="view-container content-shell">
            <${PageHeader} title="Plugin Store" subtitle="Browse and install plugins for QoL Tray"
                badge=${html`<${StoreBadge} ...${ctrl} />`} />
            <div class="view-body" data-surface-container="">
                <${StoreCredentialBanner} rateLimited=${ctrl.rateLimited} hasToken=${ctrl.hasToken} />
                <${StoreGrid} plugins=${ctrl.filtered} loading=${ctrl.loading}
                    selectedIndex=${ctrl.selectedIndex} isInstalling=${ctrl.isInstalling}
                    onCardClick=${ctrl.handleCardClick} onSelect=${ctrl.setSelectedIndex} />
            </div>
        </div>
    `;
}

function StoreBadge({ cacheAgeSecs, loading, refreshPlugins }) {
    return html`
        <span class="cache-age">${formatCacheAge(cacheAgeSecs)}</span>
        <button class="refresh-btn ${loading ? 'spinning' : ''}" title="Refresh (r)"
                aria-label="Refresh" disabled=${loading} onClick=${refreshPlugins}></button>
    `;
}

function StoreCredentialBanner({ rateLimited, hasToken }) {
    if (!rateLimited || hasToken) {
        return null;
    }
    return html`
        <div class="rate-limit-banner">
            <span>
                GitHub rate limiting is hiding store results. Connect GitHub sync on the
                Profile page to reuse its credential here.
            </span>
            <button class="btn btn-ghost btn-sm" onClick=${openProfileView}>Open Profile</button>
        </div>
    `;
}

function openProfileView() {
    window.location.hash = '#profile';
}
