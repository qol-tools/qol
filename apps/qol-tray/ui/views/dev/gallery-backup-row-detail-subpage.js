import { html } from '../../lib/html.js';
import { createSharedSlot } from '../../lib/shared-slot.js';
import { BackupDetailSubPage } from '../profile/view.js';
import { toast } from '../../lib/toast.js';

export const galleryBackupRowSlot = createSharedSlot({
    preview: null, incident: null, onAcknowledge: null,
});

function dispatchEscape() {
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
}

const sandbox = (label) => () => toast('info', `${label} (gallery sandbox)`);

const sandboxConfig = {
    formatText: (s) => s,
    onClose: dispatchEscape,
    onOpenExternal: sandbox('Open in editor'),
    onCopy: sandbox('Copy'),
    onRestore: sandbox('Restore'),
    onAcknowledge: () => { sandbox('Acknowledge')(); dispatchEscape(); },
};

export function GalleryBackupRowDetailSubPage() {
    return html`<${BackupDetailSubPage} slot=${galleryBackupRowSlot} config=${sandboxConfig} />`;
}
