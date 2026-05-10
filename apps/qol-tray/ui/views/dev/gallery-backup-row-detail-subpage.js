import { html } from '../../lib/html.js';
import { useEffect, useState } from 'preact/hooks';
import { PageHeader } from '../../components/PageHeader.js';
import { SurfaceContainer } from '../../lib/components/SurfaceContainer.js';
import { BackupDetailContent } from '../../components/domain-rows/BackupRow.js';
import { toast } from '../../lib/toast.js';
import { createSharedSlot } from '../../lib/shared-slot.js';

export const galleryBackupRowSlot = createSharedSlot({ entry: null });

function dispatchEscape() {
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
}

export function GalleryBackupRowDetailSubPage() {
    const [, bump] = useState(0);
    useEffect(() => galleryBackupRowSlot.subscribe(() => bump(t => t + 1)), []);
    const { entry } = galleryBackupRowSlot.get();
    if (!entry) {
        return html`<div class="view-container content-shell">
            <${PageHeader} title="Backup Preview" subtitle="Activate a backup row to inspect" />
        </div>`;
    }
    const sandbox = (label) => () => toast('info', `${label} (gallery sandbox)`);
    return html`
        <div class="view-container content-shell">
            <${PageHeader} title=${entry.fileName} subtitle=${entry.review ? 'Backup awaiting review' : 'Backup preview'} />
            <div class="view-body content-shell-body">
                <div class="content-shell-inner">
                    <${SurfaceContainer} className="content-frame backup-detail-frame">
                        <${BackupDetailContent}
                            text=${entry.content}
                            isIncidentBackup=${entry.review}
                            onClose=${dispatchEscape}
                            onOpenExternal=${sandbox('Open in editor')}
                            onCopy=${sandbox('Copy')}
                            onRestore=${sandbox('Restore')}
                            onAcknowledge=${() => { sandbox('Acknowledge')(); dispatchEscape(); }} />
                    <//>
                </div>
            </div>
        </div>
    `;
}
