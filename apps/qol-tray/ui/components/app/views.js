import { html } from '../../lib/html.js';
import { PluginsView } from '../../views/plugins-view.js';
import { StoreView } from '../../views/store-view.js';
import { HotkeysView } from '../../views/hotkeys-view.js';
import { ShortcutsView } from '../../views/shortcuts-view.js';
import { TaskRunnerView } from '../../views/task-runner-view.js';
import { ProfileView } from '../../views/profile/view.js';
import { DevView } from '../../views/dev/view.js';
import { LogsView } from '../../views/logs-view.js';

export const VIEW_LABELS = {
    plugins: 'Plugins',
    store: 'Plugin Store',
    hotkeys: 'Hotkeys',
    shortcuts: 'Shortcuts',
    'task-runner': 'Task Runner',
    profile: 'Profile',
    logs: 'Logs',
    dev: 'Dev'
};

const BASE_ORDER = ['plugins', 'store', 'hotkeys', 'shortcuts', 'task-runner', 'profile', 'logs'];

export function buildViewOrder(devEnabled) {
    return devEnabled ? [...BASE_ORDER, 'dev'] : [...BASE_ORDER];
}

function WorldViewSlot({ entry, children }) {
    if (!entry) return null;
    const style = `left:${entry.x}px; top:${entry.y}px; width:${entry.width}px;`;
    return html`<div class="world-view-slot" data-view-id=${entry.id} style=${style}>${children}</div>`;
}

export function renderWorldViews({ registry, openPluginConfig, openPluginUi, syncStatus, syncProviders, onSyncStatusChange, refreshSyncStatus }) {
    return html`
        <${WorldViewSlot} entry=${registry.getEntry('plugins')}><${PluginsView} onOpenPluginConfig=${openPluginConfig} onOpenPluginUi=${openPluginUi} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('store')}><${StoreView} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('hotkeys')}><${HotkeysView} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('shortcuts')}><${ShortcutsView} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('task-runner')}><${TaskRunnerView} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('profile')}><${ProfileView} syncStatus=${syncStatus}
            syncProviders=${syncProviders} onSyncStatusChange=${onSyncStatusChange} refreshSyncStatus=${refreshSyncStatus} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('logs')}><${LogsView} active=${true} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('dev')}><${DevView} /><//>
    `;
}
