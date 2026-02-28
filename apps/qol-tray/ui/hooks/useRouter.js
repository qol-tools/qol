import { useState, useEffect, useCallback } from 'preact/hooks';

const VIEW_STORAGE_KEY = 'qoltray.activeView';

function parseViewFromHash() {
    return window.location.hash.replace(/^#/, '').trim() || null;
}

function readStoredView() {
    try { return window.localStorage.getItem(VIEW_STORAGE_KEY); } catch { return null; }
}

function persistView(viewId, { updateHash = true } = {}) {
    try { window.localStorage.setItem(VIEW_STORAGE_KEY, viewId); } catch {}
    if (!updateHash) return;
    const target = `#${viewId}`;
    if (window.location.hash !== target) {
        window.history.replaceState(null, '', target);
    }
}

export function useRouter({ viewOrder }) {
    const canUse = useCallback((id) => Boolean(id) && viewOrder.includes(id), [viewOrder]);

    const [activeViewId, setActiveViewId] = useState(() => {
        const fromHash = parseViewFromHash();
        if (canUse(fromHash)) return fromHash;
        const fromStorage = readStoredView();
        if (canUse(fromStorage)) return fromStorage;
        return 'plugins';
    });
    const [activePluginId, setActivePluginId] = useState(() => {
        const hash = window.location.hash.replace(/^#/, '').trim();
        const match = hash.match(/^plugins\/(.+)$/);
        return match ? match[1] : null;
    });

    const switchView = useCallback((viewId) => {
        if (!viewOrder.includes(viewId)) return;
        setActivePluginId(null);
        setActiveViewId(viewId);
        persistView(viewId);
    }, [viewOrder]);

    const openPluginConfig = useCallback((pluginId) => {
        setActivePluginId(pluginId);
        window.history.replaceState(null, '', `#plugins/${pluginId}`);
    }, []);

    const closePluginConfig = useCallback(() => {
        setActivePluginId(null);
        setActiveViewId(prev => {
            persistView(prev);
            return prev;
        });
    }, []);

    useEffect(() => {
        const handler = () => {
            const raw = window.location.hash.replace(/^#/, '').trim();
            const pluginMatch = raw.match(/^plugins\/(.+)$/);
            if (pluginMatch) {
                setActivePluginId(pluginMatch[1]);
                return;
            }
            setActivePluginId(null);
            const viewId = parseViewFromHash();
            if (viewOrder.includes(viewId)) {
                setActiveViewId(viewId);
            }
        };
        window.addEventListener('hashchange', handler);
        return () => window.removeEventListener('hashchange', handler);
    }, [viewOrder]);

    // Persist on view change
    useEffect(() => {
        if (!activePluginId) persistView(activeViewId);
    }, [activeViewId, activePluginId]);

    return { activeViewId, activePluginId, switchView, openPluginConfig, closePluginConfig, viewOrder };
}
