import { html } from '../../lib/html.js';
import { useState } from 'preact/hooks';
import { Card, CardGrid } from '../../lib/components/Card.js';
import { useModifierState } from '../../lib/hooks/modifier-state-context.js';

const brokenCovers = new Set();

function pluginMonogram(name) {
    const words = (name || '').trim().split(/[\s\-_]+/).filter(Boolean);
    if (words.length >= 2) return (words[0][0] + words[1][0]).toUpperCase();
    if (words.length === 1) return words[0].slice(0, 2).toUpperCase();
    return '?';
}

export function PluginsGrid({ plugins, ghostPlugins, selectedIndex, updating, loaded, onCardClick, onSelect, onToggleMenu }) {
    return html`
        <${CardGrid} id="plugins-grid" className="plugin-grid-media grid-cards--zoom">
            ${plugins.length === 0 && ghostPlugins.length === 0 && (loaded
                ? html`<div class="empty">No plugins installed. Press Tab to open the store.</div>`
                : Array.from({ length: 8 }, (_, i) => html`<div key=${'sk-' + i} class="plugin-card skeleton" aria-hidden="true"></div>`))}
            ${ghostPlugins.map(plugin => html`
                <div key=${'ghost-' + plugin.id} class="plugin-card ghost">
                    <span class="refresh-btn spinning"></span>
                    <div class="plugin-name">${plugin.name}</div>
                </div>
            `)}
            ${plugins.map((plugin, index) => html`
                <${PluginCard} key=${plugin.id} plugin=${plugin} index=${index}
                    selected=${index === selectedIndex}
                    updating=${updating} onCardClick=${onCardClick} onSelect=${onSelect}
                    onToggleMenu=${onToggleMenu} />
            `)}
        <//>
    `;
}

function PluginCard({ plugin, index, selected, updating, onCardClick, onSelect, onToggleMenu }) {
    const cls = cardClassName(plugin);
    const chip = pluginStatusChip(plugin);
    const { shiftHeld } = useModifierState();
    const [, markBroken] = useState(0);
    const showCover = plugin.has_cover && !brokenCovers.has(plugin.id);
    const showShiftHint = selected && shiftHeld;
    const handleCogClick = (e) => {
        e.stopPropagation();
        onToggleMenu(index);
    };
    return html`
        <${Card} className=${cls}
             index=${index} selected=${selected} onSelect=${onSelect}
             onActivate=${(e) => onCardClick(e, index, plugin.id)}
             data-plugin-id=${plugin.id}>
            ${showCover
                ? html`<img src=${`/api/cover/${plugin.id}`} alt=${plugin.name}
                         onError=${() => { brokenCovers.add(plugin.id); markBroken(n => n + 1); }} />`
                : html`<div class="plugin-cover-placeholder" aria-hidden="true">
                         <span class="plugin-cover-monogram">${pluginMonogram(plugin.name)}</span>
                       </div>`}
            <div class="plugin-name" data-selected-text="">
                <span class="plugin-name-text">${plugin.name}</span>
                ${chip && html`<span class="plugin-status-chip ${chip.className}" title=${chip.tooltip}>${chip.label}</span>`}
            </div>
            ${plugin.loaded === false && html`<div class="plugin-load-state" data-selected-text="">Not loaded</div>`}
            ${plugin.update_available && html`<${PluginUpdateButton} plugin=${plugin} updating=${updating} />`}
            <${PluginCogButton} onClick=${handleCogClick} />
            ${showShiftHint && html`<div class="plugin-shift-overlay">Menu</div>`}
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

function PluginCogButton({ onClick }) {
    return html`
        <button class="plugin-cog" aria-label="Plugin options" onClick=${onClick}>
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
    if (!plugin.has_config) classes.push('no-ui');
    if (plugin.update_available) classes.push('has-update');
    if (plugin.loaded === false) classes.push('not-loaded');
    if (plugin.unavailable) classes.push('unavailable');
    if (plugin.resolved_from === 'fallback') classes.push('resolved-fallback');
    return classes.join(' ');
}
