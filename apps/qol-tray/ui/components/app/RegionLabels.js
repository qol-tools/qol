import { html } from '../../lib/html.js';
import { VIEW_LABELS } from './views.js';

export function RegionLabels({ registry }) {
    return html`
        ${registry.getEntriesForLayer(0).map(e => html`
            <div key=${e.id} class="world-region-label"
                style="left:${e.x}px; top:${e.y - 52}px;">
                ${VIEW_LABELS[e.id] || e.id}
            </div>
        `)}
    `;
}
