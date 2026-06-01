import { html } from '../../lib/html.js';
import { createSharedSlot } from '../../lib/shared-slot.js';
import { LogDetailSubPage } from '../logs-view.js';

export const galleryLogRowSlot = createSharedSlot({ entry: null });

export function GalleryLogRowDetailSubPage() {
    return html`<${LogDetailSubPage} slot=${galleryLogRowSlot} />`;
}
