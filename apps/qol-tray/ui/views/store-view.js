import { html } from '../lib/html.js';
import { useEffect, useCallback, useRef, useMemo } from 'preact/hooks';
import { useStateRef } from '../hooks/useStateRef.js';
import { usePersistedIndex } from '../hooks/usePersistedIndex.js';
import { useScrollIntoView } from '../hooks/useScrollIntoView.js';
import { useRefreshOnFocus } from '../hooks/useRefreshOnFocus.js';
import { useSSEDebounce } from '../hooks/useSSEDebounce.js';
import { useInstalling } from '../hooks/useInstalling.js';
import { useFeedback } from '../hooks/useFeedback.js';
import { useGridNav } from '../hooks/useGridNav.js';
import { useAsyncToken } from '../hooks/useAsyncToken.js';
import { withShiftVariants, dispatchKey } from '../utils/keys.js';
import { Feedback } from '../components/FeedbackPreact.js';
import { PageHeader } from '../components/PageHeader.js';
import {
    formatCacheAge,
    normalizeSearchQuery,
    getFilteredPlugins,
    clampSelectedIndex,
    looksLikeGithubAuthFailure
} from './store/reducer.js';
import {
    loadStorePlugins,
    loadStoreTokenState,
    saveStoreToken,
    deleteStoreToken,
    installStorePlugin
} from './store/data.js';
import { StoreTokenPanel } from './store/token-panel.js';
import { StoreGrid } from './store/grid.js';
import { useFooterShortcuts } from '../hooks/useFooterShortcuts.js';

const SHORTCUTS = [
    { key: '←↑↓→', label: 'navigate' },
    { key: 'Enter', label: 'install' },
    { key: 's', label: 'search' },
    { key: 't', label: 'token' },
    { key: '⌘R', label: 'refresh' }
];

