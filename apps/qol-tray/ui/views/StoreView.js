import { html } from '../lib/html.js';
import { useState, useEffect, useCallback, useRef } from 'preact/hooks';
import { useSSE } from '../hooks/useSSE.js';
import { useInstalling } from '../hooks/useInstalling.js';
import { useFeedback } from '../hooks/useFeedback.js';
import { navigateGrid } from '../hooks/useGridNav.js';
import { Feedback } from '../components/FeedbackPreact.js';
import {
    formatCacheAge, normalizeSearchQuery, getFilteredPlugins,
    clampSelectedIndex, sortPluginsByName, isRateLimitedWithoutToken,
    looksLikeGithubAuthFailure, isStoreUpdateAvailable
} from './store/reducer.js';
import {
    fetchTokenStatus, saveTokenRequest, deleteTokenRequest,
    fetchPluginsRequest, installPluginRequest
} from './store/effects.js';
import { renderShortcutLegend } from '../components/shortcut-legend.js';
import * as installing from '../installing.js';

const SHORTCUTS = [
    { key: '←↑↓→', label: 'navigate' },
    { key: 'Enter', label: 'install' },
    { key: 's', label: 'search' },
    { key: 't', label: 'token' },
    { key: '⌘R', label: 'refresh' }
];

export function StoreView() {
    const [plugins, setPlugins] = useState([]);
    const [selectedIndex, setSelectedIndex] = useState(0);
    const [searchQuery, setSearchQuery] = useState('');
    const [hasToken, setHasToken] = useState(false);
    const [showTokenInput, setShowTokenInput] = useState(false);
    const [rateLimited, setRateLimited] = useState(false);
    const [cacheAgeSecs, setCacheAgeSecs] = useState(null);
    const [loading, setLoading] = useState(false);
    const { feedback, setFeedback, clearFeedback } = useFeedback();
    const { has: isInstalling, add: addInstalling, remove: removeInstalling } = useInstalling();

    const loadTokenRef = useRef(0);
    const searchRef = useRef(null);

    // Footer shortcuts
    useEffect(() => {
        const el = document.getElementById('content-footer');
        if (el) el.innerHTML = renderShortcutLegend(SHORTCUTS);
        return () => { if (el) el.innerHTML = ''; };
    }, []);

    // Derived
    const filtered = getFilteredPlugins(plugins, searchQuery);

    // Load plugins
    const loadPlugins = useCallback(async (forceRefresh = false) => {
        const token = ++loadTokenRef.current;
        setLoading(true);
        try {
            const data = await fetchPluginsRequest(forceRefresh);
            if (token !== loadTokenRef.current) return;
            const sorted = sortPluginsByName(data.plugins || []);
            setPlugins(sorted);
            setCacheAgeSecs(data.cache_age_secs ?? null);
            const rl = isRateLimitedWithoutToken(sorted, hasToken);
            setRateLimited(rl);
            if (!rl) setShowTokenInput(prev => prev && rl);
        } catch (error) {
            if (token !== loadTokenRef.current) return;
            if (looksLikeGithubAuthFailure(error?.message)) {
                setRateLimited(true);
                setShowTokenInput(true);
            }
            setFeedback('error', `Failed to load plugins: ${error.message}`);
        } finally {
            if (token === loadTokenRef.current) setLoading(false);
        }
    }, [hasToken, setFeedback]);

    // Init
    useEffect(() => {
        (async () => {
            const tok = await fetchTokenStatus();
            setHasToken(tok);
            loadPlugins();
        })();
    }, []);

    // SSE
    useSSE(useCallback((event) => {
        if (event.type === 'plugins_changed') loadPlugins();
    }, [loadPlugins]));

    // Clamp selection when filtered list changes
    useEffect(() => {
        setSelectedIndex(prev => clampSelectedIndex(prev, filtered.length));
    }, [filtered.length]);

    // Scroll selected into view
    useEffect(() => {
        const el = document.querySelector('#store-list .plugin-card.selected');
        if (el) el.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
    }, [selectedIndex, filtered]);

    // Refresh
    const refreshPlugins = useCallback(() => {
        if (!loading) loadPlugins(true);
    }, [loading, loadPlugins]);

    // Token actions
    const saveToken = useCallback(async () => {
        const input = searchRef.current?.closest('.view-container')?.querySelector('#github-token-input');
        const tokenVal = input?.value?.trim();
        if (!tokenVal) { setFeedback('error', 'Token cannot be empty'); return; }
        clearFeedback();
        try {
            await saveTokenRequest(tokenVal);
            setHasToken(true);
            setShowTokenInput(false);
            setRateLimited(false);
            setFeedback('success', 'GitHub token saved');
            loadPlugins();
        } catch (e) {
            setFeedback('error', `Failed to save token: ${e.message}`);
            input?.focus();
            input?.select();
        }
    }, [clearFeedback, setFeedback, loadPlugins]);

    const deleteToken = useCallback(async () => {
        clearFeedback();
        try {
            await deleteTokenRequest();
            setHasToken(false);
            setShowTokenInput(false);
            setFeedback('success', 'GitHub token removed');
        } catch (e) {
            setFeedback('error', `Failed to delete token: ${e.message}`);
        }
    }, [clearFeedback, setFeedback]);

    // Install
    const installPlugin = useCallback(async (id) => {
        if (installing.has(id)) return;
        const plugin = plugins.find(p => p.id === id);
        clearFeedback();
        addInstalling(id, plugin?.name || id);
        try {
            await installPluginRequest(id);
            setFeedback('success', `Installed ${plugin?.name || id}`);
        } catch (error) {
            setFeedback('error', `Failed to install ${plugin?.name || id}: ${error.message}`);
        } finally {
            removeInstalling(id);
            loadPlugins();
        }
    }, [plugins, clearFeedback, setFeedback, addInstalling, removeInstalling, loadPlugins]);

    // Grid navigation
    const navigateInGrid = useCallback((direction) => {
        const next = navigateGrid('#store-list .plugin-card', selectedIndex, direction);
        if (next !== selectedIndex) setSelectedIndex(next);
    }, [selectedIndex]);

    // Keyboard
    const handleKey = useCallback((e) => {
        const inSearch = document.activeElement === searchRef.current;
        if (inSearch) {
            if (e.key === 'Escape') { e.preventDefault(); searchRef.current.blur(); return; }
            if ((e.ctrlKey || e.metaKey) && e.key === 'r') { e.preventDefault(); refreshPlugins(); return; }
            return;
        }
        if (showTokenInput && e.key === 'Escape') {
            e.preventDefault();
            setShowTokenInput(false);
            return;
        }
        if ((e.ctrlKey || e.metaKey) && e.key === 'r') { e.preventDefault(); refreshPlugins(); return; }

        const handlers = {
            ArrowUp: () => navigateInGrid('up'),
            ArrowDown: () => navigateInGrid('down'),
            ArrowLeft: () => navigateInGrid('left'),
            ArrowRight: () => navigateInGrid('right'),
            Enter: () => {
                const plugin = filtered[selectedIndex];
                if (plugin && !plugin.installed && !installing.has(plugin.id)) installPlugin(plugin.id);
            },
            s: () => searchRef.current?.focus(),
            t: () => { setShowTokenInput(true); setTimeout(() => document.getElementById('github-token-input')?.focus(), 0); },
            T: () => { setShowTokenInput(true); setTimeout(() => document.getElementById('github-token-input')?.focus(), 0); },
        };
        const handler = handlers[e.key];
        if (handler) { e.preventDefault(); handler(); }
    }, [showTokenInput, refreshPlugins, navigateInGrid, filtered, selectedIndex, installPlugin]);

    // Expose for App keyboard routing
    StoreView.handleKey = handleKey;
    StoreView.isBlocking = () => false;

    const handleSearch = useCallback((e) => {
        setSearchQuery(normalizeSearchQuery(e.target.value));
    }, []);

    return html`
        <div class="view-container">
            <header>
                <div class="header-row">
                    <div>
                        <h1>Plugin Store</h1>
                        <p>Browse and install plugins for QoL Tray</p>
                    </div>
                    <div class="header-actions">
                        <span class="cache-age">${formatCacheAge(cacheAgeSecs)}</span>
                        <button class="btn btn-ghost btn-sm" title="Manage GitHub token"
                                onClick=${() => { setShowTokenInput(true); setTimeout(() => document.getElementById('github-token-input')?.focus(), 0); }}>
                            Token
                        </button>
                        <button class="refresh-btn ${loading ? 'spinning' : ''}" title="Refresh (r)"
                                aria-label="Refresh" disabled=${loading} onClick=${refreshPlugins}></button>
                    </div>
                </div>
            </header>
            <div class="view-body">
                <div class="search-bar store-search-bar">
                    <input type="text" ref=${searchRef} placeholder="Search plugins..."
                           value=${searchQuery} onInput=${handleSearch} />
                </div>
                ${showTokenInput && html`
                    <div class="token-input-container">
                        <input type="password" id="github-token-input" placeholder="Paste GitHub token (no scopes needed)" />
                        <button class="btn btn-primary" onClick=${saveToken}>Save</button>
                        ${hasToken && html`<button class="btn btn-ghost" onClick=${deleteToken}>Remove Token</button>`}
                        <button class="btn btn-ghost" onClick=${() => setShowTokenInput(false)}>Cancel</button>
                    </div>
                    <p class="token-help">
                        <a href="https://github.com/settings/tokens/new" target="_blank">Create token</a> — no scopes needed, just for rate limits
                    </p>
                `}
                ${!showTokenInput && rateLimited && !hasToken && html`
                    <div class="rate-limit-banner">
                        <span>GitHub API rate limit reached.</span>
                        <button class="btn btn-primary"
                                onClick=${() => { setShowTokenInput(true); setTimeout(() => document.getElementById('github-token-input')?.focus(), 0); }}>
                            Add GitHub Token
                        </button>
                    </div>
                `}
                <${Feedback} feedback=${feedback} />
                <div id="store-list" class="plugins-grid grid-cards grid-cards--zoom">
                    ${loading && filtered.length === 0 && html`<div class="loading">Loading plugins...</div>`}
                    ${!loading && filtered.length === 0 && html`<div class="loading">No plugins found</div>`}
                    ${filtered.map((plugin, index) => {
                        const inst = installing.has(plugin.id);
                        const hasUpdate = isStoreUpdateAvailable(plugin);
                        const versionDisplay = hasUpdate
                            ? `v${plugin.installed_version} → v${plugin.version}`
                            : `v${plugin.version}`;
                        return html`
                            <div key=${plugin.id}
                                 class="plugin-card ${plugin.installed ? 'installed' : ''} ${inst ? 'installing' : ''} ${index === selectedIndex ? 'selected' : ''}"
                                 data-index="${index}" data-plugin-id="${plugin.id}"
                                 onClick=${(e) => {
                                     if (e.target.tagName === 'BUTTON' && e.target.classList.contains('install')) {
                                         installPlugin(plugin.id);
                                         return;
                                     }
                                     if (index !== selectedIndex) setSelectedIndex(index);
                                 }}>
                                <h3>${plugin.name}</h3>
                                <div class="version${hasUpdate ? ' has-update' : ''}">${versionDisplay}</div>
                                <div class="description">${plugin.description}</div>
                                <div class="button-group">
                                    ${plugin.installed
                                        ? html`<span class="installed-badge">${hasUpdate ? 'Update Available' : 'Installed'}</span>`
                                        : inst
                                            ? html`<button class="refresh-btn spinning" disabled></button>`
                                            : html`<button class="btn btn-primary install" style="width: 100%">Install</button>`}
                                </div>
                            </div>
                        `;
                    })}
                </div>
            </div>
        </div>
    `;
}
