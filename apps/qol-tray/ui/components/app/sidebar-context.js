import { createContext } from 'preact';
import { useContext, useState, useCallback, useMemo } from 'preact/hooks';

const SidebarContext = createContext(null);

export function useSidebarContext() {
    return useContext(SidebarContext);
}

export function useSidebarProvider({ defaultItems, defaultHeader }) {
    const [override, setOverride] = useState(null);
    const [header, setHeaderState] = useState(null);

    const setItems = useCallback((items) => setOverride(items), []);
    const setHeader = useCallback((h) => setHeaderState(h), []);
    const resetSidebar = useCallback(() => {
        setOverride(null);
        setHeaderState(null);
    }, []);

    const items = override || defaultItems;
    const value = useMemo(() => ({
        items,
        setItems,
        header: header !== null ? header : defaultHeader,
        setHeader,
        resetSidebar,
        isOverridden: override !== null,
    }), [items, header, defaultHeader, defaultItems, setItems, setHeader, resetSidebar, override]);

    return { SidebarContext, value };
}
