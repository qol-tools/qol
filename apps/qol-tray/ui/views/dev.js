import { subscribe } from '../events.js';

export const id = 'dev';

const state = {
    reloading: false,
    building: false,
    buildResults: null,
    lastReload: null,
    error: null,
    plugins: [],
    discovered: [],
    discovering: false,
    selectedIndex: 0,
    showLinkInput: false,
    linkPath: '',
    linkError: null,
    mergedList: [],
    mergedCount: 0,
    linkingId: null,
    buildProgress: {}
};

let container = null;
let unsubscribe = null;

export function render(containerEl) {
    container = containerEl;
    container.addEventListener('click', handleClick);
    loadPlugins();
    fetchDiscoveryState();
    unsubscribe = subscribe(handleEvent);
}

function handleEvent(event) {
    if (state.linkingId && (event.type === 'discovery_started' || event.type === 'discovery_complete' || event.type === 'plugins_changed')) {
        return;
    }
    if (event.type === 'discovery_started') {
        state.discovering = true;
        updateView();
    } else if (event.type === 'discovery_complete') {
        state.discovering = false;
        state.discovered = event.plugins || [];
        updateView();
    } else if (event.type === 'plugins_changed') {
        loadLinkedPlugins();
    } else if (event.type === 'build_started') {
        state.building = true;
        state.buildResults = null;
        state.buildProgress = {};
        updateView();
    } else if (event.type === 'build_plugin_progress') {
        state.buildProgress[event.plugin_id] = {
            status: event.status || 'building',
            percent: Number.isFinite(event.percent) ? event.percent : 0,
            phase: event.phase || ''
        };
        updateView();
    } else if (event.type === 'build_complete') {
        state.building = false;
        state.buildResults = event.results || [];
        updateView();
        loadLinkedPlugins();
    }
}

async function fetchDiscoveryState() {
    await refreshDiscoveryState();
    if (!state.linkingId) updateView();
}

async function loadLinkedPlugins() {
    if (state.linkingId) return;
    try {
        const res = await fetch('/api/dev/links');
        if (res.ok) state.plugins = await res.json();
        updateView();
    } catch (e) {}
}

async function loadPlugins(skipUpdate = false) {
    try {
        const res = await fetch('/api/dev/links');
        if (res.ok) state.plugins = await res.json();
    } catch (e) {
        console.error('Failed to load plugins:', e);
    }
    if (!skipUpdate && !state.linkingId) updateView();
}

function totalItems() {
    return state.mergedCount || 0;
}

function renderBuildResults() {
    if (!state.buildResults) return '';

    const failed = state.buildResults.filter(r => !r.success);
    const skipped = state.buildResults.filter(r => r.skipped);
    if (state.buildResults.length === 0 || skipped.length === state.buildResults.length) {
        return `<span class="build-success">All linked plugins are up to date</span>`;
    }
    const allSuccess = failed.length === 0;
    if (allSuccess) {
        const skippedText = skipped.length ? ` (${skipped.length} skipped)` : '';
        return `<span class="build-success">Build succeeded${skippedText}</span>`;
    }

    return `<span class="build-error">Build failed: ${failed.map(r => r.plugin_id).join(', ')}</span>`;
}

function shortFingerprint(value) {
    if (!value) return '';
    return value.slice(0, 8);
}

function renderPluginBuildMeta(plugin) {
    if (plugin.status !== 'linked') return '';

    if (!plugin.has_cargo) {
        return `<span class="plugin-build-meta muted">Not buildable: Cargo.toml missing</span>`;
    }

    const current = shortFingerprint(plugin.fingerprint);
    const last = shortFingerprint(plugin.last_built_fingerprint);
    const reason = plugin.rebuild_reason || (plugin.needs_rebuild ? 'Source changed' : 'Up to date');
    const parts = [reason];
    if (current) parts.push(`fp ${current}`);
    if (last) parts.push(`last ${last}`);
    return `<span class="plugin-build-meta">${parts.join(' • ')}</span>`;
}

