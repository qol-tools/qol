import { html } from '../lib/html.js';
import { useRef, useCallback, useEffect, useLayoutEffect, useState } from 'preact/hooks';
import { PaletteProvider, usePaletteContext } from '../palette/context.js';
import { createDebug } from '../lib/debug.js';
import { createNavigation, selectorFor, animateTransition } from '../lib/world-navigation.js';
import { getWorldSettings } from '../lib/world-settings.js';

const log = createDebug('qol:app');
import { ModifierStateProvider } from '../hooks/modifier-state-context.js';
import { PluginConfigProvider } from '../views/plugin-config/context.js';
import { useApp } from './app/useApp.js';
import { useAppKeyboardRouting } from './app/useAppKeyboardRouting.js';
import { ViewKeyboardProvider } from './app/view-keyboard-context.js';
import { buildViewOrder, renderWorldViews } from './app/views.js';
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

function registerStaticDiveTargets(registry) {
    const PAGE_WIDTH = 1280;
    const PAGE_HEIGHT = 900;
    const staticTargets = [
        { parentId: 'plugins', subId: 'plugins-config' },
        { parentId: 'hotkeys', subId: 'hotkeys-editor' },
        { parentId: 'shortcuts', subId: 'shortcuts-editor' },
        { parentId: 'logs', subId: 'logs-detail' },
        { parentId: 'task-runner', subId: 'task-runner-editor' },
    ];
    for (const t of staticTargets) {
        const parent = registry.getEntry(t.parentId);
        if (!parent) continue;
        const claim = {
            x: parent.x,
            y: parent.y,
            width: PAGE_WIDTH,
            height: PAGE_HEIGHT,
            layer: parent.layer - 1,
        };
        registry.addEntry({
            id: t.subId,
            x: claim.x,
            y: claim.y,
            width: PAGE_WIDTH,
            height: PAGE_HEIGHT,
            layer: claim.layer,
        });
        registry.addDiveTarget({
            sourceSelector: `[data-view-id="${t.parentId}"]`,
            claim,
            pages: [t.subId],
        });
    }
}

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

    const viewportRef = useRef(null);

    useEffect(() => {
        const el = document.getElementById('viewport');
        viewportRef.current = el;
    }, []);

    const cameraRef = useRef(null);
    if (!cameraRef.current) {
        cameraRef.current = createCamera({
            getViewportSize: () => {
                const el = viewportRef.current;
                return {
                    w: el?.clientWidth || window.innerWidth,
                    h: el?.clientHeight || window.innerHeight,
                };
            },
        });
    }
    const camera = cameraRef.current;

    const registryRef = useRef(null);
    if (!registryRef.current) {
        const reg = createWorldRegistry(buildViewOrder(true), {});
        registerStaticDiveTargets(reg);
        registryRef.current = reg;
    }
    const registry = registryRef.current;

    const [cameraLayer, setCameraLayer] = useState(0);
    const [diveParent, setDiveParent] = useState(null);
    const [diveDepth, setDiveDepth] = useState(0);
    const diveParentRef = useRef(null);
    const layerAnimatingRef = useRef(false);

    useEffect(() => {
        for (const id of viewOrder) {
            if (!registry.getEntry(id)) registry.placeNew(id);
        }
    }, [viewOrder, registry]);

    const navigationRef = useRef(null);
    if (!navigationRef.current) {
        navigationRef.current = createNavigation({
            registry,
            camera,
            getSettings: getWorldSettings,
            domHelpers: {
                resolveSelector: (selector) => {
                    const el = document.querySelector(selector);
                    if (!el) return null;
                    const vpEl = viewportRef.current;
                    if (!vpEl) return null;
                    const vr = el.getBoundingClientRect();
                    const vpr = vpEl.getBoundingClientRect();
                    const relCenterX = (vr.left + vr.width / 2) - vpr.left;
                    const relCenterY = (vr.top + vr.height / 2) - vpr.top;
                    return {
                        x: camera.x + relCenterX / camera.zoom,
                        y: camera.y + relCenterY / camera.zoom,
                    };
                },
                getViewportSize: () => {
                    const el = viewportRef.current;
                    return {
                        w: el?.clientWidth || window.innerWidth,
                        h: el?.clientHeight || window.innerHeight,
                    };
                },
                crossLayerTransition: (entry, applyAndPan) => {
                    const vp = viewportRef.current;
                    if (!vp) { applyAndPan(); return; }
                    const outClass = entry.layer < camera.layer ? 'dive-out' : 'ascend-out';
                    animateTransition(vp, layerAnimatingRef, outClass, applyAndPan, null);
                },
            },
        });
    }
    const navigation = navigationRef.current;

    useEffect(() => {
        const unsub = camera.subscribe(({ layer }) => setCameraLayer(layer));
        return unsub;
    }, [camera]);

    const prevViewRef = useRef(activeViewId);
    useEffect(() => {
        if (prevViewRef.current === activeViewId) return;
        prevViewRef.current = activeViewId;
        log('viewChange:', activeViewId, '→ gotoAnchor');
        navigation.setCurrentAnchor({ pageId: activeViewId });
        navigation.gotoAnchor({ pageId: activeViewId }, { respectKnob: true });
    }, [activeViewId, navigation]);

    useLayoutEffect(() => {
        const worldEl = document.getElementById('world');
        if (worldEl) camera.setWorldElement(worldEl);
        const current = navigation.getCurrentAnchor();
        const fallback = registry.getEntriesForLayer(0)[0]?.id;
        const pageId = current?.pageId || activeViewId || fallback;
        if (!pageId) return;
        navigation.setCurrentAnchor({ pageId });
        navigation.gotoAnchor({ pageId }, { respectKnob: false });
    }, []);

    useWorldNav({ camera, registry, viewportRef });

    const dive = useCallback((targetId, sourceSurface) => {
        if (layerAnimatingRef.current) {
            log('dive:', targetId, '→ skipped (animating)');
            return;
        }
        if (sourceSurface) {
            const sourcePageId = sourceSurface.closest('[data-view-id]')?.dataset?.viewId;
            const selector = selectorFor(sourceSurface);
            if (sourcePageId && selector) navigation.setFocus(sourcePageId, selector);
        }
        const sourcePageId = sourceSurface?.closest('[data-view-id]')?.dataset?.viewId;
        const targetSelector = sourcePageId ? `[data-view-id="${sourcePageId}"]` : null;
        const diveTarget = targetSelector ? registry.getDiveTargetForSource(targetSelector) : null;
        if (diveTarget) {
            navigation.diveInto(targetSelector);
            setDiveDepth(navigation.stackDepth());
            const firstPageId = diveTarget.pages[0];
            if (firstPageId) {
                const entry = registry.getEntry(firstPageId);
                const newParent = entry?.parent || firstPageId;
                diveParentRef.current = newParent;
                setDiveParent(newParent);
            }
            return;
        }
        const entry = registry.diveTarget(targetId);
        const newParent = entry?.parent || targetId;
        diveParentRef.current = newParent;
        setDiveParent(newParent);
        navigation.dive(targetId);
        setDiveDepth(navigation.stackDepth());
    }, [navigation, registry]);

    const ascend = useCallback(() => {
        const didAscend = navigation.ascend();
        if (didAscend) {
            const topAnchor = navigation.getCurrentAnchor();
            const topEntry = topAnchor?.pageId ? registry.getEntry(topAnchor.pageId) : null;
            const parentForAnchor = topEntry?.parent ?? null;
            diveParentRef.current = parentForAnchor;
            setDiveParent(parentForAnchor);
            setDiveDepth(navigation.stackDepth());
        }
        return didAscend;
    }, [navigation, registry]);

    const pluginDiveRef = useRef(false);
    useEffect(() => {
        if (activePluginId && !pluginDiveRef.current) {
            log('pluginDive: open', activePluginId, '→ dive plugins-config');
            pluginDiveRef.current = true;
            const source = document.querySelector('[data-selected-surface][data-selected="true"]');
            dive('plugins-config', source);
        } else if (!activePluginId && pluginDiveRef.current) {
            log('pluginDive: close → ascend');
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
                    <${WorldViewport} camera=${camera} onViewChange=${switchView} navigation=${navigation} registry=${registry}>
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
                    <${MinimapContainer} camera=${camera} registry=${registry} viewportRef=${viewportRef} diveParent=${diveParent}
                        activePluginId=${activePluginId} diveDepth=${diveDepth} navigation=${navigation}
                        version=${appVersion} updateState=${updateState} isDevMode=${devEnabled} onAction=${handleSidebarAction} />
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

