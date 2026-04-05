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
    const [activeViewId, setActiveViewId] = useState('plugins');

    const mode = 'action';
    const searchQuery = '';
    const actionQuery = query;

    const activate = useCallback(() => {
        setActive(true);
        setQuery('');
    }, []);

    const deactivate = useCallback(() => {
        setActive(false);
        setQuery('');
    }, []);

    const value = useMemo(() => ({
        active, query, mode, searchQuery, actionQuery, activeViewId,
        activate, deactivate, setQuery, setActiveViewId
    }), [active, query, mode, searchQuery, actionQuery, activeViewId, activate, deactivate]);

    return html`<${PaletteContext.Provider} value=${value}>${children}<//>`;
}
