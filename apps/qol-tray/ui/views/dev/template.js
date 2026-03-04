export function renderDevView({
    state,
    mergedList,
    getActivePluginBuildState,
    renderPluginBuildMeta,
    renderBuildResults
}) {
    const sparklineWidth = 120;
    const sparklineHeight = 28;
    const sparklineFloor = sparklineHeight - 1;
    const sparklineCeiling = 2;
    const sparklineSpread = sparklineFloor - sparklineCeiling;
    const sparklineMinMaxCpu = 5;
    const cpuHistoryGraphLimit = 36;

    const sampleCpuPercent = sample => Number.isFinite(sample?.cpu_percent) ? sample.cpu_percent : 0;
    const cpuMonitoringEnabled = pluginId => !!state.cpuMonitoring[pluginId];

    const cpuBadgeAria = plugin => {
        const monitoringEnabled = cpuMonitoringEnabled(plugin.id);
        if (!monitoringEnabled) {
            return `Enable CPU monitoring for ${plugin.name}`;
        }
        return `Disable CPU monitoring for ${plugin.name}`;
    };

    const renderCpuSparkline = history => {
        if (!history.length) {
            return '<div class="plugin-cpu-empty">Waiting for samples</div>';
        }

        const maxCpu = history.reduce((maxValue, point) => {
            const value = sampleCpuPercent(point);
            return Math.max(maxValue, value);
        }, sparklineMinMaxCpu);

        const pointY = value => {
            const normalized = Math.max(0, Math.min(value, maxCpu)) / maxCpu;
            return sparklineFloor - normalized * sparklineSpread;
        };

        if (history.length === 1) {
            const single = sampleCpuPercent(history[0]);
            const y = pointY(single).toFixed(2);
            return `
                <svg class="plugin-cpu-sparkline" viewBox="0 0 ${sparklineWidth} ${sparklineHeight}" preserveAspectRatio="none" aria-hidden="true">
                    <line class="plugin-cpu-sparkline-base" x1="0" y1="${sparklineFloor}" x2="${sparklineWidth}" y2="${sparklineFloor}"></line>
                    <polyline class="plugin-cpu-sparkline-line" points="0,${y} ${sparklineWidth},${y}"></polyline>
                </svg>
            `;
        }

        const points = history.map((point, index) => {
            const value = sampleCpuPercent(point);
            const x = (index / (history.length - 1)) * sparklineWidth;
            const y = pointY(value);
            return `${x.toFixed(2)},${y.toFixed(2)}`;
        }).join(' ');

        return `
            <svg class="plugin-cpu-sparkline" viewBox="0 0 ${sparklineWidth} ${sparklineHeight}" preserveAspectRatio="none" aria-hidden="true">
                <line class="plugin-cpu-sparkline-base" x1="0" y1="${sparklineFloor}" x2="${sparklineWidth}" y2="${sparklineFloor}"></line>
                <polyline class="plugin-cpu-sparkline-line" points="${points}"></polyline>
            </svg>
        `;
    };

    const renderCpuStrip = plugin => {
        if (!cpuMonitoringEnabled(plugin.id)) return '';
        const sample = state.cpuByPlugin[plugin.id];
        const history = Array.isArray(sample?.history)
            ? sample.history.slice(-cpuHistoryGraphLimit)
            : [];
        const cpuPercent = sampleCpuPercent(sample);
        return `
            <div class="plugin-cpu-strip">
                <span class="plugin-cpu-strip-value">${cpuPercent.toFixed(2)}%</span>
                <div class="plugin-cpu-strip-graph">
                    ${renderCpuSparkline(history)}
                </div>
            </div>
        `;
    };

    const pluginRows = mergedList.map((plugin, index) => {
        const isSelected = state.selectedIndex === index;
        const menuOpen = state.openPluginMenuId === plugin.id;
        const statusBadge = {
            linked: '<span class="badge badge-linked">Linked</span>',
            installed: '<span class="badge badge-installed">Installed</span>',
            local: '<span class="badge badge-local">Local Clone</span>'
        }[plugin.status];

        let buildBadge = '';
        if (plugin.status === 'linked' && !plugin.supports_platform) {
            buildBadge = '<span class="badge badge-build-skip">Unsupported</span>';
        }
        if (plugin.status === 'linked' && plugin.supports_platform && !plugin.has_cargo) {
            buildBadge = '<span class="badge badge-build-skip">No Cargo</span>';
        }

        const buildState = getActivePluginBuildState(plugin);
        const isRowBuilding = !!buildState;
        const isLinking = state.linkingId === plugin.id;
        const actionDisabled = isRowBuilding || !!state.linkingId;
        const rebuildActive = plugin.status === 'linked' && plugin.has_cargo && plugin.needs_rebuild;
        const filterCount = Array.isArray(plugin.suppressed_log_patterns)
            ? plugin.suppressed_log_patterns.length
            : 0;
        const menuControls = `
            <button type="button" class="plugin-menu-trigger" data-action="toggle-plugin-menu" data-id="${plugin.id}" aria-label="Plugin options for ${plugin.name}" aria-expanded="${menuOpen ? 'true' : 'false'}">
                <svg class="plugin-menu-trigger-icon" viewBox="0 0 12 20" fill="currentColor" aria-hidden="true" focusable="false">
                    <circle cx="6" cy="3.5" r="1.8"></circle>
                    <circle cx="6" cy="10" r="1.8"></circle>
                    <circle cx="6" cy="16.5" r="1.8"></circle>
                </svg>
            </button>
            <div class="plugin-context-menu ${menuOpen ? 'open' : ''}">
                <button type="button" class="context-action" data-action="toggle-plugin-logs" data-id="${plugin.id}" aria-label="${plugin.logs_muted ? 'Unmute logs' : 'Mute logs'} for ${plugin.name}">
                    ${plugin.logs_muted ? 'Unmute Logs' : 'Mute Logs'}
                </button>
                <button type="button" class="context-action" data-action="edit-plugin-log-filters" data-id="${plugin.id}" aria-label="Edit log filters for ${plugin.name}">
                    ${filterCount > 0 ? `Edit Filters (${filterCount})` : 'Edit Filters'}
                </button>
                <button type="button" class="context-action context-cpu ${cpuMonitoringEnabled(plugin.id) ? 'stop' : 'start'}" data-action="toggle-plugin-cpu" data-id="${plugin.id}" aria-label="${cpuBadgeAria(plugin)}">
                    ${cpuMonitoringEnabled(plugin.id) ? 'Disable CPU Monitor' : 'Enable CPU Monitor'}
                </button>
            </div>
        `;
        const statusBadges = `
            <div class="plugin-status-badges">
                ${statusBadge}
                ${buildBadge}
                ${plugin.hasStoreInstall ? '<span class="badge badge-installed-dim">+Store</span>' : ''}
            </div>
        `;

        return `
            <div class="plugin-row table-list-row status-${plugin.status} ${isSelected ? 'selected' : ''} ${isRowBuilding ? 'is-building' : ''} ${isLinking ? 'is-linking' : ''}" data-status="${plugin.status}" data-selected="${isSelected ? 'true' : 'false'}" data-index="${index}" data-plugin-id="${plugin.id}">
                <div class="plugin-main table-grid">
                    <div class="plugin-info table-col">
                        <div class="plugin-copy">
                            <div class="plugin-title-row">
                                <span class="plugin-name">${plugin.name}</span>
                            </div>
                            <span class="plugin-path">${plugin.path || ''}</span>
                            ${renderPluginBuildMeta(plugin)}
                        </div>
                        ${statusBadges}
                        ${renderCpuStrip(plugin)}
                    </div>
                    <div class="plugin-action-column table-col">
                        <button type="button" class="plugin-action-zone ${actionDisabled ? 'is-disabled' : ''} ${rebuildActive ? 'has-rebuild' : 'rebuild-idle'}" data-action="toggle-link" data-id="${plugin.id}" aria-label="${plugin.status === 'linked' ? 'Unlink' : 'Link'} ${plugin.name}" ${actionDisabled ? 'disabled' : ''}>
                            <img class="plugin-action-rebuild-icon" src="assets/qol-tray.png?v=1" alt="" aria-hidden="true">
                        </button>
                        ${menuControls}
                    </div>
                </div>
                <div class="plugin-build-overlay-host"></div>
            </div>
        `;
    }).join('');

    return `
        <div class="view-container dev-view-shell">
            <div class="page-header dev-stage-head">
                <div class="page-header-main dev-stage-title">
                    <h1>Developer Control</h1>
                    <p>Link plugins, run rebuild flows, and inspect live runtime state.</p>
                </div>
                <div class="page-header-actions dev-stage-tags" aria-hidden="true">
                    <span>Runtime</span>
                    <span>Build</span>
                    <span>Discovery</span>
                </div>
            </div>

            <div class="view-body dev-view-body">
                <div class="dev-view-content">
                <div class="dev-content-frame">
                <section class="dev-section">
                    <div class="section-header">
                        <h2>Plugins</h2>
                        <div class="section-actions">
                            <button class="refresh-btn ${state.discovering ? 'spinning' : ''}" data-action="refresh-discovery" title="Rescan" aria-label="Rescan"></button>
                            <button class="btn btn-sm btn-ghost" data-action="add-link">+ Link Path</button>
                        </div>
                    </div>

                    <div class="plugin-list-container">
                        ${mergedList.length ? `
                            <div class="plugin-list table-list">${pluginRows}</div>
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
                        <button class="refresh-btn ${state.building ? 'spinning' : ''}" tabindex="-1" aria-hidden="true"></button>
                        <div class="dev-card-content">
                            <h3>${state.building ? 'Building...' : 'Reload All Plugins'}</h3>
                            <p>${state.building ? 'Compiling linked plugins' : 'Build linked plugins and restart daemons.'}</p>
                            ${renderBuildResults(state.buildResults)}
                            ${state.lastReload ? `<span class="last-action">Last: ${state.lastReload}</span>` : ''}
                            ${state.error ? `<span class="error-msg">${state.error}</span>` : ''}
                        </div>
                        <div class="dev-card-hint"><kbd>Ctrl+r</kbd></div>
                    </div>
                    <div class="dev-card ${state.mockTesting ? 'is-loading' : ''}" data-action="mock-update">
                        <button class="refresh-btn ${state.mockTesting ? 'spinning' : 'is-hidden'}" tabindex="-1" aria-hidden="true"></button>
                        <div class="dev-card-content">
                            <h3>${state.mockTesting ? 'Stop testing mock flows' : 'Test mock flows'}</h3>
                            <p>${state.mockTesting
                                ? 'Mock progress simulation is running. Click to stop.'
                                : 'Runs all registered mock progress targets without real recompiles.'}</p>
                        </div>
                    </div>
                </section>
                </div>
                </div>
            </div>
        </div>
    `;
}
