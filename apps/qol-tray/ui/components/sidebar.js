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

    const versionHtml = version
        ? `<div class="sidebar-version">${renderVersionFooter(version, updateState)}</div>`
        : '';

    return `<div class="sidebar-nav">${items}</div>${versionHtml}`;
}

function renderVersionFooter(version, state) {
    if (state && state.status === 'done') {
        return `<div class="version-item update-done">
                    <span class="version-main">Restarting...</span>
                    <span class="version-sub">v${version} installed</span>
                </div>`;
    }
    if (state && state.status === 'checking') {
        return `<div class="version-item">
                    <span class="version-main">v${version}</span>
                    <span class="version-sub">Checking for updates...</span>
                </div>`;
    }
    if (state && state.status === 'downloading') {
        const percent = state.percent || 0;
        const label = percent > 0 ? `Downloading ${percent}%` : 'Downloading...';
        return `<div class="version-item is-downloading">
                    <div class="progress-fill" style="width: ${percent}%"></div>
                    <span class="version-main">v${version}</span>
                    <span class="version-sub">${label}</span>
                </div>`;
    }
    if (state && state.status === 'available') {
        return `<div class="version-item has-update" data-action="self-update">
                    <span class="version-main">v${state.latest} available</span>
                    <span class="version-sub">Click to update from v${version}</span>
                </div>`;
    }
    return `<div class="version-item" data-action="check-update">
                <span class="version-main">v${version}</span>
                <span class="version-sub">Check for updates</span>
            </div>`;
}

