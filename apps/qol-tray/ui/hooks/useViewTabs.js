import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { directSurfaces, firstChildContainer } from '../lib/surface-traits.js';

export function useViewTabs(tabs, { onActivate } = {}) {
    const [activeTab, setActiveTab] = useState(tabs[0]?.id ?? '');
    const pendingDescentRef = useRef(false);
    const rootRef = useRef(null);

    const descendToContent = useCallback(() => {
        const root = rootRef.current;
        if (!root) return;
        const container = root.closest('[data-surface-container]');
        if (!container) return;
        const child = firstChildContainer(container);
        if (!child) return;
        const surfaces = directSurfaces(child);
        if (surfaces.length === 0) return;
        surfaces[0].focus({ preventScroll: true });
    }, []);

    const activateTab = useCallback((index) => {
        if (index < 0 || index >= tabs.length) return;
        const tabId = tabs[index].id;
        const changed = tabId !== activeTab;
        setActiveTab(tabId);
        if (onActivate) onActivate(tabId, index);
        if (changed) {
            pendingDescentRef.current = true;
        } else {
            descendToContent();
        }
    }, [tabs, onActivate, activeTab, descendToContent]);

    useEffect(() => {
        if (!pendingDescentRef.current) return;
        pendingDescentRef.current = false;
        descendToContent();
    }, [activeTab, descendToContent]);

    const switchTab = useCallback((tabId) => {
        const idx = tabs.findIndex(t => t.id === tabId);
        if (idx >= 0) activateTab(idx);
    }, [tabs, activateTab]);

    return {
        activeTab,
        activateTab,
        switchTab,
        rootRef,
    };
}
