import { html } from '../lib/html.js';
import { useRef, useCallback, useEffect, useLayoutEffect, useMemo, useState } from 'preact/hooks';
import { PaletteProvider, usePaletteContext } from '../palette/context.js';
import { createDebug, elLabel } from '../lib/debug.js';
import { prettyLabel } from '../auto-config/heuristics.js';
import { createNavigation, selectorFor, animateTransition } from '../lib/world-navigation.js';
import { getWorldSettings, subscribeWorldSettings } from '../lib/world-settings.js';

const log = createDebug('qol:app');
import { ModifierStateProvider } from '../lib/hooks/modifier-state-context.js';
import { PluginConfigProvider } from '../views/plugin-config/context.js';
import { useApp } from '../app/useApp.js';
import { useAppKeyboardRouting } from '../app/useAppKeyboardRouting.js';
import { ViewKeyboardProvider } from '../app/view-keyboard-context.js';
import { buildViewOrder, renderPageContent, renderWorldViews, CONTENT_SIZED_PAGES } from '../app/views.js';
import { RecompileDissolve } from '../lib/components/RecompileDissolve.js';
import { GlobalToast } from './ApiErrorToast.js';
import { SelectionCursorOverlay } from '../lib/components/SelectionCursorOverlay.js';
import { CommandPalette } from './CommandPalette.js';
import { createCamera } from '../lib/world-camera.js';
import { createWorldRegistry } from '../lib/world-registry.js';
import { boundsOfEntries, maxEntryExtent, withPadding } from '../lib/world-geometry.js';
import { pageMode } from '../lib/peripheral-geometry.js';
import { pluginTraitOverride } from '../lib/plugin-trait-overrides.js';
import { WorldViewport } from './shell/WorldViewport.js';
import { MinimapContainer } from './shell/Minimap.js';
import { RegionLabels } from './shell/RegionLabels.js';
import { useWorldNav } from '../app/WorldNav.js';

const PAGE_STRIDE = 10000;
const SLOT_GAP_FACTOR = 0.85;

function computeBaseScale(zoom, threshold) {
    return Math.max(1, Math.pow(threshold / zoom, 0.85));
}

function applySlotScales(worldEl, registry, camera, baseScale) {
    const slots = worldEl.querySelectorAll('.world-view-slot');
    if (baseScale === 1) {
        for (const s of slots) s.style.removeProperty('--slot-scale');
        return;
    }
    const z = Math.max(camera.zoom, 0.05);
    const vpCenterX = camera.x + window.innerWidth / (2 * z);
    const vpCenterY = camera.y + window.innerHeight / (2 * z);
    const falloff = (PAGE_STRIDE * z) * 2.5;
    const centerFloor = 0.75;
    for (const slot of slots) {
        const entry = registry.getEntry(slot.dataset.viewId);
        if (!entry) continue;
        const cx = entry.x + entry.width / 2;
        const cy = entry.y + entry.height / 2;
        const d = Math.hypot(cx - vpCenterX, cy - vpCenterY) * z;
        const t = 1 - Math.min(d / falloff, 1);
        const proximity = centerFloor + (1 - centerFloor) * (t * t * (3 - 2 * t));
        const maxSlotScale = (PAGE_STRIDE * SLOT_GAP_FACTOR) / Math.max(entry.width, 1);
        const slotScale = Math.min(1 + (baseScale - 1) * proximity, maxSlotScale);
        slot.style.setProperty('--slot-scale', slotScale.toFixed(3));
    }
}

function measuredLayer0Entries(worldEl, entries, registry) {
    const slots = worldEl.querySelectorAll('.world-view-slot[data-layer="0"]');
    const heightById = new Map();
    for (const el of slots) {
        const entry = registry.getEntry(el.dataset.viewId);
        if (entry) heightById.set(entry.id, el.offsetHeight);
    }
    return entries.map(e => ({ ...e, height: Math.max(e.height, heightById.get(e.id) || 0) }));
}

function computeGroundConfinement(registry, viewOrder) {
    const entries = registry.getEntriesForLayer(0);
    if (!entries.length) return undefined;
    const rect = boundsOfEntries(entries);
    const { padX, padY } = maxEntryExtent(entries);
    return { bounds: { ...withPadding(rect, padX, padY), layer: 0 }, pages: viewOrder };
}

