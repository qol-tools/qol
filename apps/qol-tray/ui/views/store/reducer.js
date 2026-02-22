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
    return plugins.filter(plugin => {
        const name = String(plugin?.name ?? '').toLowerCase();
        const description = String(plugin?.description ?? '').toLowerCase();
        return name.includes(searchQuery) || description.includes(searchQuery);
    });
}

export function clampSelectedIndex(selectedIndex, itemCount) {
    return Math.min(selectedIndex, Math.max(0, itemCount - 1));
}

export function sortPluginsByName(plugins) {
    return [...plugins].sort((a, b) => {
        const left = String(a?.name ?? '');
        const right = String(b?.name ?? '');
        return left.localeCompare(right);
    });
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
    if (!plugin?.installed || !plugin?.installed_version || !plugin?.version) {
        return false;
    }
    return isVersionNewer(plugin.version, plugin.installed_version);
}

export function isVersionNewer(available, installed) {
    const availableParts = parseVersionParts(available);
    const installedParts = parseVersionParts(installed);
    const maxLen = Math.max(availableParts.length, installedParts.length);

    for (let i = 0; i < maxLen; i += 1) {
        const a = availableParts[i] ?? 0;
        const b = installedParts[i] ?? 0;
        if (a > b) return true;
        if (a < b) return false;
    }

    return false;
}

export function parseVersionParts(version) {
    return String(version)
        .trim()
        .replace(/^[vV]+/, '')
        .split('.')
        .map(part => {
            const match = part.match(/\d+/);
            return match ? Number.parseInt(match[0], 10) : null;
        })
        .filter(value => Number.isFinite(value));
}
