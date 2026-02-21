import { updateSelection as updateSel, navigateGrid } from '../utils.js';
import { subscribe } from '../events.js';
import * as installing from '../installing.js';
import { apiJson } from '../api/client.js';
import { renderFeedback as renderFeedbackComponent } from '../components/feedback.js';
import { closeModal, matchModalAction, openModal } from '../components/modal.js';
import { parseInstalledPayload } from '../utils/plugins.js';

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
    latestRevision: 0,
    feedback: null
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
            <div id="plugins-feedback"></div>
            <div id="plugins-grid" class="plugin-grid grid-cards grid-cards--zoom"></div>
            <footer class="help">
                ←↑↓→ navigate • Enter settings • u update • d delete
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

function renderFeedback() {
    const el = document.getElementById('plugins-feedback');
    renderFeedbackComponent(el, state.feedback);
}

function setFeedback(type, message) {
    state.feedback = { type, message };
    renderFeedback();
}

function clearFeedback() {
    if (!state.feedback) return;
    state.feedback = null;
    renderFeedback();
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
            <span class="refresh-btn spinning"></span>
            <div class="plugin-name">${plugin.name}</div>
        </div>
    `).join('');

    const pluginCards = state.plugins.map((plugin, index) => {
        const coverUrl = plugin.has_cover ? `/api/cover/${plugin.id}` : PLACEHOLDER_SVG;
        const noUiClass = plugin.has_ui ? '' : 'no-ui';
        const updateClass = plugin.update_available ? 'has-update' : '';
        const loadClass = plugin.loaded === false ? 'not-loaded' : '';
        const isUpdating = state.updating.has(plugin.id);

        return `
            <div class="plugin-card ${noUiClass} ${updateClass} ${loadClass}" data-index="${index}" data-plugin-id="${plugin.id}">
                <img src="${coverUrl}" alt="${plugin.name}" onerror="this.src='${PLACEHOLDER_SVG}'">
                <div class="plugin-name">${plugin.name}</div>
                ${plugin.loaded === false ? '<div class="plugin-load-state">Not loaded</div>' : ''}
                ${plugin.update_available ? `
                    <button class="plugin-update ${isUpdating ? 'updating' : ''}" aria-label="Update plugin" ${isUpdating ? 'disabled' : ''}>
                        ${isUpdating ? '<span class="refresh-btn spinning update-spinner"></span>' : `↑ ${plugin.available_version}`}
                    </button>
                ` : ''}
                <button class="plugin-cog" aria-label="Plugin options">
                    <svg class="plugin-cog-icon" viewBox="0 0 12 20" fill="currentColor" aria-hidden="true" focusable="false">
                        <circle cx="6" cy="3.5" r="1.8"></circle>
                        <circle cx="6" cy="10" r="1.8"></circle>
                        <circle cx="6" cy="16.5" r="1.8"></circle>
                    </svg>
                </button>
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
    
    openModal(container, {
        className: 'confirm-modal',
        html: `
        <div class="confirm-modal-content">
            <h3>Delete "${pluginName}"?</h3>
            <p>This will uninstall the plugin and remove all its data.</p>
            <div class="confirm-modal-buttons">
                <button class="btn btn-ghost confirm-cancel">Cancel (Esc)</button>
                <button class="btn btn-danger confirm-delete">Delete (Enter)</button>
            </div>
        </div>
    `
    });
}

function handleModalClick(e) {
    const action = matchModalAction(e, {
        backdropClass: 'confirm-modal',
        cancelSelectors: ['.confirm-cancel'],
        confirmSelectors: ['.confirm-delete']
    });
    if (action === 'cancel') closeConfirmModal();
    if (action === 'confirm') confirmUninstall();
}

function closeConfirmModal() {
    closeModal(container, '.confirm-modal');
    state.confirmModalOpen = false;
    state.pendingUninstallId = null;
}

async function confirmUninstall() {
    const pluginId = state.pendingUninstallId;
    closeConfirmModal();
    
    if (!pluginId) return;
    
    clearFeedback();
    try {
        const result = await apiJson(`/api/uninstall/${pluginId}`, { method: 'POST' });
        if (!result.success) throw new Error(result.message);
        setFeedback('success', `Uninstalled ${pluginId}`);
        await refreshPlugins();
    } catch (error) {
        setFeedback('error', `Failed to uninstall ${pluginId}: ${error.message}`);
    }
}

async function updatePlugin(pluginId) {
    if (state.updating.has(pluginId)) return;
    
    clearFeedback();
    state.updating.add(pluginId);
    renderGrid();
    updateSelection();
    
    try {
        const result = await apiJson(`/api/update/${pluginId}`, { method: 'POST' });
        if (!result.success) throw new Error(result.message);
        setFeedback('success', `Updated ${pluginId}`);
    } catch (error) {
        setFeedback('error', `Failed to update ${pluginId}: ${error.message}`);
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
        const payload = parseInstalledPayload(await apiJson('/api/installed'));
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
        setFeedback('error', `Failed to refresh plugins: ${error.message}`);
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
    if (!plugin) return;

    if (plugin.loaded === false) {
        setFeedback('error', `Plugin ${plugin.name} is not loaded${plugin.load_error ? `: ${plugin.load_error}` : ''}`);
        return;
    }

    if (plugin.has_ui) {
        saveSelection();
        window.location.href = `/plugins/${plugin.id}/`;
        return;
    }

    setFeedback('info', `No settings UI available for ${plugin.name}`);
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