function registerStaticDiveTargets(registry) {
    const PAGE_WIDTH = 1280;
    const PAGE_HEIGHT = 900;
    const staticTargets = [
        { parentId: 'hotkeys', subId: 'hotkeys-editor' },
        { parentId: 'shortcuts', subId: 'shortcuts-editor' },
        { parentId: 'logs', subId: 'logs-detail' },
        { parentId: 'task-runner', subId: 'task-runner-editor' },
        { parentId: 'profile', subId: 'profile-backup-detail' },
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
    const inner = {
        x: pluginsEntry.x,
        y: pluginsEntry.y + yOffset,
        width: (N - 1) * PLUGIN_PAGE_STRIDE + PLUGIN_PAGE_WIDTH,
        height: PLUGIN_PAGE_HEIGHT,
        layer: pluginsEntry.layer - 1,
    };
    const claim = withPadding(inner, PLUGIN_PAGE_WIDTH, PLUGIN_PAGE_HEIGHT);
    const pageIds = [];
    for (let i = 0; i < N; i++) {
        const section = sections[i];
        const sectionId = section?.id || 'config';
        const pageId = `${plugin.id}-${sectionId}`;
        registry.addEntry({
            id: pageId,
            x: inner.x + i * PLUGIN_PAGE_STRIDE,
            y: inner.y,
            width: PLUGIN_PAGE_WIDTH,
            height: PLUGIN_PAGE_HEIGHT,
            layer: inner.layer,
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
        const reg = createWorldRegistry(buildViewOrder(true), {}, { contentSizedIds: CONTENT_SIZED_PAGES });
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
        if (navigationRef.current) {
            navigationRef.current.setGroundPages(viewOrder);
            setTargetsVersion(v => v + 1);
        }
    }, [viewOrder, registry]);

    useEffect(() => {
        const worldEl = document.getElementById('world');
        if (!worldEl) return undefined;
        const recomputeBounds = () => {
            const entries = registry.getEntriesForLayer(0);
            if (!entries.length) return;
            const measured = measuredLayer0Entries(worldEl, entries, registry);
            const rect = boundsOfEntries(measured);
            const { padX, padY } = maxEntryExtent(entries);
            camera.setBounds({ ...withPadding(rect, padX, padY), layer: 0 });
        };
        const ro = new ResizeObserver(recomputeBounds);
        const slots = worldEl.querySelectorAll('.world-view-slot[data-layer="0"]');
        for (const el of slots) ro.observe(el);
        recomputeBounds();
        return () => ro.disconnect();
    }, [camera, registry, targetsVersion]);

    const navigationRef = useRef(null);
    if (!navigationRef.current) {
        const groundConfinement = computeGroundConfinement(registry, viewOrder);
        navigationRef.current = createNavigation({
            registry,
            camera,
            getSettings: getWorldSettings,
            groundConfinement,
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
        const s = getWorldSettings();
        navigation.gotoAnchor(
            { pageId: activeViewId },
            {
                respectKnob: true,
                instant: becameAvailable,
                useFocusMemory: false,
                resetZoom: viewChanged && s.resetZoomOnNav ? s.defaultZoom : null,
            },
        );
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
            }
        } else if (!activePluginId && diveRef.current) {
            diveRef.current = false;
            ascend();
        }
        if (!activePluginId && hiddenUntilDive) setHiddenUntilDive(false);
    }, [activePluginId, diveViaSelector, ascend, targetsVersion, hiddenUntilDive]);
    useEffect(() => {
        if (hiddenUntilDive && cameraLayer !== 0) setHiddenUntilDive(false);
    }, [cameraLayer, hiddenUntilDive]);
    useLayoutEffect(() => {
        document.documentElement.classList.toggle('qol-bootstrapping-dive', hiddenUntilDive);
        if (!hiddenUntilDive) return undefined;
        const failsafe = setTimeout(() => setHiddenUntilDive(false), 2000);
        return () => clearTimeout(failsafe);
    }, [hiddenUntilDive]);

    const onJumpTo = useCallback((pageId) => {
        if (pageId === activeViewId) {
            const s = getWorldSettings();
            navigation.gotoAnchor({ pageId }, { respectKnob: false, resetZoom: s.defaultZoom });
            return;
        }
        switchView(pageId);
    }, [activeViewId, switchView, navigation]);

    useEffect(() => {
        const syncMode = () => {
            const worldEl = document.getElementById('world');
            if (!worldEl) return;
            const { ghostThreshold, uiScaleOnZoomOut } = getWorldSettings();
            const zoom = Math.max(camera.zoom, 0.05);
            const baseScale = uiScaleOnZoomOut ? computeBaseScale(zoom, ghostThreshold) : 1;
            worldEl.setAttribute('data-page-mode', pageMode(camera.zoom, ghostThreshold));
            document.documentElement.style.setProperty('--zoom', zoom.toFixed(4));
            applySlotScales(worldEl, registry, camera, baseScale);
        };
        const rafId = requestAnimationFrame(syncMode);
        const unsub = camera.subscribe(syncMode);
        const unsubSettings = subscribeWorldSettings(syncMode);
        return () => {
            cancelAnimationFrame(rafId);
            unsub();
            unsubSettings();
        };
    }, [camera, registry]);

    const renderCtx = useMemo(() => ({
        activePluginId,
        openPluginConfig,
        openPluginUi,
        closePluginConfig,
        syncStatus,
        syncProviders,
        onSyncStatusChange: setSyncStatus,
        refreshSyncStatus,
        devEnabled,
        onJumpTo,
    }), [activePluginId, openPluginConfig, openPluginUi, closePluginConfig,
        syncStatus, syncProviders, setSyncStatus, refreshSyncStatus, devEnabled, onJumpTo]);
    const renderPage = useCallback((pageId) => renderPageContent(pageId, renderCtx), [renderCtx]);

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
                    <${WorldViewport} camera=${camera} onViewChange=${switchView} navigation=${navigation} registry=${registry} renderPage=${renderPage}>
                        ${hiddenUntilDive ? null : html`
                            <${RegionLabels} registry=${registry} cameraLayer=${cameraLayer} navigation=${navigation} diveDepth=${diveDepth} />
                            ${renderWorldViews({ ...renderCtx, registry, cameraLayer, confinedPages: navigation.getConfinedPages(), diveDepth })}
                        `}
                    <//>
                    <${CommandPalette} camera=${camera} navigation=${navigation} />
                    <${MinimapContainer} camera=${camera} registry=${registry} viewportRef=${viewportRef} diveParent=${diveParent}
                        activePluginId=${activePluginId} diveDepth=${diveDepth} navigation=${navigation}
                        version=${appVersion} updateState=${updateState} isDevMode=${devEnabled} onAction=${handleSidebarAction}
                        worktrees=${worktrees} defaultWorktree=${defaultWorktree} setDefaultWorktree=${setDefaultWorktree} />
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

