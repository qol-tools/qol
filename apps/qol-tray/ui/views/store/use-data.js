import { useCallback } from 'preact/hooks';
import { useStateRef } from '../../hooks/useStateRef.js';
import { useAsyncToken } from '../../hooks/useAsyncToken.js';
import { useRefreshOnFocus } from '../../hooks/useRefreshOnFocus.js';
import { useSSEDebounce } from '../../hooks/useSSEDebounce.js';
import { loadStorePlugins } from './data.js';

export function useStoreData(hasTokenRef, onLoadResult) {
    const [plugins, setPlugins, pluginsRef] = useStateRef([]);
    const [cacheAgeSecs, setCacheAgeSecs] = useStateRef(null);
    const [loading, setLoading, loadingRef] = useStateRef(false);
    const [nextToken, isCurrentToken] = useAsyncToken();
    const loadPlugins = useCallback(async (options = {}) => {
        await executeLoad(hasTokenRef, nextToken, isCurrentToken, setPlugins, setCacheAgeSecs, setLoading, onLoadResult, options);
    }, [hasTokenRef, nextToken, isCurrentToken, onLoadResult]);
    const refreshPlugins = useCallback(() => { if (!loadingRef.current) loadPlugins({ forceRefresh: true }); }, [loadPlugins, loadingRef]);
    useRefreshOnFocus(loadPlugins);
    useSSEDebounce('plugins_changed', () => loadPlugins());
    return { plugins, pluginsRef, loading, cacheAgeSecs, loadPlugins, refreshPlugins };
}

async function executeLoad(hasTokenRef, nextToken, isCurrentToken, setPlugins, setCacheAgeSecs, setLoading, onLoadResult, options) {
    const { forceRefresh = false, hasToken = hasTokenRef.current } = options;
    const token = nextToken();
    setLoading(true);
    try {
        const data = await loadStorePlugins({ forceRefresh, hasToken });
        if (!isCurrentToken(token)) return;
        setPlugins(data.plugins);
        setCacheAgeSecs(data.cacheAgeSecs);
        onLoadResult.current?.({ data });
    } catch (error) {
        if (!isCurrentToken(token)) return;
        onLoadResult.current?.({ error });
    } finally {
        if (isCurrentToken(token)) setLoading(false);
    }
}
