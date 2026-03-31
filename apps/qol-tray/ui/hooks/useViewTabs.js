import { useCallback, useRef, useState } from 'preact/hooks';

/**
 * Reusable tab + zone keyboard system.
 *
 * Manages: activeTab, zone ('tabs' | 'content'), tabCursor.
 * Keyboard: left/right to navigate tabs, Enter/Space to activate,
 *           ESC from content → tabs zone.
 *
 * The returned `handleKey` should be called from the view's keyboard handler.
 * It returns true if it handled the event (so the caller can skip further handling).
 */
export function useViewTabs(tabs, { onActivate } = {}) {
    const [activeTab, setActiveTab] = useState(tabs[0]?.id ?? '');
    const [zone, setZone] = useState('tabs');
    const [tabCursor, setTabCursor] = useState(0);
    const activeTabRef = useRef(activeTab);
    activeTabRef.current = activeTab;

    const activateTab = useCallback((index) => {
        if (index < 0 || index >= tabs.length) return;
        const tabId = tabs[index].id;
        setActiveTab(tabId);
        setZone('content');
        if (onActivate) onActivate(tabId, index);
    }, [tabs, onActivate]);

    const switchTab = useCallback((tabId) => {
        const idx = tabs.findIndex(t => t.id === tabId);
        if (idx >= 0) activateTab(idx);
    }, [tabs, activateTab]);

    const handleKey = useCallback((event) => {
        if (event.key === 'Escape' && zone === 'content') {
            event.preventDefault();
            setZone('tabs');
            const idx = tabs.findIndex(t => t.id === activeTabRef.current);
            setTabCursor(idx >= 0 ? idx : 0);
            focusTab(tabs, idx >= 0 ? idx : 0);
            return true;
        }
        if (zone === 'tabs') {
            if (event.key === 'ArrowLeft' || event.key === 'h') {
                event.preventDefault();
                const next = Math.max(0, tabCursor - 1);
                setTabCursor(next);
                focusTab(tabs, next);
                return true;
            }
            if (event.key === 'ArrowRight' || event.key === 'l') {
                event.preventDefault();
                const next = Math.min(tabs.length - 1, tabCursor + 1);
                setTabCursor(next);
                focusTab(tabs, next);
                return true;
            }
            if (event.key === 'Enter' || event.key === ' ') {
                event.preventDefault();
                activateTab(tabCursor);
                return true;
            }
            return true;
        }
        return false;
    }, [zone, tabCursor, tabs, activateTab]);

    return {
        activeTab,
        zone,
        tabCursor,
        handleKey,
        activateTab,
        switchTab,
        setZone,
    };
}

function focusTab(tabs, index) {
    if (!tabs[index]) return;
    const el = document.querySelector(`[role="tablist"] [role="tab"][data-tab-id="${tabs[index].id}"]`);
    if (el) el.focus();
}
