import { createContext } from 'preact';
import { useContext, useState, useCallback, useMemo, useRef } from 'preact/hooks';

const SidebarContext = createContext(null);

export function useSidebarContext() {
    return useContext(SidebarContext);
}

export function useSidebarProvider({ defaultItems, defaultHeader }) {
    const [override, setOverride] = useState(null);
    const [header, setHeaderState] = useState(null);
    const genRef = useRef(0);

    const setItems = useCallback((items) => {
        genRef.current += 1;
        setOverride(items);
        return genRef.current;
    }, []);
    const setHeader = useCallback((h) => setHeaderState(h), []);
    const resetSidebar = useCallback((token) => {
        if (token != null && token !== genRef.current) return;
        setOverride(null);
        setHeaderState(null);
    }, []);

    const items = override || defaultItems;
    const isOverridden = override !== null;

    const cycleItem = useCallback((direction) => {
        const clickable = items.filter(i => i.type !== 'divider' && i.onClick);
        if (clickable.length === 0) return;
        const activeIdx = clickable.findIndex(i => i.active);
        if (activeIdx < 0) { clickable[0].onClick(); return; }
        const next = direction > 0
            ? (activeIdx + 1) % clickable.length
            : (activeIdx - 1 + clickable.length) % clickable.length;
        clickable[next].onClick();
    }, [items]);

    const value = useMemo(() => ({
        items,
        setItems,
        header: header !== null ? header : defaultHeader,
        setHeader,
        resetSidebar,
        isOverridden,
        cycleItem,
    }), [items, header, defaultHeader, defaultItems, setItems, setHeader, resetSidebar, isOverridden, cycleItem]);

    return { SidebarContext, value };
}
