import { html } from '../../lib/html.js';
import { useModifierState } from '../../lib/hooks/modifier-state-context.js';
import { Card, CardGrid } from '../../lib/components/Card.js';

const brokenCovers = new Set();

const PLACEHOLDER_SVG = 'data:image/svg+xml,' + encodeURIComponent(
    '<svg xmlns="http://www.w3.org/2000/svg" width="300" height="200">' +
    '<rect fill="#2f3644" width="300" height="200"/>' +
    '<text fill="#67748f" x="50%" y="50%" text-anchor="middle" dy=".3em" font-family="sans-serif" font-size="14">No Cover</text>' +
    '</svg>'
);

export function PluginsGrid({ plugins, ghostPlugins, selectedIndex, contextMenuOpen, updating, onCardClick, onSelect }) {
    return html`
        <${CardGrid} id="plugins-grid" className="plugin-grid-media grid-cards--zoom">
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
                <${PluginCard} key=${plugin.id} plugin=${plugin} index=${index}
                    selected=${index === selectedIndex}
                    contextMenuOpen=${contextMenuOpen && index === selectedIndex}
                    updating=${updating} onCardClick=${onCardClick} onSelect=${onSelect} />
            `)}
        <//>
    `;
}

function PluginCard({ plugin, index, selected, contextMenuOpen, updating, onCardClick, onSelect }) {
    const { ctrlHeld } = useModifierState();
    const cls = cardClassName(plugin);
    const chip = pluginStatusChip(plugin);
    return html`
        <${Card} className=${cls}
             index=${index} selected=${selected} onSelect=${onSelect}
             onActivate=${(e) => onCardClick(e, index, plugin.id)}
             data-plugin-id=${plugin.id}>
            <img src=${plugin.has_cover && !brokenCovers.has(plugin.id) ? `/api/cover/${plugin.id}` : PLACEHOLDER_SVG}
                 alt=${plugin.name}
                 onError=${(e) => { brokenCovers.add(plugin.id); e.target.src = PLACEHOLDER_SVG; }} />
            <div class="plugin-name" data-selected-text="">
                <span class="plugin-name-text">${plugin.name}</span>
                ${chip && html`<span class="plugin-status-chip ${chip.className}" title=${chip.tooltip}>${chip.label}</span>`}
            </div>
            ${plugin.loaded === false && html`<div class="plugin-load-state" data-selected-text="">Not loaded</div>`}
            ${plugin.update_available && html`<${PluginUpdateButton} plugin=${plugin} updating=${updating} />`}
            <${PluginCogButton} />
            ${selected && ctrlHeld && html`
                <div class="plugin-ctrl-overlay ${plugin.has_config ? '' : 'disabled'}">Config</div>
            `}
            <div class=${contextMenuOpen ? 'plugin-context-menu open' : 'plugin-context-menu'}>
                ${plugin.update_available && html`<button class="context-update">Update</button>`}
                ${plugin.has_config && html`<button class="context-config">Config</button>`}
                <button class="context-delete">Delete</button>
            </div>
        <//>
    `;
}

function pluginStatusChip(plugin) {
    if (plugin.unavailable) {
        return {
            label: 'Broken',
            className: 'chip-unavailable',
            tooltip: plugin.load_error || 'Plugin could not be resolved from registry.'
        };
    }
    if (plugin.resolved_from === 'fallback') {
        const reason = plugin.active_failure_reason || 'unknown reason';
        return {
            label: 'Fallback',
            className: 'chip-fallback',
            tooltip: `Dev-link unavailable: ${reason}. Showing installed copy.`
        };
    }
    if (plugin.source === 'dev_linked') {
        return {
            label: 'Dev',
            className: 'chip-dev',
            tooltip: 'Running from a dev-link. Changes to the linked source take effect on reload.'
        };
    }
    return null;
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

function cardClassName(plugin) {
    const classes = ['plugin-card'];
    if (!plugin.has_custom_ui && !plugin.has_config) classes.push('no-ui');
    if (plugin.update_available) classes.push('has-update');
    if (plugin.loaded === false) classes.push('not-loaded');
    if (plugin.unavailable) classes.push('unavailable');
    if (plugin.resolved_from === 'fallback') classes.push('resolved-fallback');
    return classes.join(' ');
}
