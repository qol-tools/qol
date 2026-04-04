import { html } from '../lib/html.js';
import { useRef, useCallback, useEffect } from 'preact/hooks';
import { PaletteProvider, usePaletteContext } from '../palette/context.js';
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

    const registryRef = useRef(null);
    if (!registryRef.current) registryRef.current = createWorldRegistry(viewOrder);
    const registry = registryRef.current;

    const viewportRef = useRef(null);

    useEffect(() => {
        window.__worldCamera = camera;
        return () => { window.__worldCamera = null; };
    }, [camera]);

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
            const target = registry.cameraTargetForView(activeViewId, w, h);
            if (target) camera.panSmooth(target.x, target.y, 400);
        }
    }, [activeViewId, camera, registry]);

    useEffect(() => {
        const vp = viewportRef.current;
        const target = registry.cameraTargetForView(activeViewId, vp?.clientWidth || 800, vp?.clientHeight || 600);
        if (target) camera.panTo(target.x, target.y);
    }, []);

    useWorldNav({ camera, registry, viewportRef });

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
                />
                <div class="app-container">
                    <${WorldViewport} camera=${camera}>
                        <${RegionLabels} registry=${registry} />
                        ${renderWorldViews({
                            registry,
                            openPluginConfig,
                            openPluginUi,
                            syncStatus,
                            syncProviders,
                            onSyncStatusChange: setSyncStatus,
                            refreshSyncStatus,
                        })}
                    <//>
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

function AppKeyboardRouting({ activePluginId, activeViewId, camera, closePluginConfig, switchView, viewOrder }) {
    const palette = usePaletteContext();
    useAppKeyboardRouting({ activePluginId, activeViewId, camera, closePluginConfig, switchView, viewOrder, palette });
    return null;
}
