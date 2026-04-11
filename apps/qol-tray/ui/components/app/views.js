import { html } from '../../lib/html.js';
import { PluginsView } from '../../views/plugins-view.js';
import { PluginConfigSectionView } from '../../views/plugin-config/view.js';
import { StoreView } from '../../views/store-view.js';
import { HotkeysView, HotkeyEditorSubPage } from '../../views/hotkeys-view.js';
import { ShortcutsView, ShortcutEditorSubPage } from '../../views/shortcuts-view.js';
import { TaskRunnerView, ActionEditorSubPage } from '../../views/task-runner-view.js';
import { ProfileView } from '../../views/profile/view.js';
import { DevView } from '../../views/dev/view.js';
import { LogsView, LogDetailSubPage } from '../../views/logs-view.js';

export const VIEW_LABELS = {
    plugins: 'Plugins',
    store: 'Plugin Store',
    hotkeys: 'Hotkeys',
    shortcuts: 'Shortcuts',
    'task-runner': 'Task Runner',
    profile: 'Profile',
    logs: 'Logs',
    dev: 'Developer'
};

const BASE_ORDER = ['plugins', 'store', 'hotkeys', 'shortcuts', 'task-runner', 'profile', 'logs'];

export function buildViewOrder(devEnabled) {
    return devEnabled ? [...BASE_ORDER, 'dev'] : [...BASE_ORDER];
}

function WorldViewSlot({ entry, cameraLayer, children }) {
    if (!entry) return null;
    const visible = entry.layer === cameraLayer;
    const style = `left:${entry.x}px; top:${entry.y}px; width:${entry.width}px; height:${entry.height}px;${visible ? '' : ' display:none;'}`;
    return html`<div class="world-view-slot" data-view-id=${entry.id} data-layer=${entry.layer} style=${style}>${children}</div>`;
}

export function renderWorldViews({ registry, cameraLayer, openPluginConfig, openPluginUi, closePluginConfig, syncStatus, syncProviders, onSyncStatusChange, refreshSyncStatus }) {
    const layer = cameraLayer != null ? cameraLayer : 0;
    return html`
        <${WorldViewSlot} entry=${registry.getEntry('plugins')} cameraLayer=${layer}><${PluginsView} onOpenPluginConfig=${openPluginConfig} onOpenPluginUi=${openPluginUi} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('store')} cameraLayer=${layer}><${StoreView} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('hotkeys')} cameraLayer=${layer}><${HotkeysView} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('shortcuts')} cameraLayer=${layer}><${ShortcutsView} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('task-runner')} cameraLayer=${layer}><${TaskRunnerView} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('profile')} cameraLayer=${layer}><${ProfileView} syncStatus=${syncStatus}
            syncProviders=${syncProviders} onSyncStatusChange=${onSyncStatusChange} refreshSyncStatus=${refreshSyncStatus} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('logs')} cameraLayer=${layer}><${LogsView} active=${true} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('dev')} cameraLayer=${layer}><${DevView} /><//>
        ${registry.getAllEntries()
            .filter(e => e.layer === -1 && /^plugin-/.test(e.id) && e.id !== 'plugins-config')
            .map(e => {
                const m = e.id.match(/^(plugin-[^-]+(?:-[^-]+)*?)-([^-]+)$/);
                if (!m) return null;
                const [, pluginId, sectionId] = m;
                return html`<${WorldViewSlot} key=${e.id} entry=${e} cameraLayer=${layer}>
                    <${PluginConfigSectionView} pluginId=${pluginId} sectionId=${sectionId} onClose=${closePluginConfig} />
                <//>`;
            })}
        <${WorldViewSlot} entry=${registry.getEntry('hotkeys-editor')} cameraLayer=${layer}><${HotkeyEditorSubPage} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('shortcuts-editor')} cameraLayer=${layer}><${ShortcutEditorSubPage} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('logs-detail')} cameraLayer=${layer}><${LogDetailSubPage} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('task-runner-editor')} cameraLayer=${layer}><${ActionEditorSubPage} /><//>
    `;
}
