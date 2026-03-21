import { html } from '../../lib/html.js';
import { useLayoutEffect, useRef } from 'preact/hooks';
import { PluginsView } from '../../views/plugins-view.js';
import { StoreView } from '../../views/store-view.js';
import { HotkeysView } from '../../views/hotkeys-view.js';
import { ShortcutsView } from '../../views/shortcuts-view.js';
import { TaskRunnerView } from '../../views/task-runner-view.js';
import { DevView } from '../../views/dev/view.js';
import { LogsView } from '../../views/logs-view.js';

export const VIEW_LABELS = {
    plugins: 'Plugins',
    store: 'Plugin Store',
    hotkeys: 'Hotkeys',
    shortcuts: 'Shortcuts',
    'task-runner': 'Task Runner',
    logs: 'Logs',
    dev: 'Dev'
};

const BASE_ORDER = ['plugins', 'store', 'hotkeys', 'shortcuts', 'task-runner', 'logs'];

export function buildViewOrder(devEnabled) {
    return devEnabled ? [...BASE_ORDER, 'dev'] : [...BASE_ORDER];
}

function ViewSlot({ active, children }) {
    const ref = useRef(null);
    useLayoutEffect(() => {
        if (!active || !ref.current) return;
        const el = ref.current;
        el.style.animation = 'none';
        void el.offsetWidth;
        el.style.animation = '';
    }, [active]);
    const style = active
        ? 'flex:1;min-height:0;display:flex;flex-direction:column'
        : 'display:none';
    return html`<div class="view-slot" ref=${ref} style=${style}>${children}</div>`;
}

export function renderMountedViews({
    mounted,
    activeViewId,
    activePluginId,
    openPluginConfig,
    openPluginUi
}) {
    const active = (id) => activeViewId === id && !activePluginId;
    return html`
        ${mounted.has('plugins') && html`<${ViewSlot} active=${active('plugins')}><${PluginsView} onOpenPluginConfig=${openPluginConfig} onOpenPluginUi=${openPluginUi} /><//>`}
        ${mounted.has('store') && html`<${ViewSlot} active=${active('store')}><${StoreView} /><//>`}
        ${mounted.has('hotkeys') && html`<${ViewSlot} active=${active('hotkeys')}><${HotkeysView} /><//>`}
        ${mounted.has('shortcuts') && html`<${ViewSlot} active=${active('shortcuts')}><${ShortcutsView} /><//>`}
        ${mounted.has('task-runner') && html`<${ViewSlot} active=${active('task-runner')}><${TaskRunnerView} /><//>`}
        ${mounted.has('logs') && html`<${ViewSlot} active=${active('logs')}><${LogsView} active=${active('logs')} /><//>`}
        ${mounted.has('dev') && html`<${ViewSlot} active=${active('dev')}><${DevView} /><//>`}
    `;
}
