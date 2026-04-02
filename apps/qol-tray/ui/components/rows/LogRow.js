import { html } from '../../lib/html.js';
import { useCallback } from 'preact/hooks';
import { ListRow, ListRowHeader, ListRowBody, ListRowTitle, ListRowText } from '../ListRow.js';
import { Badge } from '../StatusIndicators.js';
import { Modal, ModalFooter } from '../ModalPreact.js';
import { CodeBlock } from '../CodeBlock.js';
import { toast } from '../../lib/toast.js';

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

export function LogDetailModal({ entry, onClose }) {
    const text = formatLogDetail(entry);
    const copy = useCallback(() => {
        navigator.clipboard.writeText(text);
        toast('success', 'Copied to clipboard');
    }, [text]);

    return html`
        <${Modal} open=${true} onClose=${onClose} size="xl" dismissOnBackdrop=${true} className="edit-modal">
            <div class="edit-modal-content" tabIndex="-1">
                <h3>Log Entry</h3>
                <${CodeBlock} text=${text} />
                <${ModalFooter} actions=${[
                    { label: 'Close', kbd: 'Esc', onClick: onClose },
                    { label: 'Copy', kbd: 'C', variant: 'btn-primary', onClick: copy },
                ]} />
            </div>
        <//>
    `;
}

function formatLogDetail(entry) {
    const lines = [];
    if (entry.time) lines.push(`Time:     ${entry.time}`);
    if (entry.level) lines.push(`Level:    ${entry.level.toUpperCase()}`);
    if (entry.src) lines.push(`Source:   ${entry.src}`);
    if (entry.loc) lines.push(`Location: ${entry.loc}`);
    if (entry.count > 1) lines.push(`Count:    ${entry.count}`);
    lines.push('');
    lines.push(entry.msg || '');
    return lines.join('\n');
}
