import { useRef, useMemo, useCallback } from 'preact/hooks';
import { usePersistedId } from '../../lib/hooks/usePersistedIndex.js';
import { useGridNav } from '../../lib/hooks/useGridNav.js';
import { getFilteredPlugins, resolveSelectedIndex } from './reducer.js';

export function useStoreNav(plugins, searchQuery) {
    const [selectedId, setSelectedId, selectedIdRef, markRestored] = usePersistedId('store-selected-id');
    const filtered = useMemo(() => getFilteredPlugins(plugins, searchQuery), [plugins, searchQuery]);
    const filteredRef = useRef(filtered);
    filteredRef.current = filtered;
    const lastIndexRef = useRef(0);
    const selectedIndex = resolveSelectedIndex(filtered, selectedId, lastIndexRef.current);
    lastIndexRef.current = selectedIndex;
    const selectedIndexRef = useRef(selectedIndex);
    selectedIndexRef.current = selectedIndex;
    const setSelectedIndex = useCallback((indexOrFn) => {
        const idx = typeof indexOrFn === 'function' ? indexOrFn(selectedIndexRef.current) : indexOrFn;
        const plugin = filteredRef.current[idx];
        markRestored();
        if (plugin) setSelectedId(plugin.id);
    }, [markRestored, setSelectedId]);
    const navigateInGrid = useGridNav('#store-list .plugin-card', selectedIndexRef, setSelectedIndex);
    return { selectedIndex, setSelectedIndex, selectedIndexRef, filtered, filteredRef, navigateInGrid };
}
