export const id = 'store';

const state = {
    plugins: [],
    selectedIndex: 0,
    searchQuery: '',
    hasToken: false,
    showTokenInput: false
};

let container = null;
let searchInput = null;

export function render(containerEl) {
    container = containerEl;
    container.innerHTML = `
        <div class="view-container">
            <header>
                <h1>Plugin Store</h1>
                <p>Browse and install plugins for QoL Tray</p>
            </header>
            <div class="search-bar">
                <input type="text" id="store-search" placeholder="Search plugins...">
            </div>
            <div id="token-banner"></div>
            <div id="store-list" class="plugins-grid">
                <div class="loading">Loading plugins...</div>
            </div>
        </div>
    `;
    
    searchInput = document.getElementById('store-search');
    if (searchInput) {
        searchInput.addEventListener('input', handleSearch);
    }
    
    const listEl = document.getElementById('store-list');
    if (listEl) {
        listEl.addEventListener('click', handleListClick);
    }
    
    checkTokenStatus();
    loadPlugins();
}

async function checkTokenStatus() {
    try {
        const response = await fetch('/api/github-token');
        const data = await response.json();
        state.hasToken = data.has_token;
    } catch (e) {
        state.hasToken = false;
    }
}

function showRateLimitBanner() {
    const banner = document.getElementById('token-banner');
    if (!banner) return;
    
    if (state.showTokenInput) {
        banner.innerHTML = `
            <div class="token-input-container">
                <input type="password" id="github-token-input" placeholder="Paste GitHub token (no scopes needed)">
                <button id="save-token-btn">Save</button>
                <button id="cancel-token-btn">Cancel</button>
            </div>
            <p class="token-help">
                <a href="https://github.com/settings/tokens/new" target="_blank">Create token</a> — no scopes needed, just for rate limits
            </p>
        `;
        
        document.getElementById('save-token-btn')?.addEventListener('click', saveToken);
        document.getElementById('cancel-token-btn')?.addEventListener('click', () => {
            state.showTokenInput = false;
            showRateLimitBanner();
        });
    } else {
        banner.innerHTML = `
            <div class="rate-limit-banner">
                <span>GitHub API rate limit reached.</span>
                <button id="add-token-btn">Add GitHub Token</button>
            </div>
        `;
        
        document.getElementById('add-token-btn')?.addEventListener('click', () => {
            state.showTokenInput = true;
            showRateLimitBanner();
            document.getElementById('github-token-input')?.focus();
        });
    }
}

async function saveToken() {
    const input = document.getElementById('github-token-input');
    const token = input?.value?.trim();
    
    if (!token) return;
    
    try {
        const response = await fetch('/api/github-token', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ token })
        });
        
        if (response.ok) {
            state.hasToken = true;
            state.showTokenInput = false;
            document.getElementById('token-banner').innerHTML = '';
            loadPlugins();
        }
    } catch (e) {
        console.error('Failed to save token:', e);
    }
}

async function loadPlugins() {
    const listEl = document.getElementById('store-list');
    if (!listEl) return;
    
    try {
        const response = await fetch('/api/plugins');
        if (!response.ok) throw new Error('Failed to fetch plugins');
        
        state.plugins = await response.json();
        
        if (state.plugins.length === 0 && !state.hasToken) {
            showRateLimitBanner();
        }
        
        state.plugins.sort((a, b) => a.name.localeCompare(b.name));
        renderPlugins(state.plugins);
        updateSelection();
    } catch (error) {
        if (listEl) {
            listEl.innerHTML = `<div class="error">Error loading plugins: ${error.message}</div>`;
        }
    }
}

function renderPlugins(plugins) {
    const listEl = document.getElementById('store-list');
    if (!listEl) return;
    
    if (plugins.length === 0) {
        listEl.innerHTML = '<div class="loading">No plugins found</div>';
        return;
    }
    
    listEl.innerHTML = plugins.map((plugin, index) => `
        <div class="plugin-card ${plugin.installed ? 'installed' : ''}" data-index="${index}" data-plugin-id="${plugin.id}" data-installed="${plugin.installed}">
            <h3>${plugin.name}</h3>
            <div class="version">v${plugin.version}</div>
            <div class="description">${plugin.description}</div>
            <div class="button-group">
                ${plugin.installed ? `
                    <span class="installed-badge">Installed</span>
                ` : `
                    <button class="install">Install</button>
                `}
            </div>
        </div>
    `).join('');
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
    state.searchQuery = e.target.value.toLowerCase();
    const filtered = getFilteredPlugins();
    state.selectedIndex = Math.min(state.selectedIndex, Math.max(0, filtered.length - 1));
    renderPlugins(filtered);
    updateSelection();
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

export function handleKey(e) {
    if (e.key === 'Enter') {
        e.preventDefault();
        const selected = document.querySelector('.plugin-card.selected');
        if (selected) {
            const pluginId = selected.dataset.pluginId;
            const isInstalled = selected.dataset.installed === 'true';
            if (!isInstalled) {
                installPlugin(pluginId);
            }
        }
        return;
    }
    
    const handlers = {
        ArrowUp: () => navigate(-1),
        ArrowDown: () => navigate(1),
        ArrowLeft: () => navigate(-1),
        ArrowRight: () => navigate(1)
    };
    
    const handler = handlers[e.key];
    if (handler) {
        e.preventDefault();
        handler();
    }
}

function navigate(delta) {
    const cards = document.querySelectorAll('.plugin-card');
    const total = cards.length;
    if (total === 0) {
        state.selectedIndex = 0;
        return;
    }
    
    const newIndex = Math.max(0, Math.min(total - 1, state.selectedIndex + delta));
    
    if (newIndex !== state.selectedIndex) {
        state.selectedIndex = newIndex;
        updateSelection();
    }
}

function getFilteredPlugins() {
    if (!state.searchQuery) return state.plugins;
    return state.plugins.filter(p =>
        p.name.toLowerCase().includes(state.searchQuery) ||
        p.description.toLowerCase().includes(state.searchQuery)
    );
}

async function installPlugin(id) {
    try {
        const response = await fetch(`/api/install/${id}`, { method: 'POST' });
        if (!response.ok) throw new Error('Installation failed');
        
        const plugin = state.plugins.find(p => p.id === id);
        if (plugin) {
            plugin.installed = true;
            renderPlugins(getFilteredPlugins());
            updateSelection();
        }
    } catch (error) {
        console.error(`Failed to install plugin: ${error.message}`);
    }
}


export function onFocus() {
    updateSelection();
    if (searchInput) {
        searchInput.focus();
    }
}

export function onBlur() {
    if (searchInput) {
        searchInput.blur();
    }
}

export function cleanup() {
    const listEl = document.getElementById('store-list');
    if (listEl) {
        listEl.removeEventListener('click', handleListClick);
    }
    if (searchInput) {
        searchInput.removeEventListener('input', handleSearch);
    }
}

