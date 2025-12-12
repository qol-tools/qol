const LABELS = {
    plugins: 'Plugins',
    store: 'Store',
    hotkeys: 'Hotkeys',
    'task-runner': 'Task Runner',
    dev: 'Developer'
};

export function render(activeViewId, viewOrder = ['plugins', 'store', 'hotkeys'], version = null) {
    const items = viewOrder.map(id => `
        <div class="sidebar-item ${id === activeViewId ? 'active' : ''}" data-view="${id}">
            ${LABELS[id] || id}
        </div>
    `).join('');

    const versionHtml = version ? `<div class="sidebar-version">v${version}</div>` : '';

    return `<div class="sidebar-nav">${items}</div>${versionHtml}`;
}

