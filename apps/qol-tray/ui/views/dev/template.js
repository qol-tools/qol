import { escapeAttr, escapeHtml } from '../../utils/escape-html.js';
import { renderPluginRows } from './plugin-row-template.js';

export function renderDevView({
    state,
    mergedList,
    getActivePluginBuildState,
    renderPluginBuildMeta,
    renderBuildResults
}) {
    const pluginRows = renderPluginRows({
        state,
        mergedList,
        getActivePluginBuildState,
        renderPluginBuildMeta
    });

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
                            <input type="text" id="link-path" placeholder="/path/to/plugin" value="${escapeAttr(state.linkPath)}">
                            <button class="btn btn-sm btn-primary" data-action="confirm-link">Link</button>
                            <button class="btn btn-sm btn-ghost" data-action="cancel-link">Cancel</button>
                        </div>
                        ${state.linkError ? `<p class="error-msg">${escapeHtml(state.linkError)}</p>` : ''}
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
                            ${state.lastReload ? `<span class="last-action">Last: ${escapeHtml(state.lastReload)}</span>` : ''}
                            ${state.error ? `<span class="error-msg">${escapeHtml(state.error)}</span>` : ''}
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
