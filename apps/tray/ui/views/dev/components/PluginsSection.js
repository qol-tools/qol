import { html } from '../../../lib/html.js';
import { Table } from '../../../lib/components/TableRow.js';
import { Button, RefreshButton } from '../../../lib/components/Button.js';
import { PluginRow } from './PluginRow.js';
import { LinkInput } from './LinkInput.js';

function PluginsSectionHeader({ ctrl }) {
    return html`
        <div class="section-header">
            <h2>Plugins</h2>
            <div class="section-actions">
                <${RefreshButton} spinning=${ctrl.discovering} onClick=${ctrl.triggerDiscovery} title="Rescan" aria-label="Rescan" />
                <${Button} small variant="btn-ghost" onActivate=${ctrl.openLinkInput}>+ Link Path<//>
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
                    ? html`<${Table} className="plugin-list" onDeselect=${() => ctrl.setSelectedIndex(-1)}>
                        ${ctrl.mergedList.map((plugin, i) => html`<${PluginRow} key=${plugin.id} plugin=${plugin} index=${i} ctrl=${ctrl} />`)}
                    <//>`
                    : html`<p class="empty-state">No plugins found</p>`}
            </div>
            <${LinkInput} showLinkInput=${ctrl.showLinkInput} linkPath=${ctrl.linkPath} linkError=${ctrl.linkError} onInput=${ctrl.onLinkInput} onConfirm=${ctrl.confirmLink} onCancel=${ctrl.cancelLink} />
        </section>
    `;
}
