import { html } from '../../../lib/html.js';
import { RefreshButton } from '../../../components/Button.js';
import { PluginRow } from './PluginRow.js';
import { LinkInput } from './LinkInput.js';

function PluginsSectionHeader({ ctrl }) {
    return html`
        <div class="section-header">
            <h2>Plugins</h2>
            <div class="section-actions">
                <${RefreshButton} spinning=${ctrl.discovering} onClick=${ctrl.triggerDiscovery} title="Rescan" aria-label="Rescan" />
                <button class="btn btn-sm btn-ghost" onClick=${ctrl.openLinkInput}>+ Link Path</button>
            </div>
        </div>
    `;
}

export function PluginsSection({ ctrl }) {
    return html`
        <section class="dev-section">
            <${PluginsSectionHeader} ctrl=${ctrl} />
            <div class="plugin-list-container">
                ${ctrl.mergedList.length
                    ? html`<div class="plugin-list table-list" onFocusOut=${(e) => {
                        if (!e.relatedTarget || !e.currentTarget.contains(e.relatedTarget)) ctrl.setSelectedIndex(-1);
                    }}>${ctrl.mergedList.map((plugin, i) => html`<${PluginRow} key=${plugin.id} plugin=${plugin} index=${i} ctrl=${ctrl} />`)}</div>`
                    : html`<p class="empty-state">No plugins found</p>`}
            </div>
            <${LinkInput} showLinkInput=${ctrl.showLinkInput} linkPath=${ctrl.linkPath} linkError=${ctrl.linkError} onInput=${ctrl.onLinkInput} onConfirm=${ctrl.confirmLink} onCancel=${ctrl.cancelLink} />
        </section>
    `;
}
