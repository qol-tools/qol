import { html } from '../../lib/html.js';
import { ListRow, ListRowHeader, ListRowBody, ListRowTitle, ListRowText } from '../../lib/components/ListRow.js';
import { Badge } from '../../lib/components/StatusIndicators.js';

const STATUS_ACCENT = { linked: 'success', local: 'warning', installed: 'accent' };
const STATUS_BADGE = {
    linked: { label: 'Linked', bg: 'rgba(var(--success-rgb),0.14)', border: 'rgba(var(--success-rgb),0.26)' },
    local: { label: 'Local', bg: 'rgba(var(--warning-rgb),0.16)', border: 'rgba(var(--warning-rgb),0.3)' },
};

const ACTION_ICON = html`<button type="button"><img class="list-row-action-icon" src="assets/qol-tray.png?v=1" alt="" /></button>`;

export function PluginRow({ name, path, status, index, selected, onSelect, onActivate, ...rest }) {
    const accent = STATUS_ACCENT[status];
    const badge = STATUS_BADGE[status];
    return html`
        <${ListRow} index=${index} selected=${selected} onSelect=${onSelect}
            accent=${accent} onActivate=${onActivate} action=${ACTION_ICON} ...${rest}>
            <${ListRowHeader}>
                <${ListRowTitle}>${name}<//>
                ${badge && html`<${Badge} style=${{ background: badge.bg, borderColor: badge.border }}>${badge.label}<//>`}
            <//>
            ${path && html`
                <${ListRowBody}>
                    <${ListRowText} mono>${path}<//>
                <//>
            `}
        <//>
    `;
}
