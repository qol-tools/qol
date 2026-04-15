import { html } from '../lib/html.js';
import { useRef, useCallback, useEffect, useLayoutEffect, useState } from 'preact/hooks';
import { PaletteProvider, usePaletteContext } from '../palette/context.js';
import { createDebug, elLabel } from '../lib/debug.js';
import { prettyLabel } from '../auto-config/heuristics.js';
import { createNavigation, selectorFor, animateTransition } from '../lib/world-navigation.js';
import { getWorldSettings } from '../lib/world-settings.js';

const log = createDebug('qol:app');
import { ModifierStateProvider } from '../lib/hooks/modifier-state-context.js';
import { PluginConfigProvider } from '../views/plugin-config/context.js';
import { useApp } from '../app/useApp.js';
import { useAppKeyboardRouting } from '../app/useAppKeyboardRouting.js';
import { ViewKeyboardProvider } from '../app/view-keyboard-context.js';
import { buildViewOrder, renderWorldViews } from '../app/views.js';
import { RecompileDissolve } from '../lib/components/RecompileDissolve.js';
import { GlobalToast } from './ApiErrorToast.js';
import { SelectionCursorOverlay } from '../lib/components/SelectionCursorOverlay.js';
import { CommandPalette } from './CommandPalette.js';
import { createCamera } from '../lib/world-camera.js';
import { createWorldRegistry } from '../lib/world-registry.js';
import { pluginTraitOverride } from '../lib/plugin-trait-overrides.js';
import { WorldViewport } from './shell/WorldViewport.js';
import { MinimapContainer } from './shell/Minimap.js';
import { RegionLabels } from './shell/RegionLabels.js';
import { useWorldNav } from '../app/WorldNav.js';

