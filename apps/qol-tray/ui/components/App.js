import { html } from '../lib/html.js';
import { useRef, useCallback } from 'preact/hooks';
import { SidebarNav } from './SidebarNav.js';
import { SidebarFooter } from './SidebarFooter.js';
import { PaletteProvider, usePaletteContext } from '../palette/context.js';
import { ModifierStateProvider } from '../hooks/modifier-state-context.js';
import { PluginConfigProvider } from '../views/plugin-config/context.js';
import { useApp } from './app/useApp.js';
import { useAppKeyboardRouting } from './app/useAppKeyboardRouting.js';
import { ViewKeyboardProvider } from './app/view-keyboard-context.js';
import { renderMountedViews } from './app/views.js';
import { useScrollFollow } from '../hooks/useScrollIntoView.js';
import { RecompileDissolve } from './RecompileDissolve.js';
import { PluginConfigView } from '../views/plugin-config/view.js';
import { GlobalToast } from './ApiErrorToast.js';
import { SelectionCursorOverlay } from './SelectionCursorOverlay.js';

export function App() {
    return html`<${PaletteProvider}><${AppShell} /><//>`;
}

function AppShell() {
    useScrollFollow();
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
    return html`
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
                        <aside id="sidebar"><${SidebarNav} activeViewId=${activeViewId} viewOrder=${viewOrder}
                            pluginOpen=${!!activePluginId} onViewClick=${handleViewClick}
                            onBack=${closePluginConfig} profileSyncHealth=${syncStatus.health} /></aside>
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
    `;
}

function AppKeyboardRouting({ activePluginId, activeViewId, closePluginConfig, switchView, viewOrder }) {
    const palette = usePaletteContext();
    useAppKeyboardRouting({ activePluginId, activeViewId, closePluginConfig, switchView, viewOrder, palette });
    return null;
}
