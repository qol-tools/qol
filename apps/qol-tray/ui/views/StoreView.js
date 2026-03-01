import { html } from '../lib/html.js';
import { useEffect, useCallback, useRef, useMemo } from 'preact/hooks';
import { useStateRef } from '../hooks/useStateRef.js';
import { useScrollIntoView } from '../hooks/useScrollIntoView.js';
import { useRefreshOnFocus } from '../hooks/useRefreshOnFocus.js';
import { useSSEDebounce } from '../hooks/useSSEDebounce.js';
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
import { useFooterShortcuts } from '../hooks/useFooterShortcuts.js';
import * as installing from '../installing.js';

const SHORTCUTS = [
    { key: '←↑↓→', label: 'navigate' },
    { key: 'Enter', label: 'install' },
    { key: 's', label: 'search' },
    { key: 't', label: 'token' },
    { key: '⌘R', label: 'refresh' }
];

export function StoreView() {
    const [plugins, setPlugins, pluginsRef] = useStateRef([]);
    const [selectedIndex, setSelectedIndex, selectedIndexRef] = useStateRef(() => {
        const saved = parseInt(localStorage.getItem('store-selected-index') || '0', 10);
        return saved >= 0 ? saved : 0;
    });
    const storeRestoredRef = useRef(false);
    const [searchQuery, setSearchQuery] = useStateRef('');
    const [hasToken, setHasToken, hasTokenRef] = useStateRef(false);
    const [showTokenInput, setShowTokenInput, showTokenInputRef] = useStateRef(false);
    const [rateLimited, setRateLimited] = useStateRef(false);
    const [cacheAgeSecs, setCacheAgeSecs] = useStateRef(null);
    const [loading, setLoading, loadingRef] = useStateRef(false);
    const { feedback, setFeedback, clearFeedback } = useFeedback();
    const { has: isInstalling, add: addInstalling, remove: removeInstalling } = useInstalling();

    const loadTokenRef = useRef(0);
    const searchRef = useRef(null);

    useFooterShortcuts(SHORTCUTS);

    // Derived — memoized to avoid new array ref every render
    const filtered = useMemo(() => getFilteredPlugins(plugins, searchQuery), [plugins, searchQuery]);
    const filteredRef = useRef(filtered);
    filteredRef.current = filtered;

    // Load plugins — uses ref for hasToken to stay stable
    const loadPlugins = useCallback(async (forceRefresh = false) => {
        const token = ++loadTokenRef.current;
        setLoading(true);
        try {
            const data = await fetchPluginsRequest(forceRefresh);
            if (token !== loadTokenRef.current) return;
            const sorted = sortPluginsByName(data.plugins || []);
            setPlugins(sorted);
            setCacheAgeSecs(data.cache_age_secs ?? null);
            const rl = isRateLimitedWithoutToken(sorted, hasTokenRef.current);
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
    }, [setFeedback]);

    // Init
    useEffect(() => {
        (async () => {
            const tok = await fetchTokenStatus();
            setHasToken(tok);
            loadPlugins();
        })();
    }, []);

    useRefreshOnFocus(loadPlugins);

    useSSEDebounce('plugins_changed', loadPlugins);

    // Clamp selection when filtered list changes
    useEffect(() => {
        setSelectedIndex(prev => {
            storeRestoredRef.current = true;
            return clampSelectedIndex(prev, filtered.length);
        });
    }, [filtered.length]);

    // Save selection
    useEffect(() => {
        if (!storeRestoredRef.current) return;
        localStorage.setItem('store-selected-index', String(selectedIndex));
    }, [selectedIndex]);

    useScrollIntoView('#store-list .plugin-card.selected', [selectedIndex]);

    // Refresh — stable via ref
    const refreshPlugins = useCallback(() => {
        if (!loadingRef.current) loadPlugins(true);
    }, [loadPlugins]);

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

    // Install — stable via ref for plugins
    const installPlugin = useCallback(async (id) => {
        if (installing.has(id)) return;
        const plugin = pluginsRef.current.find(p => p.id === id);
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
    }, [clearFeedback, setFeedback, addInstalling, removeInstalling, loadPlugins]);

    // Grid navigation — stable via ref
    const navigateInGrid = useCallback((direction) => {
        const current = selectedIndexRef.current;
        const next = navigateGrid('#store-list .plugin-card', current, direction);
        if (next !== current) setSelectedIndex(next);
    }, []);

    // Keyboard — stable: reads mutable state via refs
    const handleKey = useCallback((e) => {
        const inSearch = document.activeElement === searchRef.current;
        if (inSearch) {
            if (e.key === 'Escape') { e.preventDefault(); searchRef.current.blur(); return; }
            if ((e.ctrlKey || e.metaKey) && e.key === 'r') { e.preventDefault(); refreshPlugins(); return; }
            return;
        }
        if (showTokenInputRef.current && e.key === 'Escape') {
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
                const plugin = filteredRef.current[selectedIndexRef.current];
                if (plugin && !plugin.installed && !installing.has(plugin.id)) installPlugin(plugin.id);
            },
            s: () => searchRef.current?.focus(),
            t: () => { setShowTokenInput(true); setTimeout(() => document.getElementById('github-token-input')?.focus(), 0); },
            T: () => { setShowTokenInput(true); setTimeout(() => document.getElementById('github-token-input')?.focus(), 0); },
        };
        const handler = handlers[e.key];
        if (handler) { e.preventDefault(); handler(); }
    }, [refreshPlugins, navigateInGrid, installPlugin]);

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
