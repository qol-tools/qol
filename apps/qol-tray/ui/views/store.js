import { updateSelection as updateSel, navigateGrid } from '../utils.js';
import { subscribe } from '../events.js';
import * as installing from '../installing.js';
import { renderFeedback as renderFeedbackComponent } from '../components/feedback.js';
import {
    createStoreState,
    formatCacheAge,
    normalizeSearchQuery,
    getFilteredPlugins as filterPlugins,
    clampSelectedIndex,
    sortPluginsByName,
    isRateLimitedWithoutToken,
    looksLikeGithubAuthFailure,
    isStoreUpdateAvailable
} from './store/reducer.js';
import {
    fetchTokenStatus,
    saveTokenRequest,
    deleteTokenRequest,
    fetchPluginsRequest,
    installPluginRequest
} from './store/effects.js';
import { renderShortcutLegend } from '../components/shortcut-legend.js';

export const id = 'store';

const state = createStoreState();

let searchInput = null;
let unsubscribe = null;

export function render(containerEl) {
    containerEl.innerHTML = `
        <div class="view-container">
            <header>
                <div class="header-row">
                    <div>
                        <h1>Plugin Store</h1>
                        <p>Browse and install plugins for QoL Tray</p>
                    </div>
                    <div class="header-actions">
                        <span id="cache-age" class="cache-age"></span>
                        <button id="manage-token-btn" class="btn btn-ghost btn-sm" title="Manage GitHub token">Token</button>
                        <button id="refresh-btn" class="refresh-btn" title="Refresh (r)" aria-label="Refresh"></button>
                    </div>
                </div>
            </header>
            <div class="view-body">
                <div class="search-bar store-search-bar">
                    <input type="text" id="store-search" placeholder="Search plugins...">
                </div>
                <div id="token-banner"></div>
                <div id="store-feedback"></div>
                <div id="store-list" class="plugins-grid grid-cards grid-cards--zoom">
                    <div class="loading">Loading plugins...</div>
                </div>
            </div>
        </div>
    `;
    document.getElementById('content-footer').innerHTML = renderShortcutLegend([
        { key: '←↑↓→', label: 'navigate' },
        { key: 'Enter', label: 'install' },
        { key: '/', label: 'search' },
        { key: 't', label: 'token' },
        { key: '⌘R', label: 'refresh' }
    ]);
    
    searchInput = document.getElementById('store-search');
    if (searchInput) {
        searchInput.addEventListener('input', handleSearch);
    }
    
    const listEl = document.getElementById('store-list');
    if (listEl) {
        listEl.addEventListener('click', handleListClick);
    }
    
    document.getElementById('refresh-btn')?.addEventListener('click', () => refreshPlugins());
    document.getElementById('manage-token-btn')?.addEventListener('click', () => {
        state.showTokenInput = true;
        renderTokenBanner();
        document.getElementById('github-token-input')?.focus();
    });
    
    initializeStoreState();
    unsubscribe = subscribe((event) => {
        if (event.type === 'plugins_changed') loadPlugins();
    });
}

async function initializeStoreState() {
    await checkTokenStatus();
    await loadPlugins();
}

