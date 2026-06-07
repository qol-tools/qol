import { html } from '../../lib/html.js';
import { useCallback } from 'preact/hooks';
import { TableRow } from '../../lib/components/TableRow.js';
import { diveViaSelector } from '../../lib/world-navigation-singleton.js';
import { pluginActionsSlot } from '../../views/plugins/plugin-actions-subpage.js';

const STATUS_ACCENT = { linked: 'success', local: 'warning', installed: 'accent' };
const DEV_PLUGIN_ACTIONS_DIVE_SELECTOR = '[data-dive-source="dev-plugin-actions"]';

export function DevPluginRow({ name, path, status, pluginId, badges, meta, actions, actionIcon, overlay, index, selected, onSelect, onActivate, className, ...rest }) {
    const defaultActivate = useCallback(() => {
        if (!actions?.length) return;
        pluginActionsSlot.set({
            rowId: pluginId || name || 'dev-row',
            rowName: name || pluginId || '',
            items: actions.map(a => ({ label: a.label, run: a.run })),
        });
        diveViaSelector(DEV_PLUGIN_ACTIONS_DIVE_SELECTOR);
    }, [actions, pluginId, name]);
    const activate = onActivate || defaultActivate;

    const statusCls = status ? `status-${status}` : '';
    const cls = ['plugin-row', statusCls, className].filter(Boolean).join(' ');
    return html`
        <${TableRow} className=${cls} index=${index} selected=${selected} onSelect=${onSelect}
            onActivate=${activate} accent=${STATUS_ACCENT[status]}
            data-status=${status} data-plugin-id=${pluginId} ...${rest}>
            <div class="plugin-info">
                <div class="plugin-copy">
                    <div class="plugin-title-row">
                        <span class="plugin-name" data-selected-text="">${name}</span>
                    </div>
                    ${path && html`<span class="plugin-path" data-selected-text="" title=${path}>${path}</span>`}
                    ${meta}
                </div>
                ${badges}
            </div>
            <div class="plugin-action-column">
                <div class="plugin-action-zone">
                    ${actionIcon || html`<img class="list-row-action-icon" src="assets/qol-tray.png?v=1" alt="" />`}
                </div>
            </div>
            <div class="plugin-build-overlay-host">${overlay}</div>
        <//>
    `;
}
