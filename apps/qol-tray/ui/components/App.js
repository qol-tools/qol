import { html } from '../lib/html.js';
import { useState, useEffect, useRef, useCallback, useMemo } from 'preact/hooks';
import { SidebarNav } from './SidebarNav.js';
import { SidebarFooter } from './SidebarFooter.js';
import { useRouter } from '../hooks/useRouter.js';
import { useSSE, useSSEReconnect } from '../hooks/useSSE.js';
import { useKeyboard } from '../hooks/useKeyboard.js';
import { readResponseText } from '../api/client.js';
import { clampPercent } from '../utils/progress.js';

import { PluginsView } from '../views/plugins-view.js';
import { StoreView } from '../views/store-view.js';
import { HotkeysView } from '../views/hotkeys-view.js';
import { TaskRunnerView } from '../views/task-runner-view.js';
import { DevView } from '../views/dev-view.js';

const VIEW_MAP = { plugins: PluginsView, store: StoreView, hotkeys: HotkeysView, 'task-runner': TaskRunnerView, dev: DevView };
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

    const router = useRouter({ viewOrder });
    const { activeViewId, activePluginId, switchView, openPluginConfig, closePluginConfig } = router;

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
        if (!result) { setUpdateState({ status: 'error' }); return; }
        setUpdateState(result.available ? { status: 'available', latest: result.latest } : { status: 'up-to-date' });
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

    const DEV_FLOW_CLEAR_MS = { recompile: 1800, update: 2000 };
    const DEV_FLOW_EVENTS = {
        self_recompile_progress: 'recompile', self_recompile_complete: 'recompile', self_recompile_failed: 'recompile',
        update_progress: 'update', update_complete: 'update', update_failed: 'update'
    };

    const DEV_FLOW_TRANSITIONS = {
        progress: (flow, key, event) => {
            flow.active = true;
            flow.percent = clampPercent(event.percent);
            if (key === 'recompile') flow.phase = (typeof event.phase === 'string' && event.phase.trim()) ? event.phase : 'Recompiling QoL Tray';
            flow.done = false;
            flow.error = null;
        },
        complete: (flow, key, event) => {
            flow.active = false;
            flow.percent = 100;
            flow.done = true;
            flow.error = null;
            scheduleDevFlowDoneClear(key, DEV_FLOW_CLEAR_MS[key]);
        },
        failed: (flow, key, event) => {
            flow.active = false;
            flow.done = false;
            flow.error = event?.message || `${key} failed`;
        }
    };

    const applyDevFlowTransition = useCallback((key, phase, event) => {
        clearDevFlowTimer(key);
        DEV_FLOW_TRANSITIONS[phase](devFlowsRef.current[key], key, event);
        syncSidebar();
    }, [clearDevFlowTimer, syncSidebar, scheduleDevFlowDoneClear]);

    const NON_DEV_SSE_HANDLERS = {
        update_progress: (event) => setUpdateState({ status: 'downloading', percent: clampPercent(event.percent) }),
        update_complete: () => { setUpdateState({ status: 'done' }); setTimeout(() => checkForUpdate(), 30000); },
        update_failed: () => setUpdateState({ status: 'error' })
    };

    const handleSSE = useCallback((event) => {
        if (devEnabled) {
            const key = DEV_FLOW_EVENTS[event.type];
            if (!key) return;
            const phase = event.type.endsWith('_progress') ? 'progress' : event.type.endsWith('_complete') ? 'complete' : 'failed';
            applyDevFlowTransition(key, phase, event);
            return;
        }
        NON_DEV_SSE_HANDLERS[event.type]?.(event);
    }, [devEnabled, clearDevFlowTimer, syncSidebar, scheduleDevFlowDoneClear]);

    useSSE(handleSSE);
    useSSEReconnect(useCallback(() => {
        if (devEnabled) {
            const flows = devFlowsRef.current;
            if (flows.recompile.active) applyDevFlowTransition('recompile', 'complete', {});
            if (flows.update.active) applyDevFlowTransition('update', 'complete', {});
            return;
        }
        if (updateState.status === 'done') checkForUpdate();
    }, [devEnabled, updateState.status, checkForUpdate, applyDevFlowTransition]));

    // Sidebar actions
    const handleSidebarAction = useCallback(async (action) => {
        if (action === 'check-update') { checkForUpdate(); return; }
        if (action === 'self-update') {
            if (devEnabled) {
                clearDevFlowTimer('update');
                devFlowsRef.current.update = { active: true, percent: 0, done: false, error: null, clearTimer: null };
                syncSidebar();
            }
            if (!devEnabled) setUpdateState({ status: 'downloading', percent: 0 });
            try {
                await fetch('/api/self-update', { method: 'POST' });
            } catch {
                if (devEnabled) applyDevFlowTransition('update', 'failed', { message: 'Update failed' });
                if (!devEnabled) setUpdateState({ status: 'error' });
            }
            return;
        }
        if (action === 'dev-recompile') {
            const flows = devFlowsRef.current;
            if (!devEnabled || flows.recompile.active || flows.update.active) return;
            clearDevFlowTimer('recompile');
            flows.recompile = { active: true, percent: 0, phase: 'Preparing build', done: false, error: null, clearTimer: null };
            syncSidebar();
            const RECOMPILE_ERRORS = {
                404: 'Connected daemon is older than this UI. Stop it and launch the current checkout.',
                409: 'Recompile already in progress'
            };
            try {
                const res = await fetch('/api/dev/recompile-self', { method: 'POST' });
                if (!res.ok) {
                    const body = await readResponseText(res);
                    throw new Error(RECOMPILE_ERRORS[res.status] || body || `Could not start recompile (${res.status})`);
                }
            } catch (error) {
                applyDevFlowTransition('recompile', 'failed', { message: error?.message || 'Could not start recompile' });
            }
        }
    }, [devEnabled, checkForUpdate, clearDevFlowTimer, syncSidebar, applyDevFlowTransition]);

    const cycleView = useCallback((e) => {
        e.preventDefault();
        const idx = viewOrder.indexOf(activeViewId);
        const next = e.shiftKey
            ? (idx - 1 + viewOrder.length) % viewOrder.length
            : (idx + 1) % viewOrder.length;
        switchView(viewOrder[next]);
    }, [viewOrder, activeViewId, switchView]);

    // Global keyboard handler
    useKeyboard(useCallback((e) => {
        if (activePluginId) {
            if (e.key === 'Escape') { e.preventDefault(); closePluginConfig(); return; }
            if (e.key === 'Tab') { closePluginConfig(); cycleView(e); }
            return;
        }

        const view = VIEW_MAP[activeViewId];

        if (view?.isBlocking?.()) {
            if (view.handleKey) view.handleKey(e);
            return;
        }

        if (e.key === 'Tab') { cycleView(e); return; }

        if (view?.handleKey) view.handleKey(e);
    }, [activePluginId, activeViewId, closePluginConfig, cycleView]));

    // Lazy mount: only mount a view when first visited, then keep it alive (display:none).
    // This preserves component state across view switches like the old vanilla JS did.
    const [mounted, setMounted] = useState(() => new Set([activeViewId]));
    useEffect(() => {
        setMounted(prev => {
            if (prev.has(activeViewId)) return prev;
            const next = new Set(prev);
            next.add(activeViewId);
            return next;
        });
    }, [activeViewId]);

    const slotStyle = (id) => activeViewId === id && !activePluginId
        ? 'flex:1;min-height:0;display:flex;flex-direction:column'
        : 'display:none';

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
                    ${mounted.has('plugins') && html`<div style=${slotStyle('plugins')}><${PluginsView} onOpenPluginConfig=${openPluginConfig} /></div>`}
                    ${mounted.has('store') && html`<div style=${slotStyle('store')}><${StoreView} /></div>`}
                    ${mounted.has('hotkeys') && html`<div style=${slotStyle('hotkeys')}><${HotkeysView} /></div>`}
                    ${mounted.has('task-runner') && html`<div style=${slotStyle('task-runner')}><${TaskRunnerView} /></div>`}
                    ${mounted.has('dev') && html`<div style=${slotStyle('dev')}><${DevView} /></div>`}
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
