import { useRef, useEffect, useCallback, useMemo } from 'preact/hooks';
import { useStateRef } from '../../lib/hooks/useStateRef.js';
import { usePersistedId } from '../../lib/hooks/usePersistedIndex.js';
import { useAsyncToken } from '../../lib/hooks/useAsyncToken.js';
import { useRefreshOnFocus } from '../../lib/hooks/useRefreshOnFocus.js';
import { useSSEDebounce } from '../../hooks/useSSEDebounce.js';
import { useInstalling } from '../../hooks/useInstalling.js';
import { loadInstalledPlugins, buildGhostPlugins, readInstalledCache, writeInstalledCache } from './data.js';
import { samePluginList } from '../../utils/plugins.js';
import { toast } from '../../lib/toast.js';

const FOCUS_REFRESH_MIN_MS = 30000;

function findPluginIndex(plugins, pluginId) {
    if (!pluginId) return 0;
    const idx = plugins.findIndex(p => p.id === pluginId);
    return idx >= 0 ? idx : 0;
}

async function doRefresh(opts, ctx) {
    const { showErrorFeedback = false, restoreSelection = false, minRevision = 0 } = opts;
    const { nextToken, isCurrentToken, latestRevisionRef, applyPayload } = ctx;
    const token = nextToken();
    try {
        const payload = await loadInstalledPlugins();
        if (!isCurrentToken(token)) return;
        if (payload.revision < minRevision || payload.revision < latestRevisionRef.current) return;
        latestRevisionRef.current = payload.revision;
        applyPayload(payload.revision, payload.plugins, restoreSelection);
    } catch (error) {
        if (!isCurrentToken(token)) return;
        if (showErrorFeedback) toast('error', `Failed to load plugins: ${error.message}`);
    }
}

function useListEffects(refreshPlugins, latestRevisionRef) {
    useEffect(() => { refreshPlugins({ showErrorFeedback: true, restoreSelection: true }); }, [refreshPlugins]);
    useRefreshOnFocus(refreshPlugins, { minIntervalMs: FOCUS_REFRESH_MIN_MS });
    useSSEDebounce('plugins_changed', useCallback(e => {
        const rev = Number.isInteger(e.revision) ? e.revision : latestRevisionRef.current;
        latestRevisionRef.current = Math.max(latestRevisionRef.current, rev);
        refreshPlugins({ minRevision: rev });
    }, [refreshPlugins]));
}

export function usePluginsList() {
    const initialCache = useMemo(() => readInstalledCache(), []);
    const [plugins, setPlugins, pluginsRef] = useStateRef(initialCache?.plugins ?? []);
    const [loaded, setLoaded, loadedRef] = useStateRef(initialCache != null);
    const [selectedPluginId, setSelectedPluginId, selectedPluginIdRef, markRestored] = usePersistedId('plugins-selected-id');
    const selectedIndexRef = useRef(0);
    const { items: installingItems } = useInstalling();
    const [nextToken, isCurrentToken] = useAsyncToken();
    const latestRevisionRef = useRef(initialCache?.revision ?? 0);
    const applyPayload = useCallback((revision, items, restore) => {
        if (!samePluginList(pluginsRef.current, items)) setPlugins(items);
        writeInstalledCache(revision, items);
        if (!loadedRef.current) setLoaded(true);
        if (restore) markRestored();
    }, [markRestored]);
    const refreshPlugins = useCallback(
        opts => doRefresh(opts || {}, { nextToken, isCurrentToken, latestRevisionRef, applyPayload }),
        [applyPayload]
    );
    useListEffects(refreshPlugins, latestRevisionRef);
    const ghostPlugins = buildGhostPlugins(plugins, installingItems);
    const selectedIndex = findPluginIndex(plugins, selectedPluginId);
    selectedIndexRef.current = selectedIndex;
    const setSelectedIndex = useCallback((indexOrFn) => {
        const idx = typeof indexOrFn === 'function' ? indexOrFn(selectedIndexRef.current) : indexOrFn;
        const plugin = pluginsRef.current[idx];
        if (plugin) setSelectedPluginId(plugin.id);
    }, []);
    return { plugins, pluginsRef, selectedIndex, setSelectedIndex, selectedIndexRef, refreshPlugins, ghostPlugins, loaded };
}
