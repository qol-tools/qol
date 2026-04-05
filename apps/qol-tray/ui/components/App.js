import { html } from '../lib/html.js';
import { createDiveStack } from '../lib/dive-stack.js';
import { useRef, useCallback, useEffect, useState } from 'preact/hooks';
import { PaletteProvider, usePaletteContext } from '../palette/context.js';
import { createDebug } from '../lib/debug.js';

const log = createDebug('qol:app');
import { ModifierStateProvider } from '../hooks/modifier-state-context.js';
import { PluginConfigProvider } from '../views/plugin-config/context.js';
import { useApp } from './app/useApp.js';
import { useAppKeyboardRouting } from './app/useAppKeyboardRouting.js';
import { ViewKeyboardProvider } from './app/view-keyboard-context.js';
import { renderWorldViews } from './app/views.js';
import { RecompileDissolve } from './RecompileDissolve.js';
import { PluginConfigView } from '../views/plugin-config/view.js';
import { GlobalToast } from './ApiErrorToast.js';
import { SelectionCursorOverlay } from './SelectionCursorOverlay.js';
import { CommandPalette } from './CommandPalette.js';
import { createCamera } from '../lib/world-camera.js';
import { createWorldRegistry } from '../lib/world-registry.js';
import { WorldViewport } from './app/WorldViewport.js';
import { Minimap } from './app/Minimap.js';
import { RegionLabels } from './app/RegionLabels.js';
import { useWorldNav } from './app/WorldNav.js';

export function App() {
    return html`<${PaletteProvider}><${AppShell} /><//>`;
}

