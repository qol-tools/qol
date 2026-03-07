import { useEffect, useRef, useMemo } from 'preact/hooks';
import { usePersistedIndex } from '../../hooks/usePersistedIndex.js';
import { useGridNav } from '../../hooks/useGridNav.js';
import { getFilteredPlugins, clampSelectedIndex } from './reducer.js';

export function useStoreNav(plugins, searchQuery) {
    const [selectedIndex, setSelectedIndex, selectedIndexRef, markRestored] = usePersistedIndex('store-selected-index');
    const filtered = useMemo(() => getFilteredPlugins(plugins, searchQuery), [plugins, searchQuery]);
    const filteredRef = useRef(filtered);
    filteredRef.current = filtered;
    useEffect(() => {
        setSelectedIndex(prev => { markRestored(); return clampSelectedIndex(prev, filtered.length); });
    }, [filtered.length, setSelectedIndex, markRestored]);
    const navigateInGrid = useGridNav('#store-list .plugin-card', selectedIndexRef, setSelectedIndex);
    return { selectedIndex, setSelectedIndex, selectedIndexRef, filtered, filteredRef, navigateInGrid };
}
