import { html } from '../../lib/html.js';
import { isStoreUpdateAvailable } from './reducer.js';
import { StoreCard, StoreCardGrid } from '../../components/domain-rows/StoreCard.js';

export function StoreGrid({ plugins, loading, selectedIndex, isInstalling, onCardClick, onSelect }) {
    return html`
        <${StoreCardGrid} id="store-list">
            ${loading && plugins.length === 0 && html`<div class="loading">Loading plugins...</div>`}
            ${!loading && plugins.length === 0 && html`<div class="loading">No plugins found</div>`}
            ${plugins.map((plugin, index) => {
                const hasUpdate = isStoreUpdateAvailable(plugin);
                return html`
                    <${StoreCard}
                        key=${plugin.id}
                        name=${plugin.name}
                        version=${hasUpdate ? { from: plugin.installed_version, to: plugin.version } : plugin.version}
                        description=${plugin.description}
                        installed=${plugin.installed}
                        installing=${isInstalling(plugin.id)}
                        hasUpdate=${hasUpdate}
                        data-plugin-id=${plugin.id}
                        index=${index}
                        selected=${index === selectedIndex}
                        onSelect=${onSelect}
                        onActivate=${(e) => onCardClick(e, index, plugin.id)}
                    />
                `;
            })}
        <//>
    `;
}
