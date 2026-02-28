import { html } from '../lib/html.js';
import { useState, useEffect, useRef, useCallback, useMemo } from 'preact/hooks';
import { SidebarNav } from './SidebarNav.js';
import { SidebarFooter } from './SidebarFooter.js';
// ShortcutLegend will be used once views are migrated
// import { ShortcutLegend } from './ShortcutLegendPreact.js';
import { useRouter } from '../hooks/useRouter.js';
import { useSSE, useSSEReconnect } from '../hooks/useSSE.js';
import { useKeyboard } from '../hooks/useKeyboard.js';
import { readResponseText } from '../api/client.js';
import { clampPercent } from '../utils/progress.js';

import { PluginsView } from '../views/PluginsView.js';
import { StoreView } from '../views/StoreView.js';
import { HotkeysView } from '../views/HotkeysView.js';
import { TaskRunnerView } from '../views/TaskRunnerView.js';
import * as devView from '../views/dev.js';

const PREACT_VIEWS = new Set(['plugins', 'store', 'hotkeys', 'task-runner']);
const PREACT_VIEW_MAP = { plugins: PluginsView, store: StoreView, hotkeys: HotkeysView, 'task-runner': TaskRunnerView };
const BRIDGE_VIEWS = {};
const BASE_ORDER = ['plugins', 'store', 'hotkeys', 'task-runner'];

function initDevFlows() {
    return {
        update: { active: false, percent: 0, done: false, error: null, clearTimer: null },
        recompile: { active: false, percent: 0, phase: 'Preparing build', done: false, error: null, clearTimer: null }
    };
}

function resolveDevSidebarState(devFlows) {
    const { recompile, update } = devFlows;
    if (recompile.error) return { status: 'error', message: recompile.error };
    if (recompile.active) return { status: 'compiling', percent: recompile.percent, phase: recompile.phase || 'Recompiling QoL Tray' };
    if (update.error) return { status: 'error', message: update.error };
    if (update.active) return { status: 'downloading', percent: update.percent };
    if (recompile.done) return { status: 'recompile_done' };
    if (update.done) return { status: 'done' };
    return { status: 'idle' };
}

