import { html } from '../lib/html.js';

const LABELS = {
    plugins: 'Plugins',
    store: 'Store',
    hotkeys: 'Hotkeys',
    'task-runner': 'Task Runner',
    dev: 'Developer'
};

export function SidebarNav({ activeViewId, viewOrder, pluginOpen, onViewClick, onBack }) {
    const header = pluginOpen
        ? html`<div class="sidebar-header"><button class="sidebar-back" onClick=${onBack}>\u2190 Back</button></div>`
        : html`<div class="sidebar-header"><span class="sidebar-logo">QoL Tray</span></div>`;

    return html`
        <fragment>
            ${header}
            <div class="sidebar-nav">
                ${viewOrder.map(id => html`
                    <div key=${id} class="sidebar-item ${id === activeViewId ? 'active' : ''}"
                         onClick=${() => onViewClick(id)}>
                        ${LABELS[id] || id}
                    </div>
                `)}
            </div>
        </fragment>
    `;
}
