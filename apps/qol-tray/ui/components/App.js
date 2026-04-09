import { html } from '../lib/html.js';
import { createDiveStack } from '../lib/dive-stack.js';
import { useRef, useCallback, useEffect, useLayoutEffect, useState } from 'preact/hooks';
import { PaletteProvider, usePaletteContext } from '../palette/context.js';
import { createDebug } from '../lib/debug.js';
import { getWorldSettings } from '../lib/world-settings.js';

const log = createDebug('qol:app');
import { ModifierStateProvider } from '../hooks/modifier-state-context.js';
import { PluginConfigProvider } from '../views/plugin-config/context.js';
import { useApp } from './app/useApp.js';
import { useAppKeyboardRouting } from './app/useAppKeyboardRouting.js';
import { ViewKeyboardProvider } from './app/view-keyboard-context.js';
import { renderWorldViews } from './app/views.js';
import { RecompileDissolve } from './RecompileDissolve.js';
import { GlobalToast } from './ApiErrorToast.js';
import { SelectionCursorOverlay } from './SelectionCursorOverlay.js';
import { CommandPalette } from './CommandPalette.js';
import { createCamera } from '../lib/world-camera.js';
import { createWorldRegistry } from '../lib/world-registry.js';
import { WorldViewport } from './app/WorldViewport.js';
import { MinimapContainer } from './app/Minimap.js';
import { RegionLabels } from './app/RegionLabels.js';
import { useWorldNav } from './app/WorldNav.js';

const SUB_PAGE_MANIFEST = {
    plugins: ['config'],
    hotkeys: ['editor'],
    shortcuts: ['editor'],
    logs: ['detail'],
    'task-runner': ['editor'],
};

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

    const registryRef = useRef(null);
    if (!registryRef.current) registryRef.current = createWorldRegistry(viewOrder, SUB_PAGE_MANIFEST);
    const registry = registryRef.current;

    const diveStackRef = useRef(null);
    if (!diveStackRef.current) diveStackRef.current = createDiveStack();
    const diveStack = diveStackRef.current;

    const [cameraLayer, setCameraLayer] = useState(0);
    const [diveParent, setDiveParent] = useState(null);
    const diveParentRef = useRef(null);
    const layerAnimatingRef = useRef(false);

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

    const prevViewRef = useRef(null);
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
                    log('viewChange:', activeViewId, '→ jump (dist:', Math.round(dist), ')');
                    camera.panTo(target.x, target.y);
                } else {
                    log('viewChange:', activeViewId, '→ already near target');
                }
            }
        }
    }, [activeViewId, camera, registry]);

    useLayoutEffect(() => {
        const worldEl = document.getElementById('world');
        if (worldEl) camera.setWorldElement(worldEl);
    }, []);

    useEffect(() => {
        requestAnimationFrame(() => {
            const vp = document.getElementById('viewport');
            if (!vp) return;
            const w = vp.clientWidth;
            const h = vp.clientHeight;
            if (!w || !h) return;
            const target = registry.cameraTargetForView(activeViewId, w, h, camera.zoom);
            if (target) {
                log('init center:', activeViewId, w, 'x', h, '→', Math.round(target.x), Math.round(target.y));
                camera.panTo(target.x, target.y);
            }
        });
    }, []);

    useWorldNav({ camera, registry, viewportRef });

    const dive = useCallback((targetId, sourceSurface) => {
        if (layerAnimatingRef.current) return;
        const entry = registry.diveTarget(targetId);
        if (!entry) return;
        const vp = viewportRef.current;
        if (!vp) return;
        diveStack.push({
            layer: camera.layer,
            x: camera.x, y: camera.y, zoom: camera.zoom,
            surfaceSelector: sourceSurface ? selectorFor(sourceSurface) : null,
            diveParent: diveParentRef.current,
        });
        const focusTarget = () => requestAnimationFrame(() => {
            const slot = document.querySelector(`.world-view-slot[data-view-id="${CSS.escape(targetId)}"]`);
            const surface = slot?.querySelector('[data-selected-surface]');
            if (surface) surface.focus({ preventScroll: true });
        });
        const newParent = entry.parent || targetId;
        const applyLayer = () => {
            setCameraLayer(entry.layer);
            camera.setLayer(entry.layer);
            diveParentRef.current = newParent;
            setDiveParent(newParent);
            const w = vp.clientWidth || 800;
            const h = vp.clientHeight || 600;
            const target = registry.cameraTargetForView(targetId, w, h, camera.zoom);
            if (target) camera.panTo(target.x, target.y);
        };
        animateTransition(vp, layerAnimatingRef, 'dive-out', applyLayer, focusTarget);
    }, [camera, registry, diveStack]);

    const ascend = useCallback(() => {
        if (layerAnimatingRef.current) return false;
        const prev = diveStack.pop();
        if (!prev) return false;
        const vp = viewportRef.current;
        if (!vp) return false;
        const restoredParent = prev.diveParent ?? null;
        const applyLayer = () => {
            setCameraLayer(prev.layer);
            camera.setLayer(prev.layer);
            camera.panTo(prev.x, prev.y);
            diveParentRef.current = restoredParent;
            setDiveParent(restoredParent);
        };
        const focusTarget = prev.surfaceSelector
            ? () => requestAnimationFrame(() => {
                const surface = document.querySelector(prev.surfaceSelector);
                if (surface) surface.focus({ preventScroll: true });
            })
            : null;
        animateTransition(vp, layerAnimatingRef, 'ascend-out', applyLayer, focusTarget);
        return true;
    }, [camera, diveStack]);

    const pluginDiveRef = useRef(false);
    useEffect(() => {
        if (activePluginId && !pluginDiveRef.current) {
            pluginDiveRef.current = true;
            const source = document.querySelector('[data-selected-surface][data-selected="true"]');
            dive('plugins-config', source);
        } else if (!activePluginId && pluginDiveRef.current) {
            pluginDiveRef.current = false;
            ascend();
        }
    }, [activePluginId, dive, ascend]);

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
                            closePluginConfig,
                            syncStatus,
                            syncProviders,
                            onSyncStatusChange: setSyncStatus,
                            refreshSyncStatus,
                        })}
                    <//>
                    <${CommandPalette} />
                    <${MinimapContainer} camera=${camera} registry=${registry} viewportRef=${viewportRef} diveParent=${diveParent} />
                    <${SelectionCursorOverlay} camera=${camera} />
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

