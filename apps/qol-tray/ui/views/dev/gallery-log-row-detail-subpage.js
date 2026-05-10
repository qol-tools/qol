import { html } from '../../lib/html.js';
import { useEffect, useState } from 'preact/hooks';
import { PageHeader } from '../../components/PageHeader.js';
import { SurfaceContainer } from '../../lib/components/SurfaceContainer.js';
import { LogDetailContent } from '../../components/domain-rows/LogRow.js';
import { createSharedSlot } from '../../lib/shared-slot.js';

export const galleryLogRowSlot = createSharedSlot({ entry: null });

export function GalleryLogRowDetailSubPage() {
    const [, bump] = useState(0);
    useEffect(() => galleryLogRowSlot.subscribe(() => bump(t => t + 1)), []);
    const { entry } = galleryLogRowSlot.get();
    if (!entry) {
        return html`
            <div class="view-container content-shell">
                <${PageHeader} title="Log Detail" subtitle="Activate a log row to inspect" />
            </div>`;
    }
    return html`
        <div class="view-container content-shell">
            <${PageHeader} title="Log Detail" subtitle=${`${entry.level} ${entry.src}`} />
            <${SurfaceContainer} className="view-body">
                <${LogDetailContent} entry=${entry} />
            <//>
        </div>
    `;
}
