import { useRef, useEffect, useCallback, useMemo, useState } from 'preact/hooks';
import { useStateRef } from '../../lib/hooks/useStateRef.js';
import { usePersistedId } from '../../lib/hooks/usePersistedIndex.js';
import { useAsyncToken } from '../../lib/hooks/useAsyncToken.js';
import { useRefreshOnFocus } from '../../lib/hooks/useRefreshOnFocus.js';
import { useSSEDebounce } from '../../hooks/useSSEDebounce.js';
import { useSSE } from '../../hooks/useSSE.js';
import { useInstalling } from '../../hooks/useInstalling.js';
import { loadInstalledPlugins, loadPushStatuses, loadReadiness, buildGhostPlugins, readInstalledCache, writeInstalledCache } from './data.js';
import { samePluginList, markPluginUpdated } from '../../utils/plugins.js';
import { toast } from '../../lib/toast.js';

const FOCUS_REFRESH_MIN_MS = 30000;

export function isWarmingRuntimeStatus(status) {
    return status?.state === 'starting'
        && (status.phase === 'starting' || status.phase === 'warming');
}

function mergeReadiness(plugins, readiness) {
    const ids = Object.keys(readiness);
    if (ids.length === 0) return plugins;
    return plugins.map(plugin => readiness[plugin.id]
        ? { ...plugin, runtime_status: readiness[plugin.id] }
        : plugin);
}

function findPluginIndex(plugins, pluginId) {
    if (!pluginId) return 0;
    const idx = plugins.findIndex(p => p.id === pluginId);
    return idx >= 0 ? idx : 0;
}

async function doRefresh(opts, ctx) {
    const { showErrorFeedback = false, restoreSelection = false, minRevision = 0 } = opts;
    const { nextToken, isCurrentToken, latestRevisionRef, applyPayload, setPushStatuses, setReadiness } = ctx;
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
    try {
        const statuses = await loadPushStatuses();
        if (!isCurrentToken(token)) return;
        setPushStatuses(statuses && typeof statuses === 'object' ? statuses : {});
    } catch {
        // Pushed status is best-effort; the plugin list itself is the payload.
    }
    try {
        const warming = await loadReadiness();
        if (!isCurrentToken(token)) return;
        setReadiness(Object.fromEntries(Object.entries(warming || {})
            .filter(([, status]) => isWarmingRuntimeStatus(status))));
    } catch {
        // Readiness is best-effort; a plugin without it simply shows no chip.
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
    const [readiness, setReadiness] = useState({});
    const [loaded, setLoaded, loadedRef] = useStateRef(initialCache != null);
    const [pushStatuses, setPushStatuses] = useState({});
    const [selectedPluginId, setSelectedPluginId, selectedPluginIdRef, markRestored] = usePersistedId('plugins-selected-id');
    const selectedIndexRef = useRef(0);
    const { items: installingItems } = useInstalling();
    const [nextToken, isCurrentToken] = useAsyncToken();
    const latestRevisionRef = useRef(0);
    const applyPayload = useCallback((revision, items, restore) => {
        if (!samePluginList(pluginsRef.current, items)) setPlugins(items);
        writeInstalledCache(revision, items);
        if (!loadedRef.current) setLoaded(true);
        if (restore) markRestored();
    }, [markRestored]);
    const refreshPlugins = useCallback(
        opts => doRefresh(opts || {}, { nextToken, isCurrentToken, latestRevisionRef, applyPayload, setPushStatuses, setReadiness }),
        [applyPayload]
    );
    const markUpdated = useCallback((id) => setPlugins(prev => markPluginUpdated(prev, id)), []);
    useListEffects(refreshPlugins, latestRevisionRef);
    useSSE(useCallback(event => {
        if (event.plugin_id == null) return;
        if (event.type === 'readiness_changed') {
            setReadiness(prev => {
                if (isWarmingRuntimeStatus(event.runtime_status)) {
                    return { ...prev, [event.plugin_id]: event.runtime_status };
                }
                if (!(event.plugin_id in prev)) return prev;
                const next = { ...prev };
                delete next[event.plugin_id];
                return next;
            });
            return;
        }
        if (event.type !== 'status_changed') return;
        setPushStatuses(prev => {
            if (event.status == null) {
                if (!(event.plugin_id in prev)) return prev;
                const next = { ...prev };
                delete next[event.plugin_id];
                return next;
            }
            return { ...prev, [event.plugin_id]: event.status };
        });
    }, []));
    const ghostPlugins = buildGhostPlugins(plugins, installingItems);
    const visiblePlugins = useMemo(() => mergeReadiness(plugins, readiness), [plugins, readiness]);
    const selectedIndex = findPluginIndex(plugins, selectedPluginId);
    selectedIndexRef.current = selectedIndex;
    const setSelectedIndex = useCallback((indexOrFn) => {
        const idx = typeof indexOrFn === 'function' ? indexOrFn(selectedIndexRef.current) : indexOrFn;
        const plugin = pluginsRef.current[idx];
        if (plugin) setSelectedPluginId(plugin.id);
    }, []);
    return { plugins: visiblePlugins, pluginsRef, selectedIndex, setSelectedIndex, selectedIndexRef, refreshPlugins, markUpdated, ghostPlugins, loaded, pushStatuses };
}
