export function renderDevView({
    state,
    mergedList,
    getActivePluginBuildState,
    renderPluginBuildMeta,
    renderBuildResults
}) {
    const pluginRows = mergedList.map((plugin, index) => {
        const isSelected = state.selectedIndex === index;
        const statusBadge = {
            linked: '<span class="badge badge-linked">Linked</span>',
            installed: '<span class="badge badge-installed">Installed</span>',
            local: '<span class="badge badge-local">Local Clone</span>'
        }[plugin.status];

        let buildBadge = '';
        if (plugin.status === 'linked') {
            if (!plugin.has_cargo) {
                buildBadge = '<span class="badge badge-build-skip">No Cargo</span>';
            } else if (plugin.needs_rebuild) {
                buildBadge = '<span class="badge badge-build-pending">Will Rebuild</span>';
            }
        }

        const buildState = getActivePluginBuildState(plugin);
        const isRowBuilding = !!buildState;
        const isLinking = state.linkingId === plugin.id;
        const actionDisabled = isRowBuilding || !!state.linkingId;
        const statusBadges = `
            <div class="plugin-status-badges">
                ${statusBadge}
                ${buildBadge}
                ${plugin.hasStoreInstall ? '<span class="badge badge-installed-dim">+Store</span>' : ''}
            </div>
        `;

        return `
            <div class="plugin-row status-${plugin.status} ${isSelected ? 'selected' : ''} ${isRowBuilding ? 'is-building' : ''} ${isLinking ? 'is-linking' : ''}" data-index="${index}" data-plugin-id="${plugin.id}">
                <div class="plugin-main">
                    <div class="plugin-info">
                        <div class="plugin-copy">
                            <div class="plugin-title-row">
                                <span class="plugin-name">${plugin.name}</span>
                            </div>
                            <span class="plugin-path">${plugin.path || ''}</span>
                            ${renderPluginBuildMeta(plugin)}
                        </div>
                        ${statusBadges}
                    </div>
                    <div class="plugin-action-zone ${actionDisabled ? 'is-disabled' : ''}" data-action="toggle-link" data-id="${plugin.id}" aria-label="${plugin.status === 'linked' ? 'Unlink' : 'Link'} ${plugin.name}">
                    </div>
                </div>
                <div class="plugin-build-overlay-host"></div>
            </div>
        `;
    }).join('');

    return `
        <div class="view-container">
            <header>
                <h1>Developer</h1>
                <p>Link local plugins for development</p>
            </header>

            <section class="dev-section">
                <div class="section-header">
                    <h2>Plugins</h2>
                    <div class="section-actions">
                        <button class="refresh-btn ${state.discovering ? 'spinning' : ''}" data-action="refresh-discovery" title="Rescan">↻</button>
                        <button class="btn btn-sm btn-ghost" data-action="add-link">+ Link Path</button>
                    </div>
                </div>

                <div class="plugin-list-container">
                    ${mergedList.length ? `
                        <div class="plugin-list">${pluginRows}</div>
                    ` : '<p class="empty-state">No plugins found</p>'}
                </div>

                ${state.showLinkInput ? `
                    <div class="link-input-row">
                        <input type="text" id="link-path" placeholder="/path/to/plugin" value="${state.linkPath}" autofocus>
                        <button class="btn btn-sm btn-primary" data-action="confirm-link">Link</button>
                        <button class="btn btn-sm btn-ghost" data-action="cancel-link">Cancel</button>
                    </div>
                    ${state.linkError ? `<p class="error-msg">${state.linkError}</p>` : ''}
                ` : ''}
            </section>

            <section class="dev-section">
                <h2>Actions</h2>
                <div class="dev-card" data-action="reload">
                    <button class="refresh-btn ${state.building || state.reloading ? 'spinning' : ''}" tabindex="-1">↻</button>
                    <div class="dev-card-content">
                        <h3>${state.building ? 'Building...' : 'Reload All Plugins'}</h3>
                        <p>${state.building ? 'Compiling linked plugins' : 'Build linked plugins and restart daemons.'}</p>
                        ${renderBuildResults(state.buildResults)}
                        ${state.lastReload ? `<span class="last-action">Last: ${state.lastReload}</span>` : ''}
                        ${state.error ? `<span class="error-msg">${state.error}</span>` : ''}
                    </div>
                    <div class="dev-card-hint"><kbd>Ctrl+r</kbd></div>
                </div>
                <div class="dev-card" data-action="mock-update">
                    ${state.mockTesting ? '<button class="refresh-btn spinning" tabindex="-1">↻</button>' : ''}
                    <div class="dev-card-content">
                        <h3>${state.mockTesting ? 'Stop testing mock flows' : 'Test mock flows'}</h3>
                        <p>${state.mockTesting
                            ? 'Mock progress simulation is running. Click to stop.'
                            : 'Runs all registered mock progress targets without real recompiles.'}</p>
                    </div>
                </div>
            </section>

            <footer class="help">
                ↑/↓ navigate &nbsp; Enter/Space action &nbsp; r rescan &nbsp; Ctrl+r reload
            </footer>
        </div>
    `;
}
