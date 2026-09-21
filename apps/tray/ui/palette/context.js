import { createContext } from 'preact';
import { useState, useCallback, useMemo, useContext } from 'preact/hooks';
import { html } from '../lib/html.js';

const PaletteContext = createContext();

export function usePaletteContext() {
    return useContext(PaletteContext);
}

export function PaletteProvider({ children }) {
    const [active, setActive] = useState(false);
    const [query, setQuery] = useState('');
    const [committedFilter, setCommittedFilter] = useState('');
    const [activeViewId, setActiveViewId] = useState('plugins');

    const mode = query.startsWith('>') ? 'action' : 'search';
    const liveSearch = mode === 'search' ? query : '';
    const actionQuery = mode === 'action' ? query.slice(1) : '';
    const searchQuery = active && mode === 'search' ? liveSearch : committedFilter;

    const activate = useCallback(() => {
        setActive(true);
        setQuery('>');
    }, []);

    const deactivate = useCallback(() => {
        setActive(false);
        setQuery('');
    }, []);

    const commitFilter = useCallback(() => {
        const text = query.startsWith('>') ? '' : query.trim();
        setCommittedFilter(text);
        setActive(false);
        setQuery('');
    }, [query]);

    const clearFilter = useCallback(() => {
        setCommittedFilter('');
        setActive(false);
        setQuery('');
    }, []);

    const reopenFilter = useCallback(() => {
        setActive(true);
        setQuery(committedFilter);
    }, [committedFilter]);

    const value = useMemo(() => ({
        active, query, mode, searchQuery, actionQuery, activeViewId, committedFilter,
        activate, deactivate, setQuery, setActiveViewId, commitFilter, clearFilter, reopenFilter
    }), [active, query, mode, searchQuery, actionQuery, activeViewId, committedFilter,
        activate, deactivate, commitFilter, clearFilter, reopenFilter]);

    return html`<${PaletteContext.Provider} value=${value}>${children}<//>`;
}