export function App() {
    const [devEnabled, setDevEnabled] = useState(false);
    const [appVersion, setAppVersion] = useState(null);
    const [updateState, setUpdateState] = useState({ status: 'checking' });
    const devFlowsRef = useRef(initDevFlows());

    const viewOrder = useMemo(
        () => devEnabled ? [...BASE_ORDER, 'dev'] : [...BASE_ORDER],
        [devEnabled]
    );

    const bridgeModules = useMemo(
        () => devEnabled ? { ...BRIDGE_VIEWS, dev: devView } : { ...BRIDGE_VIEWS },
        [devEnabled]
    );

    const router = useRouter({ viewOrder });
    const { activeViewId, activePluginId, switchView, openPluginConfig, closePluginConfig } = router;

    const contentRef = useRef(null);
    const activeViewRef = useRef(null);

    // Init: fetch dev/enabled + version
    useEffect(() => {
        (async () => {
            let dev = false;
            try {
                const res = await fetch('/api/dev/enabled');
                dev = res.ok && await res.json();
            } catch {}
            setDevEnabled(dev);
            if (dev) setUpdateState({ status: 'idle' });

            try {
                const res = await fetch('/api/version');
                if (res.ok) setAppVersion(await res.text());
            } catch {}
        })();
    }, []);

    // Imperative view bridge (hotkeys, task-runner, dev only)
    useEffect(() => {
        if (PREACT_VIEWS.has(activeViewId) || activePluginId) {
            // Blur previous bridge view if switching away
            const prev = activeViewRef.current;
            if (prev?.onBlur) prev.onBlur();
            activeViewRef.current = null;
            return;
        }

        const el = contentRef.current;
        if (!el) return;

        const prev = activeViewRef.current;
        if (prev?.onBlur) prev.onBlur();

        el.innerHTML = '';
        const next = bridgeModules[activeViewId];
        if (!next) return;

        next.render(el);
        if (next.onFocus) next.onFocus();
        activeViewRef.current = next;

        return () => {
            if (next.onBlur) next.onBlur();
            activeViewRef.current = null;
        };
    }, [activeViewId, activePluginId, bridgeModules]);

    // When plugin iframe opens, blur active view
    useEffect(() => {
        if (!activePluginId) return;
        const prev = activeViewRef.current;
        if (prev?.onBlur) prev.onBlur();
        activeViewRef.current = null;
    }, [activePluginId]);

    // Check for updates (non-dev)
    const checkForUpdate = useCallback(async () => {
        setUpdateState({ status: 'checking' });
        const minDelay = new Promise(r => setTimeout(r, 800));
        let result;
        try {
            const res = await fetch('/api/check-update');
            if (!res.ok) throw new Error();
            result = await res.json();
        } catch { result = null; }
        await minDelay;
        setUpdateState(result
            ? (result.available ? { status: 'available', latest: result.latest } : { status: 'up-to-date' })
            : { status: 'error' });
    }, []);

    useEffect(() => {
        if (!devEnabled && appVersion) checkForUpdate();
    }, [devEnabled, appVersion, checkForUpdate]);

    // SSE: update/recompile events for sidebar
    const syncSidebar = useCallback(() => {
        setUpdateState(resolveDevSidebarState(devFlowsRef.current));
    }, []);

    const clearDevFlowTimer = useCallback((key) => {
        const flow = devFlowsRef.current[key];
        if (!flow?.clearTimer) return;
        clearTimeout(flow.clearTimer);
        flow.clearTimer = null;
    }, []);

    const scheduleDevFlowDoneClear = useCallback((key, ms) => {
        clearDevFlowTimer(key);
        const flow = devFlowsRef.current[key];
        if (!flow) return;
        flow.clearTimer = setTimeout(() => {
            flow.clearTimer = null;
            flow.done = false;
            if (!flow.active && !flow.error) syncSidebar();
        }, ms);
    }, [clearDevFlowTimer, syncSidebar]);

    const handleSSE = useCallback((event) => {
        const flows = devFlowsRef.current;

        if (devEnabled) {
            if (event.type === 'self_recompile_progress') {
                clearDevFlowTimer('recompile');
                flows.recompile.active = true;
                flows.recompile.percent = clampPercent(event.percent);
                flows.recompile.phase = (typeof event.phase === 'string' && event.phase.trim()) ? event.phase : 'Recompiling QoL Tray';
                flows.recompile.done = false;
                flows.recompile.error = null;
                syncSidebar();
                return;
            }
            if (event.type === 'self_recompile_complete') {
                clearDevFlowTimer('recompile');
                flows.recompile.active = false;
                flows.recompile.percent = 100;
                flows.recompile.done = true;
                flows.recompile.error = null;
                syncSidebar();
                scheduleDevFlowDoneClear('recompile', 1800);
                return;
            }
            if (event.type === 'self_recompile_failed') {
                clearDevFlowTimer('recompile');
                flows.recompile.active = false;
                flows.recompile.done = false;
                flows.recompile.error = event.message || 'Recompile failed';
                syncSidebar();
                return;
            }
            if (event.type === 'update_progress') {
                clearDevFlowTimer('update');
                flows.update.active = true;
                flows.update.percent = clampPercent(event.percent);
                flows.update.done = false;
                flows.update.error = null;
                syncSidebar();
                return;
            }
            if (event.type === 'update_complete') {
                clearDevFlowTimer('update');
                flows.update.active = false;
                flows.update.percent = 100;
                flows.update.done = true;
                flows.update.error = null;
                syncSidebar();
                scheduleDevFlowDoneClear('update', 2000);
                return;
            }
            if (event.type === 'update_failed') {
                clearDevFlowTimer('update');
                flows.update.active = false;
                flows.update.done = false;
                flows.update.error = event.message || 'Update failed';
                syncSidebar();
                return;
            }
            return;
        }

        // Non-dev mode
        if (event.type === 'update_progress') {
            setUpdateState({ status: 'downloading', percent: clampPercent(event.percent) });
        } else if (event.type === 'update_complete') {
            setUpdateState({ status: 'done' });
            setTimeout(() => checkForUpdate(), 30000);
        } else if (event.type === 'update_failed') {
            setUpdateState({ status: 'error' });
        }
    }, [devEnabled, clearDevFlowTimer, syncSidebar, scheduleDevFlowDoneClear]);

    useSSE(handleSSE);
    useSSEReconnect(useCallback(() => {
        if (!devEnabled && updateState.status === 'done') checkForUpdate();
    }, [devEnabled, updateState.status]));

    // Sidebar actions
    const handleSidebarAction = useCallback(async (action) => {
        if (action === 'check-update') { checkForUpdate(); return; }
        if (action === 'self-update') {
            if (devEnabled) {
                clearDevFlowTimer('update');
                devFlowsRef.current.update = { active: true, percent: 0, done: false, error: null, clearTimer: null };
                syncSidebar();
            } else {
                setUpdateState({ status: 'downloading', percent: 0 });
            }
            try {
                await fetch('/api/self-update', { method: 'POST' });
            } catch {
                if (devEnabled) {
                    clearDevFlowTimer('update');
                    devFlowsRef.current.update.active = false;
                    devFlowsRef.current.update.error = 'Update failed';
                    syncSidebar();
                } else {
                    setUpdateState({ status: 'error' });
                }
            }
            return;
        }
        if (action === 'dev-recompile') {
            const flows = devFlowsRef.current;
            if (!devEnabled || flows.recompile.active || flows.update.active) return;
            clearDevFlowTimer('recompile');
            flows.recompile = { active: true, percent: 0, phase: 'Preparing build', done: false, error: null, clearTimer: null };
            syncSidebar();
            try {
                const res = await fetch('/api/dev/recompile-self', { method: 'POST' });
                if (!res.ok) {
                    const body = await readResponseText(res);
                    throw new Error(res.status === 404
                        ? 'Connected daemon is older than this UI. Stop it and launch the current checkout.'
                        : res.status === 409 ? 'Recompile already in progress'
                        : body || `Could not start recompile (${res.status})`);
                }
            } catch (error) {
                clearDevFlowTimer('recompile');
                flows.recompile.active = false;
                flows.recompile.error = error?.message || 'Could not start recompile';
                syncSidebar();
            }
        }
    }, [devEnabled, checkForUpdate, clearDevFlowTimer, syncSidebar]);

    // Global keyboard handler
    useKeyboard(useCallback((e) => {
        if (activePluginId) {
            if (e.key === 'Escape') { e.preventDefault(); closePluginConfig(); return; }
            if (e.key === 'Tab') {
                e.preventDefault();
                closePluginConfig();
                const idx = viewOrder.indexOf(activeViewId);
                const next = e.shiftKey
                    ? (idx - 1 + viewOrder.length) % viewOrder.length
                    : (idx + 1) % viewOrder.length;
                switchView(viewOrder[next]);
                return;
            }
            return;
        }

        // Resolve active view handler (Preact component or bridge module)
        const view = PREACT_VIEW_MAP[activeViewId] || activeViewRef.current;

        if (view?.isBlocking?.()) {
            if (view.handleKey) view.handleKey(e);
            return;
        }

        if (e.key === 'Tab') {
            e.preventDefault();
            const idx = viewOrder.indexOf(activeViewId);
            const next = e.shiftKey
                ? (idx - 1 + viewOrder.length) % viewOrder.length
                : (idx + 1) % viewOrder.length;
            switchView(viewOrder[next]);
            return;
        }

        if (view?.handleKey) view.handleKey(e);
    }, [activePluginId, activeViewId, viewOrder, switchView, closePluginConfig]));

    return html`
        <div class="app-container">
            <div class="app-main">
                <aside id="sidebar">
                    <${SidebarNav}
                        activeViewId=${activeViewId}
                        viewOrder=${viewOrder}
                        pluginOpen=${!!activePluginId}
                        onViewClick=${(id) => {
                            if (activePluginId) closePluginConfig();
                            switchView(id);
                        }}
                        onBack=${closePluginConfig}
                    />
                </aside>
                <main id="content" class=${activePluginId ? 'has-plugin-iframe' : ''}>
                    ${activePluginId && html`<iframe src="/plugins/${activePluginId}/" class="plugin-iframe"></iframe>`}
                    ${!activePluginId && activeViewId === 'plugins' && html`<${PluginsView} onOpenPluginConfig=${openPluginConfig} />`}
                    ${!activePluginId && activeViewId === 'store' && html`<${StoreView} />`}
                    ${!activePluginId && activeViewId === 'hotkeys' && html`<${HotkeysView} />`}
                    ${!activePluginId && activeViewId === 'task-runner' && html`<${TaskRunnerView} />`}
                    ${!activePluginId && !PREACT_VIEWS.has(activeViewId) && html`<div ref=${contentRef}></div>`}
                </main>
            </div>
            <div class="app-footer">
                <div id="sidebar-footer" class="app-footer-sidebar">
                    <${SidebarFooter}
                        version=${appVersion}
                        updateState=${updateState}
                        isDevMode=${devEnabled}
                        onAction=${handleSidebarAction}
                    />
                </div>
                <div id="content-footer" class="app-footer-content"></div>
            </div>
        </div>
    `;
}
