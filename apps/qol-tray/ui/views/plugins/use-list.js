import { useRef, useEffect, useCallback } from 'preact/hooks';
import { useStateRef } from '../../lib/hooks/useStateRef.js';
import { usePersistedId } from '../../lib/hooks/usePersistedIndex.js';
import { useAsyncToken } from '../../lib/hooks/useAsyncToken.js';
import { useRefreshOnFocus } from '../../lib/hooks/useRefreshOnFocus.js';
import { useSSEDebounce } from '../../hooks/useSSEDebounce.js';
import { useInstalling } from '../../hooks/useInstalling.js';
import { loadInstalledPlugins, buildGhostPlugins } from './data.js';
import { toast } from '../../lib/toast.js';

function findPluginIndex(plugins, pluginId) {
    if (!pluginId) return 0;
    const idx = plugins.findIndex(p => p.id === pluginId);
    return idx >= 0 ? idx : 0;
}

async function doRefresh(opts, nextToken, isCurrentToken, latestRevisionRef, applyPayload) {
    const { showErrorFeedback = false, restoreSelection = false, minRevision = 0 } = opts;
    const token = nextToken();
    try {
        const payload = await loadInstalledPlugins();
        if (!isCurrentToken(token)) return;
        if (payload.revision < minRevision || payload.revision < latestRevisionRef.current) return;
        latestRevisionRef.current = payload.revision;
        applyPayload(payload.plugins, restoreSelection);
    } catch (error) {
        if (!isCurrentToken(token)) return;
        if (showErrorFeedback) toast('error', `Failed to load plugins: ${error.message}`);
    }
}

function useListEffects(refreshPlugins, latestRevisionRef) {
    useEffect(() => { refreshPlugins({ showErrorFeedback: true, restoreSelection: true }); }, [refreshPlugins]);
    useRefreshOnFocus(refreshPlugins);
    useSSEDebounce('plugins_changed', useCallback(e => {
        const rev = Number.isInteger(e.revision) ? e.revision : latestRevisionRef.current;
        latestRevisionRef.current = Math.max(latestRevisionRef.current, rev);
        refreshPlugins({ minRevision: rev });
    }, [refreshPlugins]));
}

export function usePluginsList() {
    const [plugins, setPlugins, pluginsRef] = useStateRef([]);
    const [selectedPluginId, setSelectedPluginId, selectedPluginIdRef, markRestored] = usePersistedId('plugins-selected-id');
    const selectedIndexRef = useRef(0);
    const { items: installingItems } = useInstalling();
    const [nextToken, isCurrentToken] = useAsyncToken();
    const latestRevisionRef = useRef(0);
    const applyPayload = useCallback((items, restore) => {
        setPlugins(items);
        if (restore) markRestored();
    }, []);
    const refreshPlugins = useCallback(
        opts => doRefresh(opts || {}, nextToken, isCurrentToken, latestRevisionRef, applyPayload),
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
    return { plugins, pluginsRef, selectedIndex, setSelectedIndex, selectedIndexRef, refreshPlugins, ghostPlugins };
}
