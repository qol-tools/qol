import { subscribe } from '../events.js';

export const id = 'dev';

const state = {
    reloading: false,
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
    linkingId: null
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
    if (state.linkingId) return;
    if (event.type === 'discovery_started') {
        state.discovering = true;
        updateView();
    } else if (event.type === 'discovery_complete') {
        state.discovering = false;
        state.discovered = event.plugins || [];
        updateView();
    } else if (event.type === 'plugins_changed') {
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

function updateView() {
    const unified = new Map();

    for (const d of state.discovered) {
        unified.set(d.id, {
            id: d.id,
            name: d.name,
            path: d.path,
            status: 'local'
        });
    }

    for (const p of state.plugins) {
        const existing = unified.get(p.id);
        if (p.is_symlink) {
            if (existing) {
                existing.status = 'linked';
                existing.path = p.target || existing.path;
            } else {
                unified.set(p.id, {
                    id: p.id,
                    name: p.name,
                    path: p.target,
                    status: 'linked'
                });
            }
        } else {
            if (existing) {
                existing.status = 'local';
                existing.hasStoreInstall = true;
            } else {
                unified.set(p.id, {
                    id: p.id,
                    name: p.name,
                    path: null,
                    status: 'installed'
                });
            }
        }
    }

    const mergedList = Array.from(unified.values()).sort((a, b) => a.name.localeCompare(b.name));
    state.mergedCount = mergedList.length;
    state.mergedList = mergedList;
    state.selectedIndex = Math.max(0, Math.min(state.selectedIndex, mergedList.length - 1));

    const pluginRows = mergedList.map((p, i) => {
        const isSelected = state.selectedIndex === i;
        const statusBadge = {
            linked: '<span class="badge badge-linked">Linked</span>',
            installed: '<span class="badge badge-installed">Installed</span>',
            local: '<span class="badge badge-local">Local Clone</span>'
        }[p.status];

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
                <div class="plugin-info">
                    <div class="plugin-header">
                        <span class="plugin-name">${p.name}</span>
                        <div class="plugin-status-badges">
                            ${statusBadge}
                            ${p.hasStoreInstall ? '<span class="badge badge-installed-dim">+Store</span>' : ''}
                        </div>
                    </div>
                    <span class="plugin-path">${p.path || ''}</span>
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
                    <button class="refresh-btn ${state.reloading ? 'spinning' : ''}" tabindex="-1">↻</button>
                    <div class="dev-card-content">
                        <h3>Reload All Plugins</h3>
                        <p>Restart daemons and rescan for local plugins.</p>
                        ${state.lastReload ? `<span class="last-action">Last: ${state.lastReload}</span>` : ''}
                        ${state.error ? `<span class="error-msg">${state.error}</span>` : ''}
                    </div>
                    <div class="dev-card-hint"><kbd>Ctrl+r</kbd></div>
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
    await fetch('/api/dev/reload', { method: 'POST' });
}

async function triggerDiscovery() {
    if (state.discovering) return;
    await fetch('/api/dev/discover', { method: 'POST' });
}

async function reloadPlugins() {
    if (state.reloading) return;

    state.reloading = true;
    state.error = null;
    updateView();

    try {
        const [reloadRes, discoverRes] = await Promise.all([
            fetch('/api/dev/reload', { method: 'POST' }),
            fetch('/api/dev/discover', { method: 'POST' })
        ]);

        if (reloadRes.ok && discoverRes.ok) {
            state.lastReload = new Date().toLocaleTimeString();
            await loadPlugins();
        } else {
            state.error = 'Reload or discovery trigger failed';
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
