import { html } from '../../lib/html.js';
import { ListRow, ListRowHeader, ListRowBody, ListRowTitle, ListRowText } from '../ListRow.js';
import { Badge } from '../StatusIndicators.js';

const DANGER_BADGE = { background: 'rgba(var(--danger-rgb),0.14)', borderColor: 'rgba(var(--danger-rgb),0.26)' };
const DANGER_BADGE_HIGH = { background: 'rgba(var(--danger-rgb),0.22)', borderColor: 'rgba(var(--danger-rgb),0.4)' };
const SEVERITY_THRESHOLD = 100;

export function SuppressedRow({ sigKey, count, msg, detail, expanded, index, selected, onSelect, onToggle, onUnsuppress, ...rest }) {
    const highSeverity = count >= SEVERITY_THRESHOLD;
    return html`
        <${ListRow} index=${index} selected=${selected} onSelect=${onSelect}
            accent=${highSeverity ? 'danger' : 'danger-soft'}
            onActivate=${onToggle} ...${rest}>
            <${ListRowHeader}>
                <span class="list-row-label" style="width:1rem">${expanded ? '\u25be' : '\u25b8'}</span>
                <${ListRowTitle} mono>${sigKey}<//>
                <${Badge} style=${highSeverity ? DANGER_BADGE_HIGH : DANGER_BADGE}>${'\u00d7'}${count}<//>
                ${onUnsuppress && html`<button class="btn btn-sm" tabIndex="-1"
                    onClick=${(e) => { e.stopPropagation(); onUnsuppress(sigKey); }}>Unsuppress</button>`}
            <//>
            ${expanded && (detail || (msg && html`
                <${ListRowBody}>
                    <${ListRowText} mono>${msg}<//>
                <//>
            `))}
        <//>
    `;
}
