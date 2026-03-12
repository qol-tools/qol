import { useRef, useEffect, useCallback } from 'preact/hooks';
import { useStateRef } from '../../hooks/useStateRef.js';
import { usePersistedIndex } from '../../hooks/usePersistedIndex.js';
import { useAsyncToken } from '../../hooks/useAsyncToken.js';
import { useRefreshOnFocus } from '../../hooks/useRefreshOnFocus.js';
import { useSSEDebounce } from '../../hooks/useSSEDebounce.js';
import { useInstalling } from '../../hooks/useInstalling.js';
import { loadInstalledPlugins, buildGhostPlugins } from './data.js';

async function doRefresh(opts, nextToken, isCurrentToken, latestRevisionRef, applyPayload, setFeedback) {
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
        if (showErrorFeedback) setFeedback('error', `Failed to load plugins: ${error.message}`);
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

export function usePluginsList(setFeedback) {
    const [plugins, setPlugins, pluginsRef] = useStateRef([]);
    const [selectedIndex, setSelectedIndex, selectedIndexRef, markRestored] = usePersistedIndex('plugins-selected-index', 0);
    const { items: installingItems } = useInstalling();
    const [nextToken, isCurrentToken] = useAsyncToken();
    const latestRevisionRef = useRef(0);
    const applyPayload = useCallback((items, restore) => {
        setPlugins(items);
        setSelectedIndex(prev => {
            if (restore) markRestored();
            if (prev >= items.length) return 0;
            return prev;
        });
    }, []);
    const refreshPlugins = useCallback(
        opts => doRefresh(opts || {}, nextToken, isCurrentToken, latestRevisionRef, applyPayload, setFeedback),
        [setFeedback, applyPayload]
    );
    useListEffects(refreshPlugins, latestRevisionRef);
    const ghostPlugins = buildGhostPlugins(plugins, installingItems);
    return { plugins, pluginsRef, selectedIndex, setSelectedIndex, selectedIndexRef, refreshPlugins, ghostPlugins };
}