function renderPluginBuildProgress(plugin) {
    const progress = state.buildProgress[plugin.id];
    if (!progress || plugin.status !== 'linked') return '';

    const percent = Math.max(0, Math.min(100, progress.percent || 0));
    const status = progress.status || 'building';
    const phase = progress.phase || '';

    return `
        <div class="plugin-progress-row status-${status}">
            <span class="plugin-progress-status">${status}</span>
            <span class="plugin-progress-phase">${phase}</span>
            <span class="plugin-progress-percent">${percent}%</span>
        </div>
        <div class="plugin-progress-track status-${status}">
            <div class="plugin-progress-fill" style="width:${percent}%"></div>
        </div>
    `;
}

function updateView() {
    const unified = new Map();

    for (const d of state.discovered) {
        unified.set(d.id, {
            id: d.id,
            name: d.name,
            path: d.path,
            status: 'local',
            has_cargo: false,
            needs_rebuild: false,
            rebuild_reason: '',
            fingerprint: null,
            last_built_fingerprint: null
        });
    }

    for (const p of state.plugins) {
        const existing = unified.get(p.id);
        if (existing) {
            existing.status = 'linked';
            existing.path = p.source || existing.path;
            existing.has_cargo = !!p.has_cargo;
            existing.needs_rebuild = !!p.needs_rebuild;
            existing.rebuild_reason = p.rebuild_reason || '';
            existing.fingerprint = p.fingerprint || null;
            existing.last_built_fingerprint = p.last_built_fingerprint || null;
        } else {
            unified.set(p.id, {
                id: p.id,
                name: p.name,
                path: p.source,
                status: 'linked',
                has_cargo: !!p.has_cargo,
                needs_rebuild: !!p.needs_rebuild,
                rebuild_reason: p.rebuild_reason || '',
                fingerprint: p.fingerprint || null,
                last_built_fingerprint: p.last_built_fingerprint || null
            });
        }
    }

    const mergedList = Array.from(unified.values()).sort((a, b) => a.name.localeCompare(b.name));
    state.mergedCount = mergedList.length;
    state.mergedList = mergedList;
    state.selectedIndex = Math.max(0, Math.min(state.selectedIndex, mergedList.length - 1));

    const visibleIds = new Set(mergedList.map(plugin => plugin.id));
    for (const pluginId of Object.keys(state.buildProgress)) {
        if (!visibleIds.has(pluginId)) {
            delete state.buildProgress[pluginId];
        }
    }

    const pluginRows = mergedList.map((p, i) => {
        const isSelected = state.selectedIndex === i;
        const statusBadge = {
            linked: '<span class="badge badge-linked">Linked</span>',
            installed: '<span class="badge badge-installed">Installed</span>',
            local: '<span class="badge badge-local">Local Clone</span>'
        }[p.status];
        let buildBadge = '';
        if (p.status === 'linked') {
            if (!p.has_cargo) {
                buildBadge = '<span class="badge badge-build-skip">No Cargo</span>';
            } else if (p.needs_rebuild) {
                buildBadge = '<span class="badge badge-build-pending">Will Rebuild</span>';
            } else {
                buildBadge = '<span class="badge badge-build-ready">Up To Date</span>';
            }
        }

        const isLinking = state.linkingId === p.id;
        let actionBtn = '';
        if (isLinking) {
            actionBtn = `<button class="refresh-btn spinning" disabled>↻</button>`;
        } else if (p.status === 'linked') {
            actionBtn = `<button class="btn btn-sm btn-outline-danger" data-action="unlink" data-id="${p.id}">Unlink</button>`;
        } else if (p.path) {
            actionBtn = `<button class="btn btn-sm btn-success" data-action="link" data-id="${p.id}" data-path="${p.path}">Link</button>`;
        } else {
            actionBtn = `<button class="btn btn-sm btn-ghost" data-action="link-manual" data-id="${p.id}">Link...</button>`;
        }

        return `
            <div class="plugin-row status-${p.status} ${isSelected ? 'selected' : ''}" data-index="${i}">
                <div class="plugin-main">
                    <div class="plugin-info">
                        <span class="plugin-name">${p.name}</span>
                        <span class="plugin-path">${p.path || ''}</span>
                        ${renderPluginBuildMeta(p)}
                        ${renderPluginBuildProgress(p)}
                    </div>
                    <div class="plugin-status-badges">
                        ${statusBadge}
                        ${buildBadge}
                        ${p.hasStoreInstall ? '<span class="badge badge-installed-dim">+Store</span>' : ''}
                    </div>
                </div>
                <div class="plugin-actions">
                    ${actionBtn}
                </div>
            </div>
        `;
    }).join('');

    container.innerHTML = `
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
                        ${renderBuildResults()}
                        ${state.lastReload ? `<span class="last-action">Last: ${state.lastReload}</span>` : ''}
                        ${state.error ? `<span class="error-msg">${state.error}</span>` : ''}
                    </div>
                    <div class="dev-card-hint"><kbd>Ctrl+r</kbd></div>
                </div>
                <div class="dev-card" data-action="mock-update">
                    <div class="dev-card-content">
                        <h3>Test update flow</h3>
                        <p>Streams download progress then triggers the restarting state.</p>
                    </div>
                </div>
            </section>

            <footer class="help">
                ↑/↓ navigate &nbsp; Enter/Space action &nbsp; r rescan &nbsp; Ctrl+r reload
            </footer>
        </div>
    `;

    const input = container.querySelector('#link-path');
    if (input) {
        input.addEventListener('input', e => { state.linkPath = e.target.value; });
        input.addEventListener('keydown', e => {
            if (e.key === 'Enter') confirmLink();
            if (e.key === 'Escape') cancelLink();
        });
    }
}

