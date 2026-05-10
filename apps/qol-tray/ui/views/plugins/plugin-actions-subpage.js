import { html } from '../../lib/html.js';
import { useEffect, useState } from 'preact/hooks';
import { PageHeader } from '../../components/PageHeader.js';
import { SurfaceContainer } from '../../lib/components/SurfaceContainer.js';
import { Button } from '../../lib/components/Button.js';
import { createSharedSlot } from '../../lib/shared-slot.js';

export const pluginActionsSlot = createSharedSlot({
    rowId: null,
    rowName: '',
    items: [],
});

function dispatchEscape() {
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
}

export function PluginActionsSubPage({ slot }) {
    const [, bump] = useState(0);
    useEffect(() => slot.subscribe(() => bump(t => t + 1)), [slot]);
    const value = slot.get();

    if (!value.rowId || !value.items?.length) {
        return html`<div class="view-container content-shell">
            <${PageHeader} title="Actions" subtitle="Select a row from the list" />
        </div>`;
    }

    const onActivate = (item) => {
        try { item.run?.(); }
        finally { dispatchEscape(); }
    };

    return html`
        <div class="view-container content-shell">
            <${PageHeader} title="Actions" subtitle=${value.rowName || value.rowId} />
            <div class="view-body content-shell-body">
                <div class="content-shell-inner">
                    <${SurfaceContainer} className="content-frame plugin-actions-frame">
                        ${value.items.map(item => html`
                            <${Button} key=${item.id || item.label} variant="btn-ghost"
                                onActivate=${() => onActivate(item)}>
                                ${item.label}
                            <//>
                        `)}
                    <//>
                </div>
            </div>
        </div>
    `;
}
