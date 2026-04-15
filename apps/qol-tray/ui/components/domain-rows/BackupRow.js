import { html } from '../../lib/html.js';
import { ListRow, ListRowHeader, ListRowBody, ListRowText } from '../../lib/components/ListRow.js';
import { Badge } from '../../lib/components/StatusIndicators.js';

export function BackupRow({ time, fileName, size, review, index, selected, onSelect, onActivate, ...rest }) {
    return html`
        <${ListRow} index=${index} selected=${selected} onSelect=${onSelect}
            accent="accent-soft" onActivate=${onActivate} ...${rest}>
            <${ListRowHeader}>
                <span class="list-row-label" style="width:9rem">${time}</span>
                ${review && html`<${Badge} className="profile-badge profile-badge-skipped">Review backup<//>`}
                <span class="list-row-meta">${size}</span>
            <//>
            <${ListRowBody}>
                <${ListRowText} mono>${fileName}<//>
            <//>
        <//>
    `;
}