function handleClick(e) {
    const action = e.target.closest('[data-action]')?.dataset.action;
    const id = e.target.closest('[data-id]')?.dataset.id;
    const path = e.target.closest('[data-path]')?.dataset.path;

    if (action === 'mock-update') fetch('/api/dev/mock-self-update', { method: 'POST' });
    if (action === 'reload') reloadPlugins();
    if (action === 'refresh-discovery') triggerDiscovery();
    if (action === 'add-link') showLinkInput();
    if (action === 'confirm-link') confirmLink();
    if (action === 'cancel-link') cancelLink();
    if (action === 'unlink' && id) deleteLink(id);
    if (action === 'link' && id && path) quickLink(path, id);
    if (action === 'link-manual' && id) {
        state.linkPath = '';
        showLinkInput();
    }

    const row = e.target.closest('.plugin-row');
    if (row && !e.target.closest('button')) {
        state.selectedIndex = parseInt(row.dataset.index);
        updateView();
    }
}

function handleItemActivation() {
    const item = state.mergedList[state.selectedIndex];
    if (!item) return;

    if (item.status === 'linked') {
        deleteLink(item.id);
    } else if (item.path) {
        quickLink(item.path, item.id);
    } else {
        showLinkInput();
    }
}

async function quickLink(path, id) {
    if (state.linkingId) return;
    state.linkingId = id;
    updateView();

    try {
        const res = await fetch('/api/dev/links', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ path, id })
        });
        if (!res.ok) {
            console.error('Failed to link:', await res.text());
            return;
        }
        await triggerReload();
        await loadPlugins(true);
    } catch (e) {
        console.error('Failed to link:', e);
    } finally {
        state.linkingId = null;
        updateView();
    }
}

function showLinkInput() {
    state.showLinkInput = true;
    state.linkError = null;
    updateView();
}

function cancelLink() {
    state.showLinkInput = false;
    state.linkPath = '';
    state.linkError = null;
    updateView();
}

