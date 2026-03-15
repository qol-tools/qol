import { useState, useEffect, useCallback } from 'preact/hooks';
import { tryFetchJson } from '../api/client.js';
import { preloadConfigForm } from '../views/plugin-config/usePluginConfig.js';

const VIEW_STORAGE_KEY = 'qoltray.activeView';

function parseHashRoute() {
    const raw = window.location.hash.replace(/^#/, '').trim();
    const pluginMatch = raw.match(/^plugins\/(.+)$/);
    if (pluginMatch) {
        return { viewId: 'plugins', pluginId: pluginMatch[1] };
    }
    return { viewId: raw || null, pluginId: null };
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
    const fromHash = parseHashRoute().viewId;
    if (fromHash) return fromHash;
    const fromStorage = readStoredView();
    if (fromStorage) return fromStorage;
    return 'plugins';
}

async function validatePluginConfig(pluginId) {
    if (!pluginId) return false;
    const form = await tryFetchJson(`/api/plugins/${pluginId}/config-form`);
    if (!form?.sections?.some(s => s.fields?.length)) return false;
    preloadConfigForm(pluginId, form);
    return true;
}

async function handleHashChange(viewOrder, setActivePluginId, setActiveViewId) {
    const route = parseHashRoute();
    if (route.pluginId) {
        if (await validatePluginConfig(route.pluginId)) {
            setActiveViewId('plugins');
            setActivePluginId(route.pluginId);
        }
        return;
    }
    setActivePluginId(null);
    if (route.viewId && viewOrder.includes(route.viewId)) {
        setActiveViewId(route.viewId);
    }
}

function doSwitchView(viewId, viewOrder, setActivePluginId, setActiveViewId) {
    if (!viewOrder.includes(viewId)) return;
    setActivePluginId(null);
    setActiveViewId(viewId);
    persistView(viewId);
}

function doClosePluginConfig(setActivePluginId, setActiveViewId) {
    setActivePluginId(null);
    setActiveViewId(prev => {
        const viewId = prev || 'plugins';
        persistView(viewId);
        return viewId;
    });
}

export function useRouter({ viewOrder }) {
    const [activeViewId, setActiveViewId] = useState(initActiveView);
    const [activePluginId, setActivePluginId] = useState(null);
    const switchView = useCallback(id => doSwitchView(id, viewOrder, setActivePluginId, setActiveViewId), [viewOrder]);
    const openPluginConfig = useCallback(async (pluginId) => {
        if (!await validatePluginConfig(pluginId)) return false;
        setActivePluginId(pluginId);
        window.history.pushState(null, '', `#plugins/${pluginId}`);
        return true;
    }, []);
    useEffect(() => {
        const initialPluginId = parseHashRoute().pluginId;
        if (!initialPluginId) return;
        validatePluginConfig(initialPluginId).then(valid => {
            if (valid) setActivePluginId(initialPluginId);
        });
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
