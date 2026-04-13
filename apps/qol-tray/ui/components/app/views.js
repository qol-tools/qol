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

function WorldViewSlot({ entry, cameraLayer, confinedPages, diveDepth, children }) {
    if (!entry) return null;
    const layerMatch = entry.layer === cameraLayer;
    const confined = confinedPages && confinedPages.length > 0;
    const ascending = entry.layer < 0 && (diveDepth ?? 0) === 0;
    const visible = layerMatch && !ascending && (!confined || confinedPages.includes(entry.id));
    const style = `left:${entry.x}px; top:${entry.y}px; width:${entry.width}px; height:${entry.height}px;${visible ? '' : ' display:none;'}`;
    return html`<div class="world-view-slot" data-view-id=${entry.id} data-layer=${entry.layer} style=${style}>${children}</div>`;
}

export function renderWorldViews({ registry, cameraLayer, confinedPages, diveDepth, activePluginId, openPluginConfig, openPluginUi, closePluginConfig, syncStatus, syncProviders, onSyncStatusChange, refreshSyncStatus }) {
    const layer = cameraLayer != null ? cameraLayer : 0;
    const cp = confinedPages;
    const dd = diveDepth;
    return html`
        <${WorldViewSlot} entry=${registry.getEntry('plugins')} cameraLayer=${layer} confinedPages=${cp} diveDepth=${dd}><${PluginsView} onOpenPluginConfig=${openPluginConfig} onOpenPluginUi=${openPluginUi} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('store')} cameraLayer=${layer} confinedPages=${cp} diveDepth=${dd}><${StoreView} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('hotkeys')} cameraLayer=${layer} confinedPages=${cp} diveDepth=${dd}><${HotkeysView} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('shortcuts')} cameraLayer=${layer} confinedPages=${cp} diveDepth=${dd}><${ShortcutsView} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('task-runner')} cameraLayer=${layer} confinedPages=${cp} diveDepth=${dd}><${TaskRunnerView} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('profile')} cameraLayer=${layer} confinedPages=${cp} diveDepth=${dd}><${ProfileView} syncStatus=${syncStatus}
            syncProviders=${syncProviders} onSyncStatusChange=${onSyncStatusChange} refreshSyncStatus=${refreshSyncStatus} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('logs')} cameraLayer=${layer} confinedPages=${cp} diveDepth=${dd}><${LogsView} active=${true} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('dev')} cameraLayer=${layer} confinedPages=${cp} diveDepth=${dd}><${DevView} /><//>
        ${activePluginId && registry.getAllEntries()
            .filter(e => e.layer === -1 && e.id.startsWith(`${activePluginId}-`))
            .map(e => {
                const sectionId = e.id.slice(activePluginId.length + 1);
                return html`<${WorldViewSlot} key=${e.id} entry=${e} cameraLayer=${layer} confinedPages=${cp} diveDepth=${dd}>
                    <${PluginConfigSectionView} pluginId=${activePluginId} sectionId=${sectionId} onClose=${closePluginConfig} />
                <//>`;
            })}
        <${WorldViewSlot} entry=${registry.getEntry('hotkeys-editor')} cameraLayer=${layer} confinedPages=${cp} diveDepth=${dd}><${HotkeyEditorSubPage} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('shortcuts-editor')} cameraLayer=${layer} confinedPages=${cp} diveDepth=${dd}><${ShortcutEditorSubPage} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('logs-detail')} cameraLayer=${layer} confinedPages=${cp} diveDepth=${dd}><${LogDetailSubPage} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('task-runner-editor')} cameraLayer=${layer} confinedPages=${cp} diveDepth=${dd}><${ActionEditorSubPage} /><//>
    `;
}
