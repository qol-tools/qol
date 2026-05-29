import { clampIndex, sortByName, matchesQuery } from '../../utils/collections.js';

export function createStoreState() {
    return {
        plugins: [],
        selectedIndex: 0,
        searchQuery: '',
        hasToken: false,
        showTokenInput: false,
        rateLimited: false,
        cacheAgeSecs: null,
        loading: false,
        loadToken: 0,
        feedback: null
    };
}

export function formatCacheAge(secs) {
    if (secs === null || secs === undefined) return '';
    if (secs < 60) return 'just now';
    if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
    return `${Math.floor(secs / 3600)}h ago`;
}

export function normalizeSearchQuery(value) {
    return String(value ?? '').toLowerCase();
}

export function getFilteredPlugins(plugins, searchQuery) {
    if (!searchQuery) return plugins;
    return plugins.filter(p => matchesQuery([p?.name, p?.description], searchQuery));
}

export { clampIndex as clampSelectedIndex, sortByName as sortPluginsByName };

export function resolveSelectedIndex(filtered, selectedId, fallbackIndex = 0) {
    if (!Array.isArray(filtered) || filtered.length === 0) return 0;
    if (selectedId) {
        const idx = filtered.findIndex(p => p.id === selectedId);
        if (idx >= 0) return idx;
    }
    return clampIndex(fallbackIndex, filtered.length);
}

export function isRateLimitedWithoutToken(plugins, hasToken) {
    return plugins.length === 0 && !hasToken;
}

export function looksLikeGithubAuthFailure(message) {
    const normalized = String(message || '').toLowerCase();
    return (
        normalized.includes('401') ||
        normalized.includes('403') ||
        normalized.includes('bad credentials') ||
        normalized.includes('requires authentication') ||
        normalized.includes('invalid token')
    );
}

export function isStoreUpdateAvailable(plugin) {
    return Boolean(plugin?.update_available) && !isStoreDevLinked(plugin);
}

export function isStoreDevLinked(plugin) {
    return plugin?.source === 'dev_linked';
}

export function displayedStoreVersion(plugin) {
    if (plugin?.installed) {
        return plugin.running_version ?? plugin.installed_version ?? plugin.version ?? null;
    }
    return plugin?.version ?? null;
}
