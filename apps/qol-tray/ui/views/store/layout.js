import { html } from '../../lib/html.js';
import { formatCacheAge } from './reducer.js';
import { PageHeader } from '../../components/PageHeader.js';
import { SurfaceContainer } from '../../lib/components/SurfaceContainer.js';
import { Button, RefreshButton } from '../../lib/components/Button.js';
import { StoreGrid } from './grid.js';

export function StoreLayout({ ctrl }) {
    return html`
        <div class="view-container content-shell">
            <${PageHeader} subtitle="Browse and install plugins for QoL Tray"
                badge=${html`<${StoreBadge} ...${ctrl} />`} />
            <${SurfaceContainer} className="view-body">
                <${StoreCredentialBanner} rateLimited=${ctrl.rateLimited} hasToken=${ctrl.hasToken} />
                <${StoreGrid} plugins=${ctrl.filtered} loading=${ctrl.firstLoad || ctrl.refreshing}
                    selectedIndex=${ctrl.selectedIndex} isInstalling=${ctrl.isInstalling}
                    onCardClick=${ctrl.handleCardClick} onSelect=${ctrl.setSelectedIndex} />
            <//>
        </div>
    `;
}

function StoreBadge({ cacheAgeSecs, firstLoad, refreshing, refreshPlugins }) {
    return html`
        <span class="cache-age">${formatCacheAge(cacheAgeSecs)}</span>
        <${RefreshButton} spinning=${firstLoad || refreshing} disabled=${firstLoad} title="Refresh (r)"
                aria-label="Refresh" onClick=${refreshPlugins} />
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
            <${Button} variant="btn-ghost" small onActivate=${openProfileView}>Open Profile<//>
        </div>
    `;
}

function openProfileView() {
    window.location.hash = '#profile';
}