function renderFeedback() {
    const el = document.getElementById('store-feedback');
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

async function checkTokenStatus() {
    state.hasToken = await fetchTokenStatus();
}

function renderTokenBanner() {
    const banner = document.getElementById('token-banner');
    if (!banner) return;

    if (state.showTokenInput) {
        renderTokenInput(banner);
    } else if (state.rateLimited && !state.hasToken) {
        renderRateLimitMessage(banner);
    } else {
        banner.innerHTML = '';
    }
}

function renderTokenInput(banner) {
    const removeButton = state.hasToken
        ? '<button id="remove-token-btn" class="btn btn-ghost">Remove Token</button>'
        : '';
    banner.innerHTML = `
        <div class="token-input-container">
            <input type="password" id="github-token-input" placeholder="Paste GitHub token (no scopes needed)">
            <button id="save-token-btn" class="btn btn-primary">Save</button>
            ${removeButton}
            <button id="cancel-token-btn" class="btn btn-ghost">Cancel</button>
        </div>
        <p class="token-help">
            <a href="https://github.com/settings/tokens/new" target="_blank">Create token</a> — no scopes needed, just for rate limits
        </p>
    `;

    document.getElementById('save-token-btn')?.addEventListener('click', saveToken);
    document.getElementById('remove-token-btn')?.addEventListener('click', deleteToken);
    document.getElementById('cancel-token-btn')?.addEventListener('click', () => {
        state.showTokenInput = false;
        renderTokenBanner();
    });
}

function renderRateLimitMessage(banner) {
    banner.innerHTML = `
        <div class="rate-limit-banner">
            <span>GitHub API rate limit reached.</span>
            <button id="add-token-btn" class="btn btn-primary">Add GitHub Token</button>
        </div>
    `;

    document.getElementById('add-token-btn')?.addEventListener('click', () => {
        state.showTokenInput = true;
        renderTokenBanner();
        document.getElementById('github-token-input')?.focus();
    });
}

async function saveToken() {
    const input = document.getElementById('github-token-input');
    const token = input?.value?.trim();
    
    if (!token) {
        setFeedback('error', 'Token cannot be empty');
        return;
    }
    
    clearFeedback();
    try {
        await saveTokenRequest(token);
        state.hasToken = true;
        state.showTokenInput = false;
        state.rateLimited = false;
        renderTokenBanner();
        setFeedback('success', 'GitHub token saved');
        loadPlugins();
    } catch (e) {
        setFeedback('error', `Failed to save token: ${e.message}`);
        input?.focus();
        input?.select();
    }
}

async function deleteToken() {
    clearFeedback();
    try {
        await deleteTokenRequest();
        state.hasToken = false;
        state.showTokenInput = false;
        renderTokenBanner();
        setFeedback('success', 'GitHub token removed');
    } catch (e) {
        setFeedback('error', `Failed to delete token: ${e.message}`);
    }
}

async function loadPlugins(forceRefresh = false) {
    const listEl = document.getElementById('store-list');
    if (!listEl) return;

    const token = ++state.loadToken;
    
    state.loading = true;
    updateRefreshButton();
    
    try {
        const data = await fetchPluginsRequest(forceRefresh);
        if (token !== state.loadToken) {
            return;
        }
        state.plugins = sortPluginsByName(data.plugins || []);
        state.cacheAgeSecs = data.cache_age_secs ?? null;
        state.rateLimited = isRateLimitedWithoutToken(state.plugins, state.hasToken);
        state.showTokenInput = state.showTokenInput && state.rateLimited;
        renderTokenBanner();
        
        const filtered = getVisiblePlugins();
        state.selectedIndex = clampSelectedIndex(state.selectedIndex, filtered.length);
        renderPlugins(filtered);
        updateSelection();
        updateCacheAge();
    } catch (error) {
        if (token !== state.loadToken) {
            return;
        }
        if (looksLikeGithubAuthFailure(error?.message)) {
            state.rateLimited = true;
            state.showTokenInput = true;
            renderTokenBanner();
        }
        setFeedback('error', `Failed to load plugins: ${error.message}`);
        if (listEl) {
            listEl.innerHTML = `<div class="error">Error loading plugins: ${error.message}</div>`;
        }
    } finally {
        if (token === state.loadToken) {
            state.loading = false;
            updateRefreshButton();
        }
    }
}

function refreshPlugins() {
    if (state.loading) return;
    loadPlugins(true);
}

function updateCacheAge() {
    const el = document.getElementById('cache-age');
    if (el) {
        el.textContent = formatCacheAge(state.cacheAgeSecs);
    }
}

function updateRefreshButton() {
    const btn = document.getElementById('refresh-btn');
    if (btn) {
        btn.disabled = state.loading;
        btn.classList.toggle('spinning', state.loading);
    }
}

function renderPlugins(plugins) {
    const listEl = document.getElementById('store-list');
    if (!listEl) return;

    if (plugins.length === 0) {
        listEl.innerHTML = '<div class="loading">No plugins found</div>';
        return;
    }

    listEl.innerHTML = plugins.map((plugin, index) => {
        const isInstalling = installing.has(plugin.id);
        const hasUpdate = isStoreUpdateAvailable(plugin);
        const versionDisplay = hasUpdate
            ? `v${plugin.installed_version} → v${plugin.version}`
            : `v${plugin.version}`;
        return `
            <div class="plugin-card ${plugin.installed ? 'installed' : ''} ${isInstalling ? 'installing' : ''}" data-index="${index}" data-plugin-id="${plugin.id}" data-installed="${plugin.installed}">
                <h3>${plugin.name}</h3>
                <div class="version${hasUpdate ? ' has-update' : ''}">${versionDisplay}</div>
                <div class="description">${plugin.description}</div>
                <div class="button-group">
                    ${plugin.installed ? `
                        <span class="installed-badge">${hasUpdate ? 'Update Available' : 'Installed'}</span>
                    ` : isInstalling ? `
                        <button class="refresh-btn spinning" disabled></button>
                    ` : `
                        <button class="btn btn-primary install" style="width: 100%">Install</button>
                    `}
                </div>
            </div>
        `;
    }).join('');
}
function handleListClick(e) {
    const card = e.target.closest('.plugin-card');
    if (!card) return;
    
    if (e.target.tagName === 'BUTTON' && e.target.classList.contains('install')) {
        const pluginId = card.dataset.pluginId;
        installPlugin(pluginId);
        return;
    }
    
    const index = parseInt(card.dataset.index, 10);
    if (index !== state.selectedIndex) {
        state.selectedIndex = index;
        updateSelection();
    }
}

function handleSearch(e) {
    state.searchQuery = normalizeSearchQuery(e.target.value);
    const filtered = getVisiblePlugins();
    state.selectedIndex = clampSelectedIndex(state.selectedIndex, filtered.length);
    renderPlugins(filtered);
    updateSelection();
}

function updateSelection() {
    updateSel('.plugin-card', state.selectedIndex);
}

export function handleKey(e) {
    if (document.activeElement === searchInput) {
        if (e.key === 'Escape') {
            searchInput.blur();
            e.preventDefault();
            return;
        }
        if ((e.ctrlKey || e.metaKey) && e.key === 'r') {
            e.preventDefault();
            refreshPlugins();
            return;
        }
        return;
    }

    if (state.showTokenInput && e.key === 'Escape') {
        state.showTokenInput = false;
        renderTokenBanner();
        e.preventDefault();
        return;
    }

    if ((e.ctrlKey || e.metaKey) && e.key === 'r') {
        e.preventDefault();
        refreshPlugins();
        return;
    }

    const handler = keyHandlers[e.key];
    if (handler) {
        e.preventDefault();
        handler();
    }
}

function installSelected() {
    const selected = document.querySelector('.plugin-card.selected');
    if (!selected) return;
    
    const isInstalled = selected.dataset.installed === 'true';
    if (isInstalled) return;
    
    installPlugin(selected.dataset.pluginId);
}

const keyHandlers = {
    ArrowUp: () => navigateVertical(-1),
    ArrowDown: () => navigateVertical(1),
    ArrowLeft: () => navigateHorizontal(-1),
    ArrowRight: () => navigateHorizontal(1),
    Enter: installSelected,
    '/': () => { searchInput?.focus(); },
    't': openTokenInput,
    'T': openTokenInput
};

function navigateVertical(rowDelta) {
    const direction = rowDelta < 0 ? 'up' : 'down';
    navigateInGrid(direction);
}

function navigateHorizontal(colDelta) {
    const direction = colDelta < 0 ? 'left' : 'right';
    navigateInGrid(direction);
}

function navigateInGrid(direction) {
    const nextIndex = navigateGrid('#store-list .plugin-card', state.selectedIndex, direction);
    if (nextIndex === state.selectedIndex) return;
    state.selectedIndex = nextIndex;
    updateSelection();
}

function getVisiblePlugins() {
    return filterPlugins(state.plugins, state.searchQuery);
}

async function installPlugin(id) {
    if (installing.has(id)) return;

    const plugin = state.plugins.find(p => p.id === id);
    clearFeedback();
    installing.add(id, plugin?.name || id);
    renderPlugins(getVisiblePlugins());
    updateSelection();

    try {
        await installPluginRequest(id);
        setFeedback('success', `Installed ${plugin?.name || id}`);
    } catch (error) {
        setFeedback('error', `Failed to install ${plugin?.name || id}: ${error.message}`);
    } finally {
        installing.remove(id);
        await loadPlugins();
    }
}

function openTokenInput() {
    state.showTokenInput = true;
    renderTokenBanner();
    document.getElementById('github-token-input')?.focus();
}

export function onFocus() {
    updateSelection();
}

export function onBlur() {
    searchInput?.blur();
    unsubscribe?.();
    unsubscribe = null;
}
