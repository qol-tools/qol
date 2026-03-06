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

function initActiveView() {
    const fromHash = parseViewFromHash();
    if (fromHash) return fromHash;
    const fromStorage = readStoredView();
    if (fromStorage) return fromStorage;
    return 'plugins';
}

function initActivePluginId() {
    const hash = window.location.hash.replace(/^#/, '').trim();
    const match = hash.match(/^plugins\/(.+)$/);
    return match ? match[1] : null;
}

function handleHashChange(viewOrder, setActivePluginId, setActiveViewId) {
    const raw = window.location.hash.replace(/^#/, '').trim();
    const pluginMatch = raw.match(/^plugins\/(.+)$/);
    if (pluginMatch) { setActivePluginId(pluginMatch[1]); return; }
    setActivePluginId(null);
    const viewId = parseViewFromHash();
    if (viewOrder.includes(viewId)) setActiveViewId(viewId);
}

function doSwitchView(viewId, viewOrder, setActivePluginId, setActiveViewId) {
    if (!viewOrder.includes(viewId)) return;
    setActivePluginId(null);
    setActiveViewId(viewId);
    persistView(viewId);
}

function doClosePluginConfig(setActivePluginId, setActiveViewId) {
    setActivePluginId(null);
    setActiveViewId(prev => { persistView(prev); return prev; });
}

export function useRouter({ viewOrder }) {
    const [activeViewId, setActiveViewId] = useState(initActiveView);
    const [activePluginId, setActivePluginId] = useState(initActivePluginId);
    const switchView = useCallback(id => doSwitchView(id, viewOrder, setActivePluginId, setActiveViewId), [viewOrder]);
    const openPluginConfig = useCallback((pluginId) => {
        setActivePluginId(pluginId);
        window.history.replaceState(null, '', `#plugins/${pluginId}`);
    }, []);
    const closePluginConfig = useCallback(() => doClosePluginConfig(setActivePluginId, setActiveViewId), []);
    useEffect(() => {
        const handler = () => handleHashChange(viewOrder, setActivePluginId, setActiveViewId);
        window.addEventListener('hashchange', handler);
        return () => window.removeEventListener('hashchange', handler);
    }, [viewOrder]);
    useEffect(() => { if (!activePluginId) persistView(activeViewId); }, [activeViewId, activePluginId]);
    return { activeViewId, activePluginId, switchView, openPluginConfig, closePluginConfig, viewOrder };
}