export function StoreView() {
    const [plugins, setPlugins, pluginsRef] = useStateRef([]);
    const [selectedIndex, setSelectedIndex, selectedIndexRef, storeMarkRestored] = usePersistedIndex('store-selected-index');
    const [searchQuery, setSearchQuery] = useStateRef('');
    const [hasToken, setHasToken, hasTokenRef] = useStateRef(false);
    const [showTokenInput, setShowTokenInput, showTokenInputRef] = useStateRef(false);
    const [rateLimited, setRateLimited] = useStateRef(false);
    const [cacheAgeSecs, setCacheAgeSecs] = useStateRef(null);
    const [loading, setLoading, loadingRef] = useStateRef(false);
    const { feedback, setFeedback, clearFeedback } = useFeedback();
    const { has: isInstalling, add: addInstalling, remove: removeInstalling } = useInstalling();

    const [nextToken, isCurrentToken] = useAsyncToken();
    const searchRef = useRef(null);
    const tokenInputRef = useRef(null);

    useFooterShortcuts(SHORTCUTS);

    const filtered = useMemo(() => getFilteredPlugins(plugins, searchQuery), [plugins, searchQuery]);
    const filteredRef = useRef(filtered);
    filteredRef.current = filtered;

    const focusTokenInput = useCallback(() => {
        setTimeout(() => tokenInputRef.current?.focus(), 0);
    }, []);

    const openTokenInput = useCallback(() => {
        setShowTokenInput(true);
        focusTokenInput();
    }, [focusTokenInput]);

    const closeTokenInput = useCallback(() => {
        setShowTokenInput(false);
    }, []);

    const loadPlugins = useCallback(async (options = {}) => {
        const { forceRefresh = false, hasToken = hasTokenRef.current } = options;
        const token = nextToken();
        setLoading(true);
        try {
            const data = await loadStorePlugins({ forceRefresh, hasToken });
            if (!isCurrentToken(token)) return;
            setPlugins(data.plugins);
            setCacheAgeSecs(data.cacheAgeSecs);
            setRateLimited(data.rateLimited);
            if (!data.rateLimited) {
                setShowTokenInput(false);
            }
        } catch (error) {
            if (!isCurrentToken(token)) return;
            if (looksLikeGithubAuthFailure(error?.message)) {
                setRateLimited(true);
                openTokenInput();
            }
            setFeedback('error', `Failed to load plugins: ${error.message}`);
        } finally {
            if (isCurrentToken(token)) setLoading(false);
        }
    }, [nextToken, isCurrentToken, openTokenInput, setFeedback]);

    useEffect(() => {
        (async () => {
            const tokenState = await loadStoreTokenState();
            setHasToken(tokenState);
            loadPlugins({ hasToken: tokenState });
        })();
    }, [loadPlugins]);

    useRefreshOnFocus(loadPlugins);

    useSSEDebounce('plugins_changed', () => loadPlugins());

    useEffect(() => {
        setSelectedIndex(prev => {
            storeMarkRestored();
            return clampSelectedIndex(prev, filtered.length);
        });
    }, [filtered.length, setSelectedIndex, storeMarkRestored]);

    useScrollIntoView('#store-list .plugin-card.selected', [selectedIndex]);

    const refreshPlugins = useCallback(() => {
        if (loadingRef.current) {
            return;
        }

        loadPlugins({ forceRefresh: true });
    }, [loadPlugins]);

    const saveToken = useCallback(async () => {
        const input = tokenInputRef.current;
        const tokenValue = input?.value?.trim();
        if (!tokenValue) {
            setFeedback('error', 'Token cannot be empty');
            return;
        }

        clearFeedback();
        try {
            await saveStoreToken(tokenValue);
            setHasToken(true);
            setShowTokenInput(false);
            setRateLimited(false);
            setFeedback('success', 'GitHub token saved');
            loadPlugins({ hasToken: true });
        } catch (error) {
            setFeedback('error', `Failed to save token: ${error.message}`);
            input?.focus();
            input?.select();
        }
    }, [clearFeedback, setFeedback, loadPlugins]);

    const deleteToken = useCallback(async () => {
        clearFeedback();
        try {
            await deleteStoreToken();
            setHasToken(false);
            setShowTokenInput(false);
            setFeedback('success', 'GitHub token removed');
        } catch (error) {
            setFeedback('error', `Failed to delete token: ${error.message}`);
        }
    }, [clearFeedback, setFeedback]);

    const installPlugin = useCallback(async (id) => {
        if (isInstalling(id)) {
            return;
        }

        const plugin = pluginsRef.current.find(p => p.id === id);
        clearFeedback();
        addInstalling(id, plugin?.name || id);
        try {
            await installStorePlugin(id);
            setFeedback('success', `Installed ${plugin?.name || id}`);
        } catch (error) {
            setFeedback('error', `Failed to install ${plugin?.name || id}: ${error.message}`);
        } finally {
            removeInstalling(id);
            loadPlugins();
        }
    }, [isInstalling, pluginsRef, clearFeedback, addInstalling, setFeedback, removeInstalling, loadPlugins]);

    const navigateInGrid = useGridNav('#store-list .plugin-card', selectedIndexRef, setSelectedIndex);

    const handleKey = useCallback((e) => {
        const inSearch = document.activeElement === searchRef.current;
        if (inSearch) {
            if (e.key === 'Escape') {
                e.preventDefault();
                searchRef.current.blur();
                return;
            }

            if ((e.ctrlKey || e.metaKey) && e.key === 'r') {
                e.preventDefault();
                refreshPlugins();
            }

            return;
        }

        if (showTokenInputRef.current && e.key === 'Escape') {
            e.preventDefault();
            closeTokenInput();
            return;
        }

        if ((e.ctrlKey || e.metaKey) && e.key === 'r') {
            e.preventDefault();
            refreshPlugins();
            return;
        }

        dispatchKey(e, withShiftVariants({
            ArrowUp: () => navigateInGrid('up'),
            ArrowDown: () => navigateInGrid('down'),
            ArrowLeft: () => navigateInGrid('left'),
            ArrowRight: () => navigateInGrid('right'),
            Enter: () => {
                const plugin = filteredRef.current[selectedIndexRef.current];
                if (plugin && !plugin.installed && !isInstalling(plugin.id)) {
                    installPlugin(plugin.id);
                }
            },
            s: () => searchRef.current?.focus(),
            t: openTokenInput
        }));
    }, [refreshPlugins, closeTokenInput, navigateInGrid, isInstalling, installPlugin, openTokenInput]);

    StoreView.handleKey = handleKey;
    StoreView.isBlocking = () => false;

    const handleSearch = useCallback((e) => {
        setSearchQuery(normalizeSearchQuery(e.target.value));
    }, [setSearchQuery]);

    const handleCardClick = useCallback((event, index, pluginId) => {
        if (event.target.closest('button.install')) {
            installPlugin(pluginId);
            return;
        }

        if (index !== selectedIndexRef.current) {
            setSelectedIndex(index);
        }
    }, [installPlugin, setSelectedIndex, selectedIndexRef]);

    return html`
        <div class="view-container">
            <${PageHeader}
                title="Plugin Store"
                subtitle="Browse and install plugins for QoL Tray"
                actions=${html`
                    <span class="cache-age">${formatCacheAge(cacheAgeSecs)}</span>
                    <button class="btn btn-ghost btn-sm" title="Manage GitHub token"
                            onClick=${openTokenInput}>
                        Token
                    </button>
                    <button class="refresh-btn ${loading ? 'spinning' : ''}" title="Refresh (r)"
                            aria-label="Refresh" disabled=${loading} onClick=${refreshPlugins}></button>
                `}
            />
            <div class="view-body">
                <div class="search-bar store-search-bar">
                    <input type="text" ref=${searchRef} placeholder="Search plugins..."
                           value=${searchQuery} onInput=${handleSearch} />
                </div>
                <${StoreTokenPanel}
                    showTokenInput=${showTokenInput}
                    hasToken=${hasToken}
                    rateLimited=${rateLimited}
                    tokenInputRef=${tokenInputRef}
                    onSave=${saveToken}
                    onDelete=${deleteToken}
                    onCancel=${closeTokenInput}
                    onShow=${openTokenInput}
                />
                <${Feedback} feedback=${feedback} />
                <${StoreGrid}
                    plugins=${filtered}
                    loading=${loading}
                    selectedIndex=${selectedIndex}
                    isInstalling=${isInstalling}
                    onCardClick=${handleCardClick}
                />
            </div>
        </div>
    `;
}
