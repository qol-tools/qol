import { html } from '../../lib/html.js';
import { useEffect, useState, useCallback } from 'preact/hooks';
import { PageHeader } from '../../components/PageHeader.js';
import { SurfaceContainer } from '../../lib/components/SurfaceContainer.js';
import { Button } from '../../lib/components/Button.js';
import { createSharedSlot } from '../../lib/shared-slot.js';

export const uninstallConfirmSlot = createSharedSlot({
    pluginId: null,
    pluginName: '',
    confirm: null,
});

function dispatchEscape() {
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
}

export function UninstallConfirmSubPage({ slot }) {
    const [, bump] = useState(0);
    useEffect(() => slot.subscribe(() => bump(t => t + 1)), [slot]);
    const value = slot.get();

    const onCancel = useCallback(() => dispatchEscape(), []);
    const onConfirm = useCallback(async () => {
        const fn = slot.get().confirm;
        try {
            if (fn) await fn();
        } finally {
            dispatchEscape();
        }
    }, [slot]);

    if (!value.pluginId) {
        return html`<div class="view-container content-shell">
            <${PageHeader} title="Confirm Uninstall" subtitle="Select a plugin from the grid" />
        </div>`;
    }

    const displayName = value.pluginName || value.pluginId;
    return html`
        <div class="view-container content-shell">
            <${PageHeader} title="Delete plugin?" subtitle=${displayName} />
            <div class="view-body content-shell-body">
                <div class="content-shell-inner">
                    <${SurfaceContainer} className="content-frame uninstall-confirm-frame">
                        <p class="uninstall-confirm-message">
                            This will uninstall <strong>${displayName}</strong> and remove all of its data.
                        </p>
                        <div class="uninstall-confirm-actions">
                            <${Button} variant="btn-ghost" onActivate=${onCancel}>
                                Cancel <kbd>Esc</kbd>
                            <//>
                            <${Button} variant="btn-danger" onActivate=${onConfirm}>
                                Delete <kbd>Enter</kbd>
                            <//>
                        </div>
                    <//>
                </div>
            </div>
        </div>
    `;
}