function animateTransition(vp, animatingRef, outClass, applyLayer, onDone) {
    const { transitionStyle, transitionSpeed } = getWorldSettings();
    const minimap = document.querySelector('.world-minimap-container');
    if (transitionStyle === 'instant') {
        applyLayer();
        if (onDone) onDone();
        return;
    }
    animatingRef.current = true;
    const outAnim = transitionStyle === 'fade' ? 'fade-out' : outClass;
    const dur = `${transitionSpeed}ms`;
    const durIn = `${Math.round(transitionSpeed * 0.6)}ms`;
    applyAnimClass(vp, outAnim, dur);
    applyAnimClass(minimap, outAnim, dur);
    vp.addEventListener('animationend', function onEnd() {
        vp.removeEventListener('animationend', onEnd);
        clearAnimClass(vp, outAnim);
        clearAnimClass(minimap, outAnim);
        applyLayer();
        applyAnimClass(vp, 'layer-in', durIn);
        applyAnimClass(minimap, 'layer-in', durIn);
        vp.addEventListener('animationend', function onIn() {
            vp.removeEventListener('animationend', onIn);
            clearAnimClass(vp, 'layer-in');
            clearAnimClass(minimap, 'layer-in');
            animatingRef.current = false;
        });
        if (onDone) onDone();
    });
}

function applyAnimClass(el, cls, dur) {
    if (!el) return;
    el.style.animationDuration = dur;
    el.classList.add(cls);
}

function clearAnimClass(el, cls) {
    if (!el) return;
    el.classList.remove(cls);
    el.style.animationDuration = '';
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
