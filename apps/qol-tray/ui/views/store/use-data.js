import { useCallback, useRef } from 'preact/hooks';
import { useStateRef } from '../../lib/hooks/useStateRef.js';
import { useAsyncToken } from '../../lib/hooks/useAsyncToken.js';
import { useRefreshOnFocus } from '../../lib/hooks/useRefreshOnFocus.js';
import { useSSEDebounce } from '../../hooks/useSSEDebounce.js';
import { loadStorePlugins } from './data.js';
import { samePluginList } from '../../utils/plugins.js';

const FOCUS_REFRESH_MIN_MS = 30000;
const REVALIDATE_MAX_MS = 25000;

export function useStoreData(hasTokenRef, onLoadResult) {
    const [plugins, setPlugins, pluginsRef] = useStateRef([]);
    const [cacheAgeSecs, setCacheAgeSecs] = useStateRef(null);
    const [firstLoad, setFirstLoad, firstLoadRef] = useStateRef(true);
    const [refreshing, setRefreshing] = useStateRef(false);
    const [nextToken, isCurrentToken] = useAsyncToken();
    const revalTimerRef = useRef(null);
    const ctx = { hasTokenRef, nextToken, isCurrentToken, setPlugins, pluginsRef, setCacheAgeSecs, setFirstLoad, firstLoadRef, setRefreshing, revalTimerRef, onLoadResult };
    const loadPlugins = useCallback(options => executeLoad(ctx, options || {}), [hasTokenRef, nextToken, isCurrentToken, onLoadResult]);
    const refreshPlugins = useCallback(() => loadPlugins({ forceRefresh: true }), [loadPlugins]);
    useRefreshOnFocus(loadPlugins, { minIntervalMs: FOCUS_REFRESH_MIN_MS });
    useSSEDebounce('plugins_changed', () => loadPlugins());
    return { plugins, pluginsRef, firstLoad, refreshing, cacheAgeSecs, loadPlugins, refreshPlugins };
}

async function executeLoad(ctx, options) {
    const { hasTokenRef, nextToken, isCurrentToken, setFirstLoad, firstLoadRef, onLoadResult } = ctx;
    const { forceRefresh = false, hasToken = hasTokenRef.current } = options;
    const token = nextToken();
    try {
        const data = await loadStorePlugins({ forceRefresh, hasToken });
        if (!isCurrentToken(token)) return;
        applyStorePayload(ctx, data);
        markRevalidating(ctx, Boolean(data.revalidating));
        onLoadResult.current?.({ data });
    } catch (error) {
        if (!isCurrentToken(token)) return;
        markRevalidating(ctx, false);
        onLoadResult.current?.({ error });
    } finally {
        if (isCurrentToken(token) && firstLoadRef.current) setFirstLoad(false);
    }
}

function applyStorePayload({ setPlugins, pluginsRef, setCacheAgeSecs }, data) {
    setCacheAgeSecs(data.cacheAgeSecs);
    const next = data.plugins;
    if (next.length === 0 && pluginsRef.current.length > 0) return;
    if (samePluginList(pluginsRef.current, next)) return;
    setPlugins(next);
}

function markRevalidating({ setRefreshing, revalTimerRef }, on) {
    if (revalTimerRef.current) {
        clearTimeout(revalTimerRef.current);
        revalTimerRef.current = null;
    }
    setRefreshing(on);
    if (on) {
        revalTimerRef.current = setTimeout(() => {
            revalTimerRef.current = null;
            setRefreshing(false);
        }, REVALIDATE_MAX_MS);
    }
}
