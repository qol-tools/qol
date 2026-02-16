const LABELS = {
    plugins: 'Plugins',
    store: 'Store',
    hotkeys: 'Hotkeys',
    'task-runner': 'Task Runner',
    dev: 'Developer'
};

export function render(activeViewId, viewOrder = ['plugins', 'store', 'hotkeys'], version = null, updateState = null) {
    const items = viewOrder.map(id => `
        <div class="sidebar-item ${id === activeViewId ? 'active' : ''}" data-view="${id}">
            ${LABELS[id] || id}
        </div>
    `).join('');

    const versionHtml = version ? `
        <div class="sidebar-version">
            <span class="version-label">v${version}</span>
            ${renderUpdateControl(updateState)}
        </div>
    ` : '';

    return `<div class="sidebar-nav">${items}</div>${versionHtml}`;
}

function renderUpdateControl(state) {
    if (!state || state.status === 'idle') {
        return '';
    }
    if (state.status === 'checking' || state.status === 'downloading') {
        return '<button class="refresh-btn spinning update-btn" disabled></button>';
    }
    if (state.status === 'available') {
        return `<button class="refresh-btn update-btn update-download" data-action="self-update" title="Update (${state.latest})">⬇</button>
                <span class="update-text">Update (${state.latest})</span>`;
    }
    return `<button class="refresh-btn update-btn" data-action="check-update" title="Check for updates">↻</button>
            <span class="update-text">Check for updates</span>`;
}

