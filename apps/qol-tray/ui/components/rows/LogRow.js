import { html } from '../../lib/html.js';
import { ListRow, ListRowHeader, ListRowBody, ListRowTitle, ListRowText } from '../ListRow.js';
import { Badge } from '../StatusIndicators.js';

const LEVEL_ACCENT = { startup: 'accent', error: 'danger', suppressed: 'muted' };
const DANGER_BADGE = { background: 'rgba(var(--danger-rgb),0.14)', borderColor: 'rgba(var(--danger-rgb),0.26)' };

export function LogRow({ time, level, src, msg, loc, count, index, selected, onSelect, onActivate, ...rest }) {
    const levelCls = `level-${level}`;
    const label = level.toUpperCase();
    return html`
        <${ListRow} index=${index} selected=${selected} onSelect=${onSelect}
            accent=${LEVEL_ACCENT[level]} onActivate=${onActivate} ...${rest}>
            <${ListRowHeader}>
                <span class="list-row-label" style="width:5.5rem">${time}</span>
                <span class="log-level-badge ${levelCls}" style="width:5.8rem; flex-shrink:0">${label}</span>
                <${ListRowTitle} mono>${src}<//>
                ${loc && html`<span class="list-row-label" style="font-family:var(--font-mono); font-size:var(--fs-sm)">${loc}</span>`}
                ${count > 1 && html`<${Badge} style=${DANGER_BADGE}>${'\u00d7'}${count}<//>`}
            <//>
            <${ListRowBody}>
                <${ListRowText}>${msg}<//>
            <//>
        <//>
    `;
}
