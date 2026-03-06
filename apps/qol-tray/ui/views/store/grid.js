import { html } from '../../lib/html.js';
import { isStoreUpdateAvailable } from './reducer.js';

export function StoreGrid({ plugins, loading, selectedIndex, isInstalling, onCardClick }) {
    return html`
        <div id="store-list" class="plugin-grid-store grid-cards grid-cards--zoom">
            ${loading && plugins.length === 0 && html`<div class="loading">Loading plugins...</div>`}
            ${!loading && plugins.length === 0 && html`<div class="loading">No plugins found</div>`}
            ${plugins.map((plugin, index) => html`
                <${StoreCard}
                    key=${plugin.id}
                    plugin=${plugin}
                    index=${index}
                    selected=${index === selectedIndex}
                    installing=${isInstalling(plugin.id)}
                    onCardClick=${onCardClick}
                />
            `)}
        </div>
    `;
}

function StoreCard({ plugin, index, selected, installing, onCardClick }) {
    const hasUpdate = isStoreUpdateAvailable(plugin);
    const versionDisplay = hasUpdate
        ? `v${plugin.installed_version} → v${plugin.version}`
        : `v${plugin.version}`;

    return html`
        <div
            class=${storeCardClassName(plugin, selected, installing)}
            data-index=${String(index)}
            data-plugin-id=${plugin.id}
            onClick=${(event) => onCardClick(event, index, plugin.id)}
        >
            <h3>${plugin.name}</h3>
            <div class="version${hasUpdate ? ' has-update' : ''}">${versionDisplay}</div>
            <div class="description">${plugin.description}</div>
            <div class="button-group">
                ${storeCardAction(plugin, installing, hasUpdate)}
            </div>
        </div>
    `;
}

function storeCardClassName(plugin, selected, installing) {
    const classes = ['plugin-card'];
    if (plugin.installed) classes.push('installed');
    if (installing) classes.push('installing');
    if (selected) classes.push('selected');
    return classes.join(' ');
}

function storeCardAction(plugin, installing, hasUpdate) {
    if (plugin.installed) {
        return html`<span class="installed-badge">${hasUpdate ? 'Update Available' : 'Installed'}</span>`;
    }

    if (installing) {
        return html`<button class="refresh-btn spinning" disabled></button>`;
    }

    return html`<button class="btn btn-primary install" style="width: 100%">Install</button>`;
}
