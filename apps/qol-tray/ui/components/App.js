import { html } from '../lib/html.js';
import { useRef, useCallback, useMemo } from 'preact/hooks';
import { SidebarNav } from './SidebarNav.js';
import { SidebarFooter } from './SidebarFooter.js';
import { PaletteProvider, usePaletteContext } from '../palette/context.js';
import { ModifierStateProvider } from '../hooks/modifier-state-context.js';
import { PluginConfigProvider } from '../views/plugin-config/context.js';
import { useApp } from './app/useApp.js';
import { useSidebarProvider } from './app/sidebar-context.js';
import { useAppKeyboardRouting } from './app/useAppKeyboardRouting.js';
import { ViewKeyboardProvider } from './app/view-keyboard-context.js';
import { renderMountedViews } from './app/views.js';
import { useScrollIntoView } from '../hooks/useScrollIntoView.js';
import { RecompileDissolve } from './RecompileDissolve.js';
import { PluginConfigView } from '../views/plugin-config/view.js';
import { GlobalToast } from './ApiErrorToast.js';
import { SelectionCursorOverlay } from './SelectionCursorOverlay.js';

export function App() {
    return html`<${PaletteProvider}><${AppShell} /><//>`;
}

function AppShell() {
    useScrollIntoView();
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
        mounted,
        updateState,
        handleSidebarAction,
        handleViewClick,
        worktrees,
        defaultWorktree,
        setDefaultWorktree,
        syncStatus,
        syncProviders,
        setSyncStatus,
        refreshSyncStatus,
    } = useApp({ onDissolve });

    const defaultItems = useMemo(() => {
        const LABELS = {
            plugins: 'Plugins', store: 'Store', hotkeys: 'Hotkeys',
            shortcuts: 'Shortcuts', 'task-runner': 'Task Runner',
            profile: 'Profile', logs: 'Logs', dev: 'Developer'
        };
        const DIVIDER_BEFORE = new Set(['hotkeys', 'profile', 'dev']);
        return viewOrder.flatMap(id => {
            const out = [];
            if (DIVIDER_BEFORE.has(id)) out.push({ type: 'divider', key: `divider:${id}` });
            out.push({
                type: 'item',
                key: id,
                id,
                label: LABELS[id] || id,
                active: id === activeViewId,
                onClick: () => handleViewClick(id),
                trailing: id === 'profile'
                    ? html`<span class="sidebar-status-dot" data-health=${syncStatus.health}
                        title=${syncStatus.health === 'healthy' ? 'Cloud sync healthy'
                            : syncStatus.health === 'attention' ? 'Cloud sync needs review'
                            : syncStatus.health === 'error' ? 'Cloud sync error'
                            : 'Cloud sync not configured'}></span>`
                    : null,
            });
            return out;
        });
    }, [viewOrder, activeViewId, handleViewClick, syncStatus.health]);

    const { SidebarContext, value: sidebarValue } = useSidebarProvider({ defaultItems, defaultHeader: null });

    return html`
        <${SidebarContext.Provider} value=${sidebarValue}>
        <${ModifierStateProvider}>
        <${PluginConfigProvider} pluginId=${activePluginId} mode=${activePluginMode}>
            <${ViewKeyboardProvider}>
                <${AppKeyboardRouting}
                    activePluginId=${activePluginId}
                    activeViewId=${activeViewId}
                    closePluginConfig=${closePluginConfig}
                    switchView=${switchView}
                    viewOrder=${viewOrder}
                />
                <div class="app-container">
                    <div class="app-main">
                        <aside id="sidebar"><${SidebarNav} /></aside>
                        <main id="content">
                            ${activePluginId && html`<${PluginConfigView} onClose=${closePluginConfig} />`}
                            ${renderMountedViews({
                                mounted,
                                activeViewId,
                                activePluginId,
                                openPluginConfig,
                                openPluginUi,
                                syncStatus,
                                syncProviders,
                                onSyncStatusChange: setSyncStatus,
                                refreshSyncStatus,
                            })}
                        </main>
                    </div>
                    <div class="app-footer">
                        <div id="sidebar-footer" class="app-footer-sidebar"><${SidebarFooter}
                            version=${appVersion} updateState=${updateState} isDevMode=${devEnabled} onAction=${handleSidebarAction}
                            worktrees=${worktrees} defaultWorktree=${defaultWorktree} setDefaultWorktree=${setDefaultWorktree} /></div>
                        <div id="content-footer" class="app-footer-content"></div>
                    </div>
                    <${SelectionCursorOverlay} />
                    <${RecompileDissolve} triggerRef=${dissolveRef} />
                    <${GlobalToast} />
                </div>
            <//>
        <//>
        <//>
        <//>
    `;
}

function AppKeyboardRouting({ activePluginId, activeViewId, closePluginConfig, switchView, viewOrder }) {
    const palette = usePaletteContext();
    useAppKeyboardRouting({ activePluginId, activeViewId, closePluginConfig, switchView, viewOrder, palette });
    return null;
}