function AppShell() {
    const dissolveRef = useRef(null);
    const onDissolve = useCallback((reload) => dissolveRef.current?.(reload), []);
    const {
        devEnabled,
        appVersion,
        viewOrder,
        activeViewId,
        activePluginId,
        activePluginMode,
        switchView,
        openPluginConfig,
        openPluginUi,
        closePluginConfig,
        updateState,
        handleSidebarAction,
        worktrees,
        defaultWorktree,
        setDefaultWorktree,
        syncStatus,
        syncProviders,
        setSyncStatus,
        refreshSyncStatus,
    } = useApp({ onDissolve });

    const cameraRef = useRef(null);
    if (!cameraRef.current) cameraRef.current = createCamera();
    const camera = cameraRef.current;
    window.__worldCamera = camera;

    const registryRef = useRef(null);
    const SUB_PAGE_MANIFEST = { hotkeys: ['editor'] };
    if (!registryRef.current) registryRef.current = createWorldRegistry(viewOrder, SUB_PAGE_MANIFEST);
    const registry = registryRef.current;

    const diveStackRef = useRef(null);
    if (!diveStackRef.current) diveStackRef.current = createDiveStack();
    const diveStack = diveStackRef.current;

    const [cameraLayer, setCameraLayer] = useState(0);

    // Add views that appear after initial render (e.g. dev when devEnabled flips)
    useEffect(() => {
        for (const id of viewOrder) {
            if (!registry.getEntry(id)) {
                log('registry: adding late view', id);
                registry.placeNew(id);
            }
        }
    }, [viewOrder, registry]);

    const viewportRef = useRef(null);

    useEffect(() => {
        const el = document.getElementById('viewport');
        viewportRef.current = el;
    }, []);

    const prevViewRef = useRef(activeViewId);
    useEffect(() => {
        const vp = viewportRef.current;
        const w = vp?.clientWidth || 800;
        const h = vp?.clientHeight || 600;
        if (prevViewRef.current !== activeViewId) {
            prevViewRef.current = activeViewId;
            const target = registry.cameraTargetForView(activeViewId, w, h, camera.zoom);
            if (target) {
                const dist = Math.hypot(camera.x - target.x, camera.y - target.y);
                if (dist > 50) {
                    log('viewChange:', activeViewId, '→ pan (dist:', Math.round(dist), ')');
                    camera.panSmooth(target.x, target.y, 250);
                } else {
                    log('viewChange:', activeViewId, '→ already near target');
                }
            }
        }
    }, [activeViewId, camera, registry]);

    const OVERVIEW_ZOOM = 0.12;

    useEffect(() => {
        camera.zoomTo(OVERVIEW_ZOOM);
        const vp = viewportRef.current;
        const target = registry.cameraTargetForView(activeViewId, vp?.clientWidth || 800, vp?.clientHeight || 600, OVERVIEW_ZOOM);
        if (target) camera.panTo(target.x, target.y);
    }, []);

    useWorldNav({ camera, registry, viewportRef });

    const dive = useCallback((targetId, sourceSurface) => {
        const entry = registry.diveTarget(targetId);
        if (!entry) return;
        diveStack.push({
            layer: camera.layer,
            x: camera.x, y: camera.y, zoom: camera.zoom,
            surfaceSelector: sourceSurface ? selectorFor(sourceSurface) : null,
        });
        const vp = viewportRef.current;
        const w = vp?.clientWidth || 800;
        const h = vp?.clientHeight || 600;
        setCameraLayer(entry.layer);
        camera.setLayer(entry.layer);
        const target = registry.cameraTargetForView(targetId, w, h, 1.0);
        if (target) {
            camera.zoomSmooth(target.x, target.y, 1.0, 400, () => {
                const slot = document.querySelector(`.world-view-slot[data-view-id="${targetId}"]`);
                const surface = slot?.querySelector('[data-selected-surface]');
                if (surface) surface.focus({ preventScroll: true });
            });
        }
    }, [camera, registry, diveStack]);

    const ascend = useCallback(() => {
        const prev = diveStack.pop();
        if (!prev) return false;
        setCameraLayer(prev.layer);
        camera.setLayer(prev.layer);
        camera.zoomSmooth(prev.x, prev.y, prev.zoom, 400, () => {
            if (prev.surfaceSelector) {
                const surface = document.querySelector(prev.surfaceSelector);
                if (surface) surface.focus({ preventScroll: true });
            }
        });
        return true;
    }, [camera, diveStack]);

    return html`
        <${ModifierStateProvider}>
        <${PluginConfigProvider} pluginId=${activePluginId} mode=${activePluginMode}>
            <${ViewKeyboardProvider}>
                <${AppKeyboardRouting}
                    activePluginId=${activePluginId}
                    activeViewId=${activeViewId}
                    camera=${camera}
                    closePluginConfig=${closePluginConfig}
                    switchView=${switchView}
                    viewOrder=${viewOrder}
                    dive=${dive}
                    ascend=${ascend}
                />
                <div class="app-container">
                    <${WorldViewport} camera=${camera} onViewChange=${switchView}>
                        <${RegionLabels} registry=${registry} />
                        ${renderWorldViews({
                            registry,
                            cameraLayer,
                            openPluginConfig,
                            openPluginUi,
                            syncStatus,
                            syncProviders,
                            onSyncStatusChange: setSyncStatus,
                            refreshSyncStatus,
                        })}
                    <//>
                    <${CommandPalette} />
                    <${Minimap} camera=${camera} registry=${registry} viewportRef=${viewportRef} />
                    <${SelectionCursorOverlay} />
                    <${RecompileDissolve} triggerRef=${dissolveRef} />
                    <${GlobalToast} />
                </div>
            <//>
        <//>
        <//>
    `;
}

function AppKeyboardRouting({ activePluginId, activeViewId, camera, closePluginConfig, switchView, viewOrder, dive, ascend }) {
    const palette = usePaletteContext();
    useAppKeyboardRouting({ activePluginId, activeViewId, camera, closePluginConfig, switchView, viewOrder, palette, dive, ascend });
    return null;
}

function selectorFor(el) {
    if (el.id) return `#${CSS.escape(el.id)}`;
    const viewId = el.closest('[data-view-id]')?.dataset?.viewId;
    const index = el.getAttribute('data-index');
    if (viewId && index != null) {
        return `[data-view-id="${CSS.escape(viewId)}"] [data-selected-surface][data-index="${index}"]`;
    }
    return null;
}
