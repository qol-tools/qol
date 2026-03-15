import { html } from '../../lib/html.js';

const brokenCovers = new Set();

const PLACEHOLDER_SVG = 'data:image/svg+xml,' + encodeURIComponent(
    '<svg xmlns="http://www.w3.org/2000/svg" width="300" height="200">' +
    '<rect fill="#2f3644" width="300" height="200"/>' +
    '<text fill="#67748f" x="50%" y="50%" text-anchor="middle" dy=".3em" font-family="sans-serif" font-size="14">No Cover</text>' +
    '</svg>'
);

export function PluginsGrid({ plugins, ghostPlugins, selectedIndex, contextMenuOpen, updating, onCardClick }) {
    return html`
        <div id="plugins-grid" class="plugin-grid-media grid-cards grid-cards--zoom">
            ${plugins.length === 0 && ghostPlugins.length === 0 && html`
                <div class="empty">No plugins installed. Press Tab to open the store.</div>
            `}
            ${ghostPlugins.map(plugin => html`
                <div key=${'ghost-' + plugin.id} class="plugin-card ghost">
                    <span class="refresh-btn spinning"></span>
                    <div class="plugin-name">${plugin.name}</div>
                </div>
            `)}
            ${plugins.map((plugin, index) => html`
                <${PluginCard} plugin=${plugin} index=${index} selectedIndex=${selectedIndex}
                    contextMenuOpen=${contextMenuOpen} updating=${updating} onCardClick=${onCardClick} />
            `)}
        </div>
    `;
}

function PluginCard({ plugin, index, selectedIndex, contextMenuOpen, updating, onCardClick }) {
    return html`
        <div key=${plugin.id}
             class=${cardClassName(plugin, index === selectedIndex)}
             data-selected-surface=""
             data-selected=${index === selectedIndex ? 'true' : 'false'}
             data-index="${index}" data-plugin-id="${plugin.id}"
             onClick=${(e) => onCardClick(e, index, plugin.id)}>
            <img src=${plugin.has_cover && !brokenCovers.has(plugin.id) ? `/api/cover/${plugin.id}` : PLACEHOLDER_SVG}
                 alt=${plugin.name}
                 onError=${(e) => { brokenCovers.add(plugin.id); e.target.src = PLACEHOLDER_SVG; }} />
            <div class="plugin-name" data-selected-text="">${plugin.name}</div>
            ${plugin.loaded === false && html`<div class="plugin-load-state" data-selected-text="">Not loaded</div>`}
            ${plugin.update_available && html`<${PluginUpdateButton} plugin=${plugin} updating=${updating} />`}
            <${PluginCogButton} />
            <div class=${contextMenuClassName(contextMenuOpen, index === selectedIndex)}>
                ${plugin.update_available && html`<button class="context-update">Update</button>`}
                <button class="context-delete">Delete</button>
            </div>
        </div>
    `;
}

function PluginUpdateButton({ plugin, updating }) {
    return html`
        <button class="plugin-update ${updating.has(plugin.id) ? 'updating' : ''}"
                aria-label="Update plugin"
                disabled=${updating.has(plugin.id)}>
            ${updating.has(plugin.id)
                ? html`<span class="refresh-btn spinning update-spinner"></span>`
                : `↑ ${plugin.available_version}`}
        </button>
    `;
}

function PluginCogButton() {
    return html`
        <button class="plugin-cog" aria-label="Plugin options">
            <svg class="plugin-cog-icon" viewBox="0 0 12 20" fill="currentColor" aria-hidden="true" focusable="false">
                <circle cx="6" cy="3.5" r="1.8"></circle>
                <circle cx="6" cy="10" r="1.8"></circle>
                <circle cx="6" cy="16.5" r="1.8"></circle>
            </svg>
        </button>
    `;
}

function cardClassName(plugin, selected) {
    const classes = ['plugin-card'];
    if (!plugin.has_ui) classes.push('no-ui');
    if (plugin.update_available) classes.push('has-update');
    if (plugin.loaded === false) classes.push('not-loaded');
    if (selected) classes.push('selected');
    return classes.join(' ');
}

function contextMenuClassName(contextMenuOpen, selected) {
    if (contextMenuOpen && selected) return 'plugin-context-menu open';
    return 'plugin-context-menu';
}
