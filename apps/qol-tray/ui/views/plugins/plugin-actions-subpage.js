import { html } from '../../lib/html.js';
import { useEffect, useState } from 'preact/hooks';
import { PageShell } from '../../components/PageShell.js';
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
        return html`<${PageShell} subtitle="Select a row from the list" frame=${false} />`;
    }

    const onActivate = (item) => {
        try { item.run?.(); }
        finally { dispatchEscape(); }
    };

    return html`
        <${PageShell} subtitle=${value.rowName || value.rowId} frameClassName="plugin-actions-frame">
            ${value.items.map(item => html`
                <${Button} key=${item.id || item.label} variant="btn-ghost"
                    onActivate=${() => onActivate(item)}>
                    ${item.label}
                <//>
            `)}
        <//>
    `;
}
