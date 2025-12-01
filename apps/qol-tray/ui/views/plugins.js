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
    columns: 4,
    contextMenuOpen: false,
    confirmModalOpen: false,
    pendingUninstallId: null
};

let container = null;

export function render(containerEl) {
    container = containerEl;
    container.innerHTML = `
        <div class="view-container">
            <header>
                <h1>Plugins</h1>
            </header>
            <div id="plugins-grid" class="plugin-grid"></div>
            <footer class="help">
                ←↑↓→ navigate • Enter open • d delete
            </footer>
        </div>
    `;
    
    loadPlugins();
}

async function loadPlugins() {
    const gridEl = document.getElementById('plugins-grid');
    if (!gridEl) return;
    
    gridEl.addEventListener('click', handleClick);
    
    try {
        const response = await fetch('/api/installed');
        if (!response.ok) throw new Error('Failed to fetch plugins');
        
        state.plugins = await response.json();
        state.plugins.sort((a, b) => a.name.localeCompare(b.name));
        renderGrid();
        updateSelection();
    } catch (error) {
        gridEl.innerHTML = `<div class="error">Error loading plugins: ${error.message}</div>`;
    }
}

function renderGrid() {
    const gridEl = document.getElementById('plugins-grid');
    if (!gridEl) return;
    
    if (state.plugins.length === 0) {
        gridEl.innerHTML = '<div class="empty">No plugins installed. Press Tab to open the store.</div>';
        return;
    }
    
    gridEl.innerHTML = state.plugins.map((plugin, index) => {
        const coverUrl = plugin.has_cover ? `/api/cover/${plugin.id}` : PLACEHOLDER_SVG;
        const noUiClass = plugin.has_ui ? '' : 'no-ui';
        
        return `
            <div class="plugin-card ${noUiClass}" data-index="${index}" data-plugin-id="${plugin.id}">
                <img src="${coverUrl}" alt="${plugin.name}" onerror="this.src='${PLACEHOLDER_SVG}'">
                <div class="plugin-name">${plugin.name}</div>
                <button class="plugin-cog" aria-label="Plugin options">⚙</button>
                <div class="plugin-context-menu">
                    <button class="context-delete">Delete</button>
                </div>
            </div>
        `;
    }).join('');
}

function updateSelection() {
    document.querySelectorAll('.plugin-card').forEach((card, i) => {
        card.classList.toggle('selected', i === state.selectedIndex);
    });
    
    const selected = document.querySelector('.plugin-card.selected');
    if (selected) {
        selected.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
    }
}

function handleClick(e) {
    if (state.confirmModalOpen) {
        handleModalClick(e);
        return;
    }
    
    const cog = e.target.closest('.plugin-cog');
    if (cog) {
        e.stopPropagation();
        const card = cog.closest('.plugin-card');
        toggleContextMenu(card);
        return;
    }
    
    const deleteBtn = e.target.closest('.context-delete');
    if (deleteBtn) {
        e.stopPropagation();
        const card = deleteBtn.closest('.plugin-card');
        const pluginId = card.dataset.pluginId;
        closeAllContextMenus();
        showConfirmModal(pluginId);
        return;
    }
    
    if (state.contextMenuOpen) {
        closeAllContextMenus();
        return;
    }
    
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
                <button class="confirm-cancel">Cancel (Esc)</button>
                <button class="confirm-delete">Delete (Enter)</button>
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
        
        state.plugins = state.plugins.filter(p => p.id !== pluginId);
        state.selectedIndex = Math.min(state.selectedIndex, Math.max(0, state.plugins.length - 1));
        renderGrid();
        updateSelection();
    } catch (error) {
        console.error(`Failed to uninstall plugin: ${error.message}`);
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
    ArrowUp: () => navigate(-state.columns),
    ArrowDown: () => navigate(state.columns),
    ArrowLeft: () => navigate(-1),
    ArrowRight: () => navigate(1),
    Enter: openSelected,
    d: deleteSelected,
    D: deleteSelected
};

function navigate(delta) {
    const total = state.plugins.length;
    if (total === 0) return;
    
    const newIndex = Math.max(0, Math.min(total - 1, state.selectedIndex + delta));
    
    if (newIndex !== state.selectedIndex) {
        state.selectedIndex = newIndex;
        updateSelection();
    }
}

function openSelected() {
    if (state.plugins.length === 0) return;
    
    const plugin = state.plugins[state.selectedIndex];
    if (plugin.has_ui) {
        window.location.href = `/plugins/${plugin.id}/`;
    }
}

export function onFocus() {
    updateSelection();
}

export function onBlur() {
}

export function cleanup() {
    const grid = document.getElementById('plugins-grid');
    if (grid) {
        grid.removeEventListener('click', handleClick);
    }
}

