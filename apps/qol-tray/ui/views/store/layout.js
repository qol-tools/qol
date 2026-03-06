import { html } from '../../lib/html.js';
import { formatCacheAge } from './reducer.js';
import { Feedback } from '../../components/FeedbackPreact.js';
import { PageHeader } from '../../components/PageHeader.js';
import { StoreTokenPanel } from './token-panel.js';
import { StoreGrid } from './grid.js';

export function StoreLayout({ ctrl }) {
    return html`
        <div class="view-container">
            <${PageHeader} title="Plugin Store" subtitle="Browse and install plugins for QoL Tray"
                actions=${html`<${StoreActions} ...${ctrl} />`} />
            <div class="view-body">
                <${StoreSearchBar} searchRef=${ctrl.searchRef} searchQuery=${ctrl.searchQuery}
                    handleSearch=${ctrl.handleSearch} />
                <${StoreTokenPanel} showTokenInput=${ctrl.showTokenInput} hasToken=${ctrl.hasToken}
                    rateLimited=${ctrl.rateLimited} tokenInputRef=${ctrl.tokenInputRef}
                    onSave=${ctrl.saveToken} onDelete=${ctrl.deleteToken}
                    onCancel=${ctrl.closeTokenInput} onShow=${ctrl.openTokenInput} />
                <${Feedback} feedback=${ctrl.feedback} />
                <${StoreGrid} plugins=${ctrl.filtered} loading=${ctrl.loading}
                    selectedIndex=${ctrl.selectedIndex} isInstalling=${ctrl.isInstalling}
                    onCardClick=${ctrl.handleCardClick} />
            </div>
        </div>
    `;
}

function StoreActions({ cacheAgeSecs, loading, openTokenInput, refreshPlugins }) {
    return html`
        <span class="cache-age">${formatCacheAge(cacheAgeSecs)}</span>
        <button class="btn btn-ghost btn-sm" title="Manage GitHub token"
                onClick=${openTokenInput}>Token</button>
        <button class="refresh-btn ${loading ? 'spinning' : ''}" title="Refresh (r)"
                aria-label="Refresh" disabled=${loading} onClick=${refreshPlugins}></button>
    `;
}

function StoreSearchBar({ searchRef, searchQuery, handleSearch }) {
    return html`
        <div class="search-bar store-search-bar">
            <input type="text" ref=${searchRef} placeholder="Search plugins..."
                   value=${searchQuery} onInput=${handleSearch} />
        </div>
    `;
}
