import { useState, useEffect, useCallback } from 'preact/hooks';
import { tryFetchJson } from '../api/client.js';
import { preloadConfigForm } from '../views/plugin-config/usePluginConfig.js';

const VIEW_STORAGE_KEY = 'qoltray.activeView';

function parseHashRoute() {
    const raw = window.location.hash.replace(/^#/, '').trim();
    const configMatch = raw.match(/^plugins\/(.+)\/config$/);
    if (configMatch) {
        return { viewId: 'plugins', pluginId: configMatch[1], mode: 'config' };
    }
    const pluginMatch = raw.match(/^plugins\/(.+)$/);
    if (pluginMatch) {
        return { viewId: 'plugins', pluginId: pluginMatch[1], mode: 'ui' };
    }
    const viewId = raw.split('/')[0] || null;
    return { viewId, pluginId: null, mode: null };
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
    const hasRootFields = form?.fields?.length > 0;
    const hasSectionFields = form?.sections?.some(s => s.fields?.length);
    if (!hasRootFields && !hasSectionFields) return false;
    preloadConfigForm(pluginId, form);
    return true;
}

async function handleHashChange(viewOrder, setActivePluginId, setActiveViewId, setActivePluginMode) {
    const route = parseHashRoute();
    if (route.pluginId) {
        setActiveViewId('plugins');
        if (await validatePluginConfig(route.pluginId)) {
            setActivePluginId(route.pluginId);
            setActivePluginMode('config');
            return;
        }
        if (route.mode === 'ui') {
            setActivePluginId(route.pluginId);
            setActivePluginMode('ui');
        }
        return;
    }
    setActivePluginId(null);
    setActivePluginMode(null);
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

function doClosePluginConfig(setActivePluginId, setActiveViewId, setActivePluginMode) {
    setActivePluginId(null);
    setActivePluginMode(null);
    setActiveViewId(prev => {
        const viewId = prev || 'plugins';
        persistView(viewId);
        return viewId;
    });
}

export function useRouter({ viewOrder }) {
    const [activeViewId, setActiveViewId] = useState(initActiveView);
    const [activePluginId, setActivePluginId] = useState(null);
    const [activePluginMode, setActivePluginMode] = useState(null);
    const switchView = useCallback(id => doSwitchView(id, viewOrder, setActivePluginId, setActiveViewId), [viewOrder]);
    const openPluginConfig = useCallback(async (pluginId) => {
        if (!await validatePluginConfig(pluginId)) return false;
        setActivePluginId(pluginId);
        setActivePluginMode('config');
        window.history.pushState(null, '', `#plugins/${pluginId}/config`);
        return true;
    }, []);
    const openPluginUi = useCallback((pluginId) => {
        setActivePluginId(pluginId);
        setActivePluginMode('ui');
        window.location.hash = `#plugins/${pluginId}`;
    }, []);
    useEffect(() => {
        const route = parseHashRoute();
        if (!route.pluginId) return;
        validatePluginConfig(route.pluginId).then(valid => {
            if (valid) {
                setActivePluginId(route.pluginId);
                setActivePluginMode('config');
            } else if (route.mode === 'ui') {
                setActivePluginId(route.pluginId);
                setActivePluginMode('ui');
            }
        });
    }, []);
    const closePluginConfig = useCallback(() => doClosePluginConfig(setActivePluginId, setActiveViewId, setActivePluginMode), []);
    useEffect(() => {
        const handler = () => handleHashChange(viewOrder, setActivePluginId, setActiveViewId, setActivePluginMode);
        window.addEventListener('hashchange', handler);
        return () => window.removeEventListener('hashchange', handler);
    }, [viewOrder]);
    useEffect(() => { if (!activePluginId) persistView(activeViewId); }, [activeViewId, activePluginId]);
    return { activeViewId, activePluginId, activePluginMode, switchView, openPluginConfig, openPluginUi, closePluginConfig, viewOrder };
}
