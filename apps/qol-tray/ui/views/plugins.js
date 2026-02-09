import { updateSelection as updateSel, navigateGrid } from '../utils.js';
import { subscribe } from '../events.js';
import * as installing from '../installing.js';

const PLACEHOLDER_SVG = 'data:image/svg+xml,' + encodeURIComponent(
    '<svg xmlns="http://www.w3.org/2000/svg" width="300" height="200">' +
    '<rect fill="#333" width="300" height="200"/>' +
    '<text fill="#666" x="50%" y="50%" text-anchor="middle" dy=".3em" font-family="sans-serif" font-size="14">No Cover</text>' +
    '</svg>'
);

export const id = 'plugins';

const state = {
    plugins: [],
    selectedIndex: 0,
    contextMenuOpen: false,
    confirmModalOpen: false,
    pendingUninstallId: null,
    updating: new Set(),
    refreshToken: 0,
    restoredSelection: false,
    latestRevision: 0
};

let container = null;
let unsubscribe = null;
let unsubscribeInstalling = null;
let clickHandler = null;

export function render(containerEl) {
    container = containerEl;
    container.innerHTML = `
        <div class="view-container">
            <header>
                <h1>Plugins</h1>
            </header>
            <div id="plugins-grid" class="plugin-grid grid-cards grid-cards--zoom"></div>
            <footer class="help">
                ←↑↓→ navigate • Enter open • u update • d delete
            </footer>
        </div>
    `;

    loadPlugins();
    unsubscribe = subscribe((event) => {
        if (event.type !== 'plugins_changed') return;
        const revision = Number.isInteger(event.revision) ? event.revision : state.latestRevision;
        state.latestRevision = Math.max(state.latestRevision, revision);
        refreshPlugins({ minRevision: revision });
    });
    unsubscribeInstalling = installing.subscribe(() => renderGrid());
}

function parseInstalledResponse(payload) {
    if (Array.isArray(payload)) {
        return { revision: 0, plugins: payload };
    }
    return {
        revision: Number.isInteger(payload?.revision) ? payload.revision : 0,
        plugins: Array.isArray(payload?.plugins) ? payload.plugins : []
    };
}

async function loadPlugins() {
    const gridEl = document.getElementById('plugins-grid');
    if (!gridEl) return;

    clickHandler = handleClick;
    container.addEventListener('click', clickHandler);

    await refreshPlugins({ showErrorInGrid: true, restoreSavedSelection: true });
    checkForUpdates();
}

async function checkForUpdates() {
    try {
        await fetch('/api/plugins');
        await refreshPlugins();
    } catch (e) {}
}

function restoreSelection() {
    const saved = localStorage.getItem('plugins-selected-index');
    if (saved !== null) {
        const index = parseInt(saved, 10);
        if (index >= 0 && index < state.plugins.length) {
            state.selectedIndex = index;
        }
    }
}

function saveSelection() {
    localStorage.setItem('plugins-selected-index', state.selectedIndex.toString());
}

function renderGrid() {
    const gridEl = document.getElementById('plugins-grid');
    if (!gridEl) return;

    const installingPlugins = installing.getAll();
    const installedIds = new Set(state.plugins.map(p => p.id));
    const ghostPlugins = installingPlugins.filter(p => !installedIds.has(p.id));

    if (state.plugins.length === 0 && ghostPlugins.length === 0) {
        gridEl.innerHTML = '<div class="empty">No plugins installed. Press Tab to open the store.</div>';
        return;
    }

    const ghostCards = ghostPlugins.map(plugin => `
        <div class="plugin-card ghost">
            <span class="ghost-spinner">↻</span>
            <div class="plugin-name">${plugin.name}</div>
        </div>
    `).join('');

    const pluginCards = state.plugins.map((plugin, index) => {
        const coverUrl = plugin.has_cover ? `/api/cover/${plugin.id}` : PLACEHOLDER_SVG;
        const noUiClass = plugin.has_ui ? '' : 'no-ui';
        const updateClass = plugin.update_available ? 'has-update' : '';
        const isUpdating = state.updating.has(plugin.id);

        return `
            <div class="plugin-card ${noUiClass} ${updateClass}" data-index="${index}" data-plugin-id="${plugin.id}">
                <img src="${coverUrl}" alt="${plugin.name}" onerror="this.src='${PLACEHOLDER_SVG}'">
                <div class="plugin-name">${plugin.name}</div>
                ${plugin.update_available ? `
                    <button class="plugin-update ${isUpdating ? 'updating' : ''}" aria-label="Update plugin" ${isUpdating ? 'disabled' : ''}>
                        ${isUpdating ? '<span class="refresh-btn spinning"></span>' : `↑ ${plugin.available_version}`}
                    </button>
                ` : ''}
                <button class="plugin-cog" aria-label="Plugin options"><svg width="4" height="16" viewBox="0 0 4 16" fill="currentColor"><circle cx="2" cy="2" r="2"/><circle cx="2" cy="8" r="2"/><circle cx="2" cy="14" r="2"/></svg></button>
                <div class="plugin-context-menu">
                    ${plugin.update_available ? '<button class="context-update">Update</button>' : ''}
                    <button class="context-delete">Delete</button>
                </div>
            </div>
        `;
    }).join('');

    gridEl.innerHTML = ghostCards + pluginCards;
}

