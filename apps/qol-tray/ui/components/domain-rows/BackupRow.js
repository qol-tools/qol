import { html } from '../../lib/html.js';
import { ListRow, ListRowHeader, ListRowBody, ListRowText } from '../../lib/components/ListRow.js';
import { Badge } from '../../lib/components/StatusIndicators.js';
import { CodeBlock } from '../../lib/components/CodeBlock.js';
import { Button } from '../../lib/components/Button.js';
import { ConfirmButton } from '../../lib/components/ConfirmButton.js';

export function BackupRow({
    time, fileName, size, review,
    index, selected, onSelect, onActivate, onSecondaryActivate, actions,
    ...rest
}) {
    return html`
        <${ListRow} index=${index} selected=${selected} onSelect=${onSelect}
            accent="accent-soft"
            onActivate=${onActivate}
            onSecondaryActivate=${onSecondaryActivate}
            actions=${actions}
            ...${rest}>
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

export function BackupDetailContent({
    text,
    isIncidentBackup,
    onClose,
    onOpenExternal,
    onCopy,
    onRestore,
    onAcknowledge,
}) {
    return html`
        <${CodeBlock}
            text=${text}
            onSecondaryActivate=${onOpenExternal}
            secondaryLabel="Open in editor" />
        <div class="backup-detail-actions">
            <${Button} variant="btn-ghost" onActivate=${onClose}>Close <kbd>Esc</kbd><//>
            <${Button} variant="btn-ghost" onActivate=${onOpenExternal}>Open in editor<//>
            <${Button} variant="btn-ghost" onActivate=${onCopy}>Copy<//>
            <${ConfirmButton} confirmWith="restore" onActivate=${onRestore}>Restore this backup<//>
            ${isIncidentBackup && html`<${Button} variant="btn-ghost" onActivate=${onAcknowledge}>Looks Good<//>`}
        </div>
    `;
}
