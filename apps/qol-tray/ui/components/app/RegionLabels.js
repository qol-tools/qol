import { html } from '../../lib/html.js';
import { useRef } from 'preact/hooks';
import { createDebug } from '../../lib/debug.js';
import { VIEW_LABELS } from './views.js';

const log = createDebug('qol:world');

export function RegionLabels({ registry, cameraLayer, navigation, diveDepth }) {
    const layer = cameraLayer ?? 0;
    const pages = navigation?.getConfinedPages?.() || [];
    const ascending = layer < 0 && (diveDepth ?? 0) === 0;
    const allEntries = ascending ? [] : registry.getEntriesForLayer(layer);
    const entries = pages.length > 0
        ? allEntries.filter(e => pages.includes(e.id))
        : allEntries;

    const prevRef = useRef(null);
    const key = `${layer}:${entries.length}`;
    if (prevRef.current !== key) {
        prevRef.current = key;
        log('regionLabels: layer', layer,
            `entries=${entries.length}/${allEntries.length}`,
            pages.length > 0 ? 'confined' : 'all',
            entries.map(e => e.label || VIEW_LABELS[e.id] || e.id).join(', '));
    }

    return html`
        ${entries.map(e => html`
            <div key=${e.id} class="world-region-label"
                style="left:${e.x}px; top:${e.y - 52}px;">
                ${VIEW_LABELS[e.id] || e.label || e.id}
            </div>
        `)}
    `;
}