function updateSelection() {
    updateSel('.plugin-card', state.selectedIndex);
}

const clickHandlers = [
    {
        selector: '.plugin-update:not([disabled])',
        handler: el => updatePlugin(el.closest('.plugin-card').dataset.pluginId)
    },
    {
        selector: '.context-update',
        handler: el => {
            closeAllContextMenus();
            updatePlugin(el.closest('.plugin-card').dataset.pluginId);
        }
    },
    {
        selector: '.plugin-cog',
        handler: el => toggleContextMenu(el.closest('.plugin-card'))
    },
    {
        selector: '.context-delete',
        handler: el => {
            const pluginId = el.closest('.plugin-card').dataset.pluginId;
            closeAllContextMenus();
            showConfirmModal(pluginId);
        }
    }
];

function handleClick(e) {
    if (state.confirmModalOpen) {
        handleModalClick(e);
        return;
    }

    for (const { selector, handler } of clickHandlers) {
        const target = e.target.closest(selector);
        if (target) {
            e.stopPropagation();
            handler(target);
            return;
        }
    }

    if (state.contextMenuOpen) {
        closeAllContextMenus();
        return;
    }

    handleCardClick(e);
}

function handleCardClick(e) {
    const card = e.target.closest('.plugin-card');
    if (!card) return;

    const index = parseInt(card.dataset.index, 10);
    if (index !== state.selectedIndex) {
        state.selectedIndex = index;
        updateSelection();
    } else {
        openSelected();
    }
}

function toggleContextMenu(card) {
    const menu = card.querySelector('.plugin-context-menu');
    const wasOpen = menu.classList.contains('open');
    
    closeAllContextMenus();
    
    if (!wasOpen) {
        menu.classList.add('open');
        state.contextMenuOpen = true;
    }
}

function closeAllContextMenus() {
    document.querySelectorAll('.plugin-context-menu.open').forEach(m => m.classList.remove('open'));
    state.contextMenuOpen = false;
}

function showConfirmModal(pluginId) {
    state.pendingUninstallId = pluginId;
    state.confirmModalOpen = true;
    
    const plugin = state.plugins.find(p => p.id === pluginId);
    const pluginName = plugin ? plugin.name : pluginId;
    
    const modal = document.createElement('div');
    modal.className = 'confirm-modal';
    modal.innerHTML = `
        <div class="confirm-modal-content">
            <h3>Delete "${pluginName}"?</h3>
            <p>This will uninstall the plugin and remove all its data.</p>
            <div class="confirm-modal-buttons">
                <button class="btn btn-ghost confirm-cancel">Cancel (Esc)</button>
                <button class="btn btn-danger confirm-delete">Delete (Enter)</button>
            </div>
        </div>
    `;
    
    container.appendChild(modal);
}

function handleModalClick(e) {
    if (e.target.closest('.confirm-cancel') || e.target.classList.contains('confirm-modal')) {
        closeConfirmModal();
        return;
    }
    
    if (e.target.closest('.confirm-delete')) {
        confirmUninstall();
        return;
    }
}

function closeConfirmModal() {
    const modal = container.querySelector('.confirm-modal');
    if (modal) modal.remove();
    state.confirmModalOpen = false;
    state.pendingUninstallId = null;
}

async function confirmUninstall() {
    const pluginId = state.pendingUninstallId;
    closeConfirmModal();
    
    if (!pluginId) return;
    
    try {
        const response = await fetch(`/api/uninstall/${pluginId}`, { method: 'POST' });
        const result = await response.json();
        
        if (!result.success) throw new Error(result.message);
        await refreshPlugins();
    } catch (error) {
        console.error(`Failed to uninstall plugin: ${error.message}`);
    }
}

