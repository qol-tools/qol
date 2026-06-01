import { useState, useEffect, useCallback, useRef } from 'preact/hooks';
import { tryFetchJson } from '../api/client.js';
import { preloadConfigForm } from '../views/plugin-config/usePluginConfig.js';
import { parseDeepRoute } from '../lib/deeplink-route.js';
import { resolveDeepLink } from '../lib/deeplink-resolve.js';
import { setPendingShortcutPrefill } from '../lib/deeplink-intent.js';

const VIEW_STORAGE_KEY = 'qoltray.activeView';
const PLUGIN_STORAGE_KEY = 'qoltray.activePlugin';

function clearLegacyStoredPlugin() {
    try { window.localStorage.removeItem(PLUGIN_STORAGE_KEY); } catch {}
}

function parseHashRoute() {
    const raw = window.location.hash.replace(/^#/, '').trim();
    const configMatch = raw.match(/^plugins\/(.+)\/config$/);
    if (configMatch) {
        return { viewId: 'plugins', pluginId: configMatch[1] };
    }
    const viewId = raw.split('/')[0] || null;
    return { viewId, pluginId: null };
}

function readStoredView() {
    try { return window.localStorage.getItem(VIEW_STORAGE_KEY); } catch { return null; }
}

function hashBaseViewId() {
    const raw = window.location.hash.replace(/^#/, '').trim();
    return raw.split(/[/?]/)[0] || null;
}

function isPluginConfigHash() {
    const raw = window.location.hash.replace(/^#/, '').trim();
    return /^plugins\/(.+)\/config$/.test(raw);
}

function persistView(viewId, { updateHash = true } = {}) {
    try { window.localStorage.setItem(VIEW_STORAGE_KEY, viewId); } catch {}
    if (!updateHash) return;
    // Keep a deep-link route already in the hash (e.g. `#shortcuts/add?type=url`)
    // when it targets this same view, so resolving it does not flatten the URL
    // back to `#shortcuts` (which loses the prefill on reload/share). Plugin
    // config routes are still flattened so closing config returns to the base.
    if (hashBaseViewId() === viewId && !isPluginConfigHash()) return;
    window.history.replaceState(null, '', `#${viewId}`);
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

async function handleHashChange(viewOrder, setActivePluginId, setActiveViewId) {
    const route = parseHashRoute();
    if (route.pluginId) {
        setActiveViewId('plugins');
        if (await validatePluginConfig(route.pluginId)) {
            setActivePluginId(route.pluginId);
        }
        return;
    }
    setActivePluginId(null);
    if (route.viewId && viewOrder.includes(route.viewId)) {
        setActiveViewId(route.viewId);
    }
    const deep = parseDeepRoute(window.location.hash);
    if (deep.action && viewOrder.includes(deep.page)) {
        resolveDeepLink(deep, { setPendingShortcutPrefill });
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
    const bootResolvedRef = useRef(false);
    const switchView = useCallback(id => doSwitchView(id, viewOrder, setActivePluginId, setActiveViewId), [viewOrder]);
    const openPluginConfig = useCallback(async (pluginId) => {
        if (!await validatePluginConfig(pluginId)) return false;
        setActivePluginId(pluginId);
        window.history.pushState(null, '', `#plugins/${pluginId}/config`);
        return true;
    }, []);
    useEffect(() => {
        clearLegacyStoredPlugin();
        const route = parseHashRoute();
        if (!route.pluginId) return;
        validatePluginConfig(route.pluginId).then(valid => {
            if (valid) setActivePluginId(route.pluginId);
        });
    }, []);
    const closePluginConfig = useCallback(() => doClosePluginConfig(setActivePluginId, setActiveViewId), []);
    useEffect(() => {
        const handler = () => handleHashChange(viewOrder, setActivePluginId, setActiveViewId);
        window.addEventListener('hashchange', handler);
        if (!bootResolvedRef.current) {
            bootResolvedRef.current = true;
            const deepBoot = parseDeepRoute(window.location.hash);
            if (deepBoot.action && viewOrder.includes(deepBoot.page)) {
                resolveDeepLink(deepBoot, { setPendingShortcutPrefill });
            }
        }
        return () => window.removeEventListener('hashchange', handler);
    }, [viewOrder]);
    useEffect(() => { if (!activePluginId) persistView(activeViewId); }, [activeViewId, activePluginId]);
    return { activeViewId, activePluginId, switchView, openPluginConfig, closePluginConfig, viewOrder };
}
