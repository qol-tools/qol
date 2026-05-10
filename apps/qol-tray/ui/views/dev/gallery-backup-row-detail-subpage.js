import { html } from '../../lib/html.js';
import { useEffect, useState } from 'preact/hooks';
import { PageHeader } from '../../components/PageHeader.js';
import { SurfaceContainer } from '../../lib/components/SurfaceContainer.js';
import { CodeBlock } from '../../lib/components/CodeBlock.js';
import { createSharedSlot } from '../../lib/shared-slot.js';

export const galleryBackupRowSlot = createSharedSlot({ entry: null });

export function GalleryBackupRowDetailSubPage() {
    const [, bump] = useState(0);
    useEffect(() => galleryBackupRowSlot.subscribe(() => bump(t => t + 1)), []);
    const { entry } = galleryBackupRowSlot.get();
    if (!entry) {
        return html`
            <div class="view-container content-shell">
                <${PageHeader} title="Backup Preview" subtitle="Activate a backup row to inspect" />
            </div>`;
    }
    return html`
        <div class="view-container content-shell">
            <${PageHeader} title=${entry.fileName} subtitle=${`${entry.time} ${entry.size}`} />
            <${SurfaceContainer} className="view-body">
                <${CodeBlock} text=${entry.content} />
            <//>
        </div>
    `;
}
