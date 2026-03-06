import { useEffect, useCallback, useRef, useMemo } from 'preact/hooks';
import { useStateRef } from '../../hooks/useStateRef.js';
import { usePersistedIndex } from '../../hooks/usePersistedIndex.js';
import { useScrollIntoView } from '../../hooks/useScrollIntoView.js';
import { useGridNav } from '../../hooks/useGridNav.js';
import { getFilteredPlugins, clampSelectedIndex, normalizeSearchQuery } from './reducer.js';

export function useStoreNav(plugins) {
    const [selectedIndex, setSelectedIndex, selectedIndexRef, markRestored] = usePersistedIndex('store-selected-index');
    const [searchQuery, setSearchQuery] = useStateRef('');
    const filtered = useMemo(() => getFilteredPlugins(plugins, searchQuery), [plugins, searchQuery]);
    const filteredRef = useRef(filtered);
    filteredRef.current = filtered;
    useEffect(() => {
        setSelectedIndex(prev => { markRestored(); return clampSelectedIndex(prev, filtered.length); });
    }, [filtered.length, setSelectedIndex, markRestored]);
    useScrollIntoView('#store-list .plugin-card.selected', [selectedIndex]);
    const navigateInGrid = useGridNav('#store-list .plugin-card', selectedIndexRef, setSelectedIndex);
    const handleSearch = useCallback(e => setSearchQuery(normalizeSearchQuery(e.target.value)), []);
    return { selectedIndex, setSelectedIndex, selectedIndexRef, searchQuery, filtered, filteredRef, navigateInGrid, handleSearch };
}
