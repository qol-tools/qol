export const id = 'dev';

const state = {
    reloading: false,
    lastReload: null,
    error: null,
    plugins: [],
    discovered: [],
    selectedIndex: 0,
    showLinkInput: false,
    linkPath: '',
    linkError: null
};

let container = null;

export function render(containerEl) {
    container = containerEl;
    container.addEventListener('click', handleClick);
    container.addEventListener('change', handleChange);
    loadPlugins();
}

async function loadPlugins() {
    try {
        const [linksRes, discoverRes] = await Promise.all([
            fetch('/api/dev/links'),
            fetch('/api/dev/discover')
        ]);
        if (linksRes.ok) state.plugins = await linksRes.json();
        if (discoverRes.ok) state.discovered = await discoverRes.json();
    } catch (e) {
        console.error('Failed to load plugins:', e);
    }
    updateView();
}

function isPluginSelected(i) {
    return state.selectedIndex === i;
}

function isDiscoveredSelected(i) {
    return state.selectedIndex === state.plugins.length + i;
}

function totalItems() {
    return state.plugins.length + state.discovered.length;
}

function updateView() {
    const pluginRows = state.plugins.map((p, i) => `
        <div class="plugin-row ${isPluginSelected(i) ? 'selected' : ''}" data-index="${i}">
            <label class="toggle-label">
                <input type="checkbox" ${p.is_symlink ? 'checked' : ''} ${!p.is_symlink ? 'disabled' : ''} data-id="${p.id}">
                <span class="toggle-slider"></span>
            </label>
            <div class="plugin-info">
                <span class="plugin-name">${p.name}</span>
                <span class="plugin-meta">${p.is_symlink ? p.target : 'installed'}</span>
            </div>
        </div>
    `).join('');

    container.innerHTML = `
        <div class="view-container">
            <header>
                <h1>Developer</h1>
                <p>Tools for plugin development</p>
            </header>

            <section class="dev-section">
                <h2>Plugin Links</h2>
                <p class="section-desc">Symlink local plugins for development. Toggle removes the symlink.</p>
                ${state.plugins.length ? `
                    <div class="plugin-list">${pluginRows}</div>
                ` : '<p class="empty-state">No plugins installed</p>'}

                ${state.showLinkInput ? `
                    <div class="link-input-row">
                        <input type="text" id="link-path" placeholder="/path/to/plugin" value="${state.linkPath}" autofocus>
                        <button class="btn-confirm" data-action="confirm-link">Link</button>
                        <button class="btn-cancel" data-action="cancel-link">Cancel</button>
                    </div>
                    ${state.linkError ? `<p class="error-msg">${state.linkError}</p>` : ''}
                ` : ''}
            </section>

            ${state.discovered.length ? `
            <section class="dev-section">
                <h2>Discovered Plugins</h2>
                <p class="section-desc">Found in common dev directories. Click to link.</p>
                <div class="discovered-list">
                    ${state.discovered.map((p, i) => `
                        <div class="discovered-row ${isDiscoveredSelected(i) ? 'selected' : ''}" data-path="${p.path}" data-id="${p.id}" data-disc-index="${i}">
                            <span class="discovered-name">${p.name}</span>
                            <span class="discovered-path">${p.path}</span>
                            <button class="btn-link-quick" data-action="quick-link" data-path="${p.path}" data-id="${p.id}">Link</button>
                        </div>
                    `).join('')}
                </div>
            </section>
            ` : ''}

            <section class="dev-section">
                <h2>Actions</h2>
                <div class="dev-card ${state.reloading ? 'loading' : ''}" data-action="reload">
                    <div class="dev-card-icon">&#x21bb;</div>
                    <div class="dev-card-content">
                        <h3>Reload All Plugins</h3>
                        <p>Stop all daemons and restart. Use after rebuilding.</p>
                        ${state.lastReload ? `<span class="last-action">Last: ${state.lastReload}</span>` : ''}
                        ${state.error ? `<span class="error-msg">${state.error}</span>` : ''}
                    </div>
                    <div class="dev-card-hint"><kbd>r</kbd></div>
                </div>
            </section>

            <footer class="help">
                ↑/↓ navigate &nbsp; Space toggle &nbsp; l link &nbsp; r reload
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
    const path = e.target.closest('[data-path]')?.dataset.path;

    if (action === 'reload') reloadPlugins();
    if (action === 'add-link') showLinkInput();
    if (action === 'confirm-link') confirmLink();
    if (action === 'cancel-link') cancelLink();
    if (action === 'quick-link' && path) {
        const id = e.target.dataset.id;
        quickLink(path, id);
    }

    const row = e.target.closest('.plugin-row');
    if (row && !e.target.closest('.toggle-label')) {
        state.selectedIndex = parseInt(row.dataset.index);
        updateView();
    }
}

function handleChange(e) {
    const checkbox = e.target.closest('input[type="checkbox"]');
    if (!checkbox) return;

    const id = checkbox.dataset.id;
    const plugin = state.plugins.find(p => p.id === id);
    if (!plugin || !plugin.is_symlink) return;

    e.preventDefault();
    checkbox.checked = true;
    deleteLink(id);
}

function handleItemActivation() {
    const inDiscovered = state.selectedIndex >= state.plugins.length;

    if (inDiscovered) {
        const discIndex = state.selectedIndex - state.plugins.length;
        const discovered = state.discovered[discIndex];
        if (discovered) quickLink(discovered.path, discovered.id);
        return;
    }

    const plugin = state.plugins[state.selectedIndex];
    if (!plugin?.is_symlink) return;

    deleteLink(plugin.id);
}

async function quickLink(path, id) {
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
        await loadPlugins();
    } catch (e) {
        console.error('Failed to link:', e);
    }
}

function showLinkInput() {
    state.showLinkInput = true;
    state.linkPath = '';
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
    try {
        const res = await fetch(`/api/dev/links/${id}`, { method: 'DELETE' });
        if (!res.ok) {
            console.error('Failed to delete link:', await res.text());
            return;
        }
        await triggerReload();
        await loadPlugins();
    } catch (e) {
        console.error('Failed to delete link:', e);
    }
}

async function triggerReload() {
    await fetch('/api/dev/reload', { method: 'POST' });
}

async function reloadPlugins() {
    if (state.reloading) return;

    state.reloading = true;
    state.error = null;
    updateView();

    try {
        const res = await fetch('/api/dev/reload', { method: 'POST' });
        if (res.ok) {
            state.lastReload = new Date().toLocaleTimeString();
            await loadPlugins();
        } else {
            state.error = await res.text() || 'Reload failed';
        }
    } catch (err) {
        state.error = err.message;
    } finally {
        state.reloading = false;
        updateView();
    }
}

export function handleKey(e) {
    if (state.showLinkInput) {
        return;
    }

    if (e.ctrlKey || e.altKey || e.metaKey) {
        return;
    }

    if (e.key === 'ArrowDown') {
        e.preventDefault();
        state.selectedIndex = Math.min(state.selectedIndex + 1, totalItems() - 1);
        updateView();
    }

    if (e.key === 'ArrowUp') {
        e.preventDefault();
        state.selectedIndex = Math.max(state.selectedIndex - 1, 0);
        updateView();
    }

    if (e.key === ' ' || e.key === 'Enter') {
        e.preventDefault();
        handleItemActivation();
    }

    if (e.key === 'l' || e.key === 'L') {
        e.preventDefault();
        showLinkInput();
    }

    if (e.key === 'r' || e.key === 'R') {
        e.preventDefault();
        reloadPlugins();
    }
}

export function onFocus() {
    loadPlugins();
}

export function onBlur() {}