function registerStaticDiveTargets(registry) {
    const PAGE_WIDTH = 1280;
    const PAGE_HEIGHT = 900;
    const staticTargets = [
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

const PLUGIN_PAGE_WIDTH = 1280;
const PLUGIN_PAGE_HEIGHT = 900;
const PLUGIN_PAGE_STRIDE = 10000;

async function fetchInstalledPlugins() {
    const res = await fetch('/api/installed');
    if (!res.ok) return [];
    const payload = await res.json();
    if (Array.isArray(payload)) return payload;
    if (Array.isArray(payload?.plugins)) return payload.plugins;
    return [];
}

async function fetchPluginContract(pluginId) {
    const res = await fetch(`/api/plugins/${pluginId}/config-form`);
    if (!res.ok) return { sections: [], traits: null };
    const form = await res.json();
    const traits = (form?.traits && typeof form.traits === 'object') ? form.traits : null;
    const sections = (form?.sections || []).filter(s => s.fields?.length);
    if (sections.length > 0) return { sections, traits };
    if (form?.fields?.length > 0) return { sections: [{ id: '_root', label: form.title || '' }], traits };
    return { sections: [], traits };
}

function registerPluginDiveTarget(registry, plugin, sections, traits, pluginsEntry, pluginIndex) {
    const N = Math.max(1, sections.length);
    const yOffset = pluginIndex * PLUGIN_PAGE_STRIDE;
    const claim = {
        x: pluginsEntry.x,
        y: pluginsEntry.y + yOffset,
        width: (N - 1) * PLUGIN_PAGE_STRIDE + PLUGIN_PAGE_WIDTH,
        height: PLUGIN_PAGE_HEIGHT,
        layer: pluginsEntry.layer - 1,
    };
    const pageIds = [];
    for (let i = 0; i < N; i++) {
        const section = sections[i];
        const sectionId = section?.id || 'config';
        const pageId = `${plugin.id}-${sectionId}`;
        registry.addEntry({
            id: pageId,
            x: claim.x + i * PLUGIN_PAGE_STRIDE,
            y: claim.y,
            width: PLUGIN_PAGE_WIDTH,
            height: PLUGIN_PAGE_HEIGHT,
            layer: claim.layer,
            label: section?.label || prettyLabel(sectionId),
        });
        pageIds.push(pageId);
    }
    const effectiveTraits = traits || pluginTraitOverride(plugin.id);
    registry.addDiveTarget({
        sourceSelector: `[data-plugin-id="${plugin.id}"]`,
        claim,
        pages: pageIds,
        ...(effectiveTraits ? { traits: effectiveTraits } : {}),
    });
}

async function registerAllPluginDiveTargets(registry, registered, isCancelled, onPlaceholdersReady) {
    const plugins = await fetchInstalledPlugins();
    if (isCancelled()) { log('diveTargets: cancelled'); return; }
    if (!plugins.length) { log('diveTargets: no plugins'); return; }
    const pluginsEntry = registry.getEntry('plugins');
    if (!pluginsEntry) { log('diveTargets: no plugins entry in registry'); return; }

    const pending = plugins.filter(p => !registered.has(p.id));
    pending.forEach((plugin, i) => {
        registerPluginDiveTarget(registry, plugin, [], null, pluginsEntry, registered.size + i);
        registered.add(plugin.id);
    });
    log('diveTargets: pre-registered', pending.length, 'placeholders');
    onPlaceholdersReady?.();

    await Promise.all(pending.map(async (plugin, i) => {
        if (isCancelled()) return;
        const { sections, traits } = await fetchPluginContract(plugin.id);
        if (isCancelled()) return;
        const pluginIndex = registered.size - pending.length + i;
        registerPluginDiveTarget(registry, plugin, sections, traits, pluginsEntry, pluginIndex);
    }));
    log('diveTargets: resolved', registered.size, 'plugins:', [...registered].join(', '));
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
    const [targetsVersion, setTargetsVersion] = useState(0);
    const diveParentRef = useRef(null);
    const layerAnimatingRef = useRef(false);
    const diveTargetsRegisteredRef = useRef(new Set());

    useEffect(() => {
        let cancelled = false;
        let retryTimer;
        function attempt(delay) {
            registerAllPluginDiveTargets(
                registry,
                diveTargetsRegisteredRef.current,
                () => cancelled,
                () => { if (!cancelled) setTargetsVersion(v => v + 1); },
            )
                .then(() => {
                    if (!cancelled) {
                        navigationRef.current?.refreshCurrentDive?.();
                        setTargetsVersion(v => v + 1);
                    }
                })
                .catch(err => {
                    log('diveTargets: registration failed, retry in', delay, 'ms');
                    if (!cancelled) retryTimer = setTimeout(() => attempt(Math.min(delay * 2, 5000)), delay);
                });
        }
        attempt(500);
        return () => { cancelled = true; clearTimeout(retryTimer); };
    }, [registry]);

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

    const [activeAnchorId, setActiveAnchorId] = useState(() => navigation.getCurrentAnchor()?.pageId || null);
    useEffect(() => {
        const unsub = navigation.subscribeAnchor((anchor) => setActiveAnchorId(anchor?.pageId || null));
        return unsub;
    }, [navigation]);
    const activeSectionId = (activeAnchorId && activePluginId && activeAnchorId.startsWith(`${activePluginId}-`))
        ? activeAnchorId.slice(activePluginId.length + 1)
        : null;

    const prevViewRef = useRef(activeViewId);
    const prevViewOrderRef = useRef(viewOrder);
    useEffect(() => {
        const prevOrder = prevViewOrderRef.current;
        prevViewOrderRef.current = viewOrder;
        const viewChanged = prevViewRef.current !== activeViewId;
        const becameAvailable = !viewChanged && prevOrder !== viewOrder
            && viewOrder.includes(activeViewId) && !prevOrder.includes(activeViewId);
        if (!viewChanged && !becameAvailable) return;
        prevViewRef.current = activeViewId;
        log('viewChange:', activeViewId, viewChanged ? '→ switched' : '→ became available');
        navigation.setCurrentAnchor({ pageId: activeViewId });
        navigation.gotoAnchor({ pageId: activeViewId }, { respectKnob: true, instant: becameAvailable });
    }, [activeViewId, viewOrder, navigation]);

    useLayoutEffect(() => {
        const worldEl = document.getElementById('world');
        if (worldEl) camera.setWorldElement(worldEl);
        if (!activeViewId) return;
        navigation.setCurrentAnchor({ pageId: activeViewId });
        navigation.gotoAnchor({ pageId: activeViewId }, { respectKnob: false, instant: true });
    }, []);

    useWorldNav({ camera, registry, viewportRef });

    const diveViaSelector = useCallback((selector) => {
        if (layerAnimatingRef.current) return false;
        const target = navigation.diveInto(selector);
        if (!target) return false;
        setDiveDepth(navigation.stackDepth());
        const firstPageId = target.pages[0];
        if (firstPageId) {
            const entry = registry.getEntry(firstPageId);
            const newParent = entry?.parent || firstPageId;
            diveParentRef.current = newParent;
            setDiveParent(newParent);
        }
        return true;
    }, [navigation, registry]);

    const dive = useCallback((targetId, sourceSurface) => {
        if (layerAnimatingRef.current) return;
        if (sourceSurface) {
            const sourcePageId = sourceSurface.closest('[data-view-id]')?.dataset?.viewId;
            const selector = selectorFor(sourceSurface);
            if (sourcePageId && selector) navigation.setFocus(sourcePageId, selector);
        }
        const pluginId = sourceSurface?.dataset?.pluginId
            || sourceSurface?.closest?.('[data-plugin-id]')?.dataset?.pluginId;
        if (pluginId && diveViaSelector(`[data-plugin-id="${pluginId}"]`)) return;
        const parentPageId = sourceSurface?.closest?.('[data-view-id]')?.dataset?.viewId;
        if (parentPageId && diveViaSelector(`[data-view-id="${parentPageId}"]`)) return;
        log('dive:', targetId, '→ no DiveTarget matched');
    }, [navigation, diveViaSelector]);

    const ascend = useCallback(() => {
        const didAscend = navigation.ascend();
        if (!didAscend) return false;
        const topAnchor = navigation.getCurrentAnchor();
        const topEntry = topAnchor?.pageId ? registry.getEntry(topAnchor.pageId) : null;
        const parentForAnchor = topEntry?.parent ?? null;
        diveParentRef.current = parentForAnchor;
        setDiveParent(parentForAnchor);
        setDiveDepth(navigation.stackDepth());
        return true;
    }, [navigation, registry]);

    const diveRef = useRef(false);
    const [hiddenUntilDive, setHiddenUntilDive] = useState(() => {
        try { return !!window.localStorage?.getItem('qoltray.activePlugin'); } catch { return false; }
    });
    useEffect(() => {
        if (activePluginId && !diveRef.current) {
            if (diveViaSelector(`[data-plugin-id="${activePluginId}"]`)) {
                diveRef.current = true;
                setHiddenUntilDive(false);
            }
        } else if (!activePluginId && diveRef.current) {
            diveRef.current = false;
            ascend();
        }
        if (!activePluginId && hiddenUntilDive) setHiddenUntilDive(false);
    }, [activePluginId, diveViaSelector, ascend, targetsVersion, hiddenUntilDive]);
    useLayoutEffect(() => {
        if (!hiddenUntilDive) {
            document.body.classList.remove('qol-bootstrapping-dive');
            return undefined;
        }
        document.body.classList.add('qol-bootstrapping-dive');
        const failsafe = setTimeout(() => setHiddenUntilDive(false), 2000);
        return () => {
            document.body.classList.remove('qol-bootstrapping-dive');
            clearTimeout(failsafe);
        };
    }, [hiddenUntilDive]);

    return html`
        <${ModifierStateProvider}>
        <${PluginConfigProvider} pluginId=${activePluginId} mode=${activePluginMode} activeSectionId=${activeSectionId}>
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
                    navigation=${navigation}
                    registry=${registry}
                />
                <div class="app-container">
                    <${WorldViewport} camera=${camera} onViewChange=${switchView} navigation=${navigation} registry=${registry}>
                        <${RegionLabels} registry=${registry} cameraLayer=${cameraLayer} navigation=${navigation} diveDepth=${diveDepth} />
                        ${renderWorldViews({
                            registry,
                            cameraLayer,
                            confinedPages: navigation.getConfinedPages(),
                            diveDepth,
                            activePluginId,
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

function AppKeyboardRouting({ activePluginId, activeViewId, camera, closePluginConfig, switchView, viewOrder, dive, ascend, navigation, registry }) {
    const palette = usePaletteContext();
    useAppKeyboardRouting({ activePluginId, activeViewId, camera, closePluginConfig, switchView, viewOrder, palette, dive, ascend, navigation, registry });
    return null;
}

