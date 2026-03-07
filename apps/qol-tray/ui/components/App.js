import { html } from '../lib/html.js';
import { SidebarNav } from './SidebarNav.js';
import { SidebarFooter } from './SidebarFooter.js';
import { PaletteProvider } from '../palette/context.js';
import { useApp } from './app/useApp.js';
import { renderMountedViews } from './app/views.js';

export function App() {
    return html`<${PaletteProvider}><${AppShell} /><//>`;
}

function AppShell() {
    const { devEnabled, appVersion, viewOrder, activeViewId, activePluginId, openPluginConfig, closePluginConfig, mounted, updateState, handleSidebarAction, handleViewClick, worktrees, defaultWorktree, setDefaultWorktree } = useApp();
    return html`
        <div class="app-container">
            <div class="app-main">
                <aside id="sidebar"><${SidebarNav} activeViewId=${activeViewId} viewOrder=${viewOrder}
                    pluginOpen=${!!activePluginId} onViewClick=${handleViewClick} onBack=${closePluginConfig} /></aside>
                <main id="content" class=${activePluginId ? 'has-plugin-iframe' : ''}>
                    ${activePluginId && html`<iframe src="/plugins/${activePluginId}/" class="plugin-iframe"></iframe>`}
                    ${renderMountedViews({ mounted, activeViewId, activePluginId, openPluginConfig })}
                </main>
            </div>
            <div class="app-footer">
                <div id="sidebar-footer" class="app-footer-sidebar"><${SidebarFooter}
                    version=${appVersion} updateState=${updateState} isDevMode=${devEnabled} onAction=${handleSidebarAction}
                    worktrees=${worktrees} defaultWorktree=${defaultWorktree} setDefaultWorktree=${setDefaultWorktree} /></div>
                <div id="content-footer" class="app-footer-content"></div>
            </div>
        </div>
    `;
}
