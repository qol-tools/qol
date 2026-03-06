import { html } from '../../lib/html.js';
import { PluginsView } from '../../views/plugins-view.js';
import { StoreView } from '../../views/store-view.js';
import { HotkeysView } from '../../views/hotkeys-view.js';
import { TaskRunnerView } from '../../views/task-runner-view.js';
import { DevView } from '../../views/dev/view.js';

export const VIEW_MAP = {
    plugins: PluginsView,
    store: StoreView,
    hotkeys: HotkeysView,
    'task-runner': TaskRunnerView,
    dev: DevView
};

const BASE_ORDER = ['plugins', 'store', 'hotkeys', 'task-runner'];

export function buildViewOrder(devEnabled) {
    if (!devEnabled) {
        return [...BASE_ORDER];
    }

    return [...BASE_ORDER, 'dev'];
}

export function renderMountedViews({
    mounted,
    activeViewId,
    activePluginId,
    openPluginConfig
}) {
    return html`
        ${mounted.has('plugins') && html`<div style=${slotStyle('plugins', activeViewId, activePluginId)}><${PluginsView} onOpenPluginConfig=${openPluginConfig} /></div>`}
        ${mounted.has('store') && html`<div style=${slotStyle('store', activeViewId, activePluginId)}><${StoreView} /></div>`}
        ${mounted.has('hotkeys') && html`<div style=${slotStyle('hotkeys', activeViewId, activePluginId)}><${HotkeysView} /></div>`}
        ${mounted.has('task-runner') && html`<div style=${slotStyle('task-runner', activeViewId, activePluginId)}><${TaskRunnerView} /></div>`}
        ${mounted.has('dev') && html`<div style=${slotStyle('dev', activeViewId, activePluginId)}><${DevView} /></div>`}
    `;
}

function slotStyle(id, activeViewId, activePluginId) {
    if (activeViewId === id && !activePluginId) {
        return 'flex:1;min-height:0;display:flex;flex-direction:column';
    }

    return 'display:none';
}
