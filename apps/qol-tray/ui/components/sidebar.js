export function render(activeViewId) {
    const views = [
        { id: 'plugins', label: 'Plugins' },
        { id: 'store', label: 'Store' },
        { id: 'hotkeys', label: 'Hotkeys' }
    ];
    
    return views.map(view => `
        <div class="sidebar-item ${view.id === activeViewId ? 'active' : ''}" data-view="${view.id}">
            ${view.label}
        </div>
    `).join('');
}

