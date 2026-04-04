import { html } from '../../lib/html.js';

export function RegionLabels({ registry }) {
    return html`
        ${registry.getAllEntries().map(e => html`
            <div key=${e.id} class="world-region-label"
                style="left:${e.x}px; top:${e.y - 52}px;">
                ${formatLabel(e.id)}
            </div>
        `)}
    `;
}

function formatLabel(id) {
    const labels = {
        plugins: 'Plugins', store: 'Store', hotkeys: 'Hotkeys',
        shortcuts: 'Shortcuts', 'task-runner': 'Task Runner',
        profile: 'Profile', logs: 'Logs', dev: 'Developer',
    };
    return labels[id] || id;
}