async function confirmLink() {
    if (!state.linkPath.trim()) {
        state.linkError = 'Enter a path';
        updateView();
        return;
    }

    try {
        const res = await fetch('/api/dev/links', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ path: state.linkPath })
        });

        if (!res.ok) {
            state.linkError = await res.text();
            updateView();
            return;
        }

        state.showLinkInput = false;
        state.linkPath = '';
        state.linkError = null;
        await triggerReload();
        await loadPlugins();
    } catch (e) {
        state.linkError = e.message;
        updateView();
    }
}

async function deleteLink(id) {
    if (state.linkingId) return;
    state.linkingId = id;
    updateView();

    try {
        const res = await fetch(`/api/dev/links/${id}`, { method: 'DELETE' });
        if (!res.ok) {
            console.error('Failed to delete link:', await res.text());
            return;
        }
        await triggerReload();
        await Promise.all([
            loadPlugins(true),
            refreshDiscoveryState()
        ]);
    } catch (e) {
        console.error('Failed to delete link:', e);
    } finally {
        state.linkingId = null;
        updateView();
    }
}

async function refreshDiscoveryState() {
    try {
        const res = await fetch('/api/dev/discovery-state');
        if (!res.ok) return;
        const data = await res.json();
        state.discovering = data.status === 'discovering';
        if (data.status === 'complete') {
            state.discovered = data.plugins;
        }
    } catch (e) {}
}

async function triggerReload() {
    const res = await fetch('/api/dev/reload', { method: 'POST' });
    if (!res.ok && res.status !== 409) {
        const message = await res.text();
        throw new Error(message || 'Failed to queue reload');
    }
    return res;
}

async function triggerDiscovery() {
    if (state.discovering) return;
    await fetch('/api/dev/discover', { method: 'POST' });
}

async function reloadPlugins() {
    if (state.reloading || state.building) return;

    state.reloading = true;
    state.error = null;
    state.buildResults = null;
    updateView();

    try {
        const [reloadRes, discoverRes] = await Promise.all([
            fetch('/api/dev/reload', { method: 'POST' }),
            fetch('/api/dev/discover', { method: 'POST' })
        ]);

        if (reloadRes.ok && discoverRes.ok) {
            state.lastReload = new Date().toLocaleTimeString();
            await loadPlugins();
        } else if (reloadRes.status === 409) {
            state.error = 'Build already in progress';
        } else {
            const [reloadText, discoverText] = await Promise.all([
                reloadRes.text().catch(() => ''),
                discoverRes.text().catch(() => '')
            ]);
            state.error = reloadText || discoverText || 'Reload or discovery trigger failed';
        }
    } catch (err) {
        state.error = err.message;
    } finally {
        state.reloading = false;
        updateView();
    }
}

export function handleKey(e) {
    if (state.showLinkInput) return;

    if ((e.ctrlKey || e.metaKey) && (e.key === 'r' || e.key === 'R')) {
        e.preventDefault();
        reloadPlugins();
        return;
    }

    if (e.ctrlKey || e.altKey || e.metaKey) return;

    const total = totalItems();

    if (e.key === 'ArrowDown' && total > 0) {
        e.preventDefault();
        state.selectedIndex = Math.min(state.selectedIndex + 1, total - 1);
        updateView();
    }

    if (e.key === 'ArrowUp' && total > 0) {
        e.preventDefault();
        state.selectedIndex = Math.max(state.selectedIndex - 1, 0);
        updateView();
    }

    if (e.key === ' ' || e.key === 'Enter') {
        e.preventDefault();
        handleItemActivation();
    }

    if (e.key === 'r' || e.key === 'R') {
        e.preventDefault();
        triggerDiscovery();
    }
}

export function onFocus() {
    if (!state.linkingId) {
        loadPlugins();
        fetchDiscoveryState();
    }
    if (!unsubscribe) {
        unsubscribe = subscribe(handleEvent);
    }
}

export function onBlur() {
    unsubscribe?.();
    unsubscribe = null;
}
