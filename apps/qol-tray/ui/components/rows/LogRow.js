import { html } from '../../lib/html.js';
import { useCallback } from 'preact/hooks';
import { ListRow } from '../ListRow.js';
import { Modal, ModalFooter } from '../ModalPreact.js';
import { CodeBlock } from '../CodeBlock.js';
import { toast } from '../../lib/toast.js';

const LEVEL_ACCENT = { startup: 'accent', error: 'danger', suppressed: 'muted' };

export function LogRow({ time, level, src, msg, loc, count, severity, index, selected, onSelect, onActivate, ...rest }) {
    const levelCls = `level-${level}`;
    const label = level.toUpperCase();
    return html`
        <${ListRow} className="log-row" index=${index} selected=${selected} onSelect=${onSelect}
            accent=${LEVEL_ACCENT[level]} onActivate=${onActivate}
            data-level=${levelCls} data-severity=${severity || undefined} ...${rest}>
            <div class="log-row-top">
                <span class="log-time">${time}</span>
                <span class="log-level-badge ${levelCls}">${label}</span>
                <span class="log-src" data-selected-text="">${src || ''}</span>
                ${loc && html`<span class="log-loc">${loc}</span>`}
                ${count > 1 && html`<span class="log-count">${'\u00d7'}${count}</span>`}
            </div>
            <div class="log-row-bottom">
                <span class="log-msg" data-selected-text="">${msg}</span>
            </div>
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
    const ts = entry.time || entry.ts;
    if (ts) lines.push(`Time:     ${ts.includes?.('T') ? ts.replace('T', ' ') : ts}`);
    if (entry.level) lines.push(`Level:    ${entry.level.toUpperCase()}`);
    if (entry.src) lines.push(`Source:   ${entry.src}`);
    if (entry.key) lines.push(`Key:      ${entry.key}`);
    if (entry.loc && entry.loc !== 'unknown:0' && entry.loc !== ':0') lines.push(`Location: ${entry.loc}`);
    if (entry.count > 1) lines.push(`Count:    ${entry.count}`);
    if (entry.v) lines.push(`Version:  ${entry.v}`);
    if (entry.commit) lines.push(`Commit:   ${entry.commit}`);
    lines.push('');
    lines.push(entry.msg || '');
    return lines.join('\n');
}
