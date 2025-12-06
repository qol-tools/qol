const LABELS = {
    plugins: 'Plugins',
    store: 'Store',
    hotkeys: 'Hotkeys',
    dev: 'Developer'
};

export function render(activeViewId, viewOrder = ['plugins', 'store', 'hotkeys']) {
    return viewOrder.map(id => `
        <div class="sidebar-item ${id === activeViewId ? 'active' : ''}" data-view="${id}">
            ${LABELS[id] || id}
        </div>
    `).join('');
}