async function updatePlugin(pluginId) {
    if (state.updating.has(pluginId)) return;
    
    state.updating.add(pluginId);
    renderGrid();
    updateSelection();
    
    try {
        const response = await fetch(`/api/update/${pluginId}`, { method: 'POST' });
        const result = await response.json();
        
        if (!result.success) throw new Error(result.message);
    } catch (error) {
        console.error(`Failed to update plugin: ${error.message}`);
    } finally {
        state.updating.delete(pluginId);
        await refreshPlugins();
    }
}

async function refreshPlugins(options = {}) {
    const { showErrorInGrid = false, restoreSavedSelection = false, minRevision = 0 } = options;
    const gridEl = document.getElementById('plugins-grid');
    if (!gridEl) return;

    const token = ++state.refreshToken;

    try {
        const response = await fetch('/api/installed');
        if (!response.ok) throw new Error('Failed to fetch plugins');

        const payload = parseInstalledResponse(await response.json());
        if (token !== state.refreshToken) {
            return;
        }
        if (payload.revision < minRevision || payload.revision < state.latestRevision) {
            return;
        }

        state.latestRevision = payload.revision;
        state.plugins = payload.plugins;
        state.plugins.sort((a, b) => a.name.localeCompare(b.name));
        if (restoreSavedSelection && !state.restoredSelection) {
            restoreSelection();
            state.restoredSelection = true;
        }
        state.selectedIndex = Math.min(state.selectedIndex, Math.max(0, state.plugins.length - 1));
        renderGrid();
        updateSelection();
    } catch (error) {
        if (token !== state.refreshToken) {
            return;
        }
        if (showErrorInGrid) {
            gridEl.innerHTML = `<div class="error">Error loading plugins: ${error.message}</div>`;
            return;
        }
        console.error(`Failed to refresh plugins: ${error.message}`);
    }
}

function updateSelected() {
    const plugin = state.plugins[state.selectedIndex];
    if (plugin?.update_available) {
        updatePlugin(plugin.id);
    }
}

export function handleKey(e) {
    if (state.confirmModalOpen) {
        handleModalKey(e);
        return;
    }
    
    if (state.contextMenuOpen) {
        handleContextMenuKey(e);
        return;
    }
    
    const handler = keyHandlers[e.key];
    if (handler) {
        e.preventDefault();
        handler();
    }
}

function handleModalKey(e) {
    if (e.key === 'Escape') {
        e.preventDefault();
        closeConfirmModal();
    } else if (e.key === 'Enter') {
        e.preventDefault();
        confirmUninstall();
    }
}

function handleContextMenuKey(e) {
    if (e.key === 'Escape') {
        e.preventDefault();
        closeAllContextMenus();
        return;
    }
    
    if (e.key !== 'Enter') return;
    
    e.preventDefault();
    const plugin = state.plugins[state.selectedIndex];
    if (!plugin) return;
    
    closeAllContextMenus();
    showConfirmModal(plugin.id);
}

function deleteSelected() {
    const plugin = state.plugins[state.selectedIndex];
    if (plugin) showConfirmModal(plugin.id);
}

const keyHandlers = {
    ArrowUp: () => navigateInGrid('up'),
    ArrowDown: () => navigateInGrid('down'),
    ArrowLeft: () => navigateInGrid('left'),
    ArrowRight: () => navigateInGrid('right'),
    Enter: openSelected,
    d: deleteSelected,
    D: deleteSelected,
    u: updateSelected,
    U: updateSelected
};

function navigateInGrid(direction) {
    const nextIndex = navigateGrid('#plugins-grid .plugin-card', state.selectedIndex, direction);
    if (nextIndex === state.selectedIndex) return;
    state.selectedIndex = nextIndex;
    updateSelection();
}

function openSelected() {
    if (state.plugins.length === 0) return;
    
    const plugin = state.plugins[state.selectedIndex];
    if (plugin.has_ui) {
        saveSelection();
        window.location.href = `/plugins/${plugin.id}/`;
    }
}

export function onFocus() {
    refreshPlugins();
    updateSelection();
}

export function onBlur() {
    closeAllContextMenus();
    closeConfirmModal();
    unsubscribe?.();
    unsubscribe = null;
    unsubscribeInstalling?.();
    unsubscribeInstalling = null;
    if (clickHandler) {
        container?.removeEventListener('click', clickHandler);
        clickHandler = null;
    }
}
