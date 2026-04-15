import { html } from '../../lib/html.js';
import { useEffect, useState } from 'preact/hooks';

const NEIGHBOR_HARD_CAP = 4;

export function PeripheralPreview({ navigation, registry }) {
    const [, setTick] = useState(0);
    useEffect(() => {
        if (!navigation?.subscribeAnchor) return undefined;
        return navigation.subscribeAnchor(() => setTick((t) => t + 1));
    }, [navigation]);

    const traits = navigation?.getCurrentTraits?.() || {};
    const cfg = traits['peripheral-preview'];
    if (!cfg) return null;
    const requested = Number.isInteger(cfg.neighbors) ? cfg.neighbors : 1;
    if (requested <= 0) return null;
    const neighbors = Math.min(requested, NEIGHBOR_HARD_CAP);

    const anchorId = navigation?.getCurrentAnchor?.()?.pageId;
    const confinedPages = navigation?.getConfinedPages?.() || [];
    if (!anchorId || !confinedPages.length) return null;

    const idx = confinedPages.indexOf(anchorId);
    if (idx < 0) return null;

    const slots = [];
    for (let d = 1; d <= neighbors; d++) {
        const prevId = confinedPages[idx - d];
        if (prevId) slots.push({ id: prevId, side: 'prev', distance: d });
        const nextId = confinedPages[idx + d];
        if (nextId) slots.push({ id: nextId, side: 'next', distance: d });
    }
    if (!slots.length) return null;

    return html`
        <div class="peripheral-preview" aria-hidden="true">
            ${slots.map((slot) => html`
                <div
                    class="peripheral-slot peripheral-slot-${slot.side}"
                    data-distance=${slot.distance}
                    key=${slot.id}
                >
                    <${PeripheralMini} registry=${registry} pageId=${slot.id} />
                </div>
            `)}
        </div>
    `;
}

function PeripheralMini({ registry, pageId }) {
    const entry = registry?.getEntry?.(pageId);
    const label = entry?.label || pageId;
    return html`
        <div class="peripheral-mini">
            <div class="peripheral-mini-label">${label}</div>
        </div>
    `;
}
