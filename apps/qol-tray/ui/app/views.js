import { html } from '../lib/html.js';
import { PluginsView } from '../views/plugins-view.js';
import { PluginConfigSectionView } from '../views/plugin-config/view.js';
import { StoreView } from '../views/store-view.js';
import { HotkeysView, HotkeyEditorSubPage } from '../views/hotkeys-view.js';
import { ShortcutsView, ShortcutEditorSubPage } from '../views/shortcuts-view.js';
import { TaskRunnerView, ActionEditorSubPage } from '../views/task-runner-view.js';
import { TestRunnerSubPage } from '../views/task-runner/test-runner-subpage.js';
import { ProfileView, BackupDetailSubPage } from '../views/profile/view.js';
import { DevView } from '../views/dev/view.js';
import { LogsView, LogDetailSubPage } from '../views/logs-view.js';
import { LogFiltersSubPage } from '../views/dev/log-filters-subpage.js';
import { UninstallConfirmSubPage } from '../views/plugins/uninstall-confirm-subpage.js';
import { PluginActionsSubPage } from '../views/plugins/plugin-actions-subpage.js';

// VIEW_LABELS: id → string OR { text, animation }.
// Plain string is the default (no animation). The object form opts a single
// view into a named label animation (see ANIMATIONS map in RegionLabels.js).
// Add new animations by extending that map, not by branching on id.
export const VIEW_LABELS = {
    plugins: 'Plugins',
    store: 'Plugin Store',
    hotkeys: 'Hotkeys',
    shortcuts: 'Shortcuts',
    'task-runner': 'Task Runner',
    profile: 'Profile',
    logs: 'Logs',
    dev: { text: 'Developer', animation: 'scramble' },
};

// Normalised accessor: always returns { text, animation }.
// `animation` is null when the entry is a bare string or has no animation field.
// Callers that only care about the display text can use `.text`.
export function getViewLabel(id) {
    const entry = VIEW_LABELS[id];
    if (entry == null) return { text: id, animation: null };
    if (typeof entry === 'string') return { text: entry, animation: null };
    return { text: entry.text || id, animation: entry.animation || null };
}

const BASE_ORDER = ['plugins', 'store', 'hotkeys', 'shortcuts', 'task-runner', 'profile', 'logs'];

export function buildViewOrder(devEnabled) {
    return devEnabled ? [...BASE_ORDER, 'dev'] : [...BASE_ORDER];
}

const WORLD_PAGES = [
    { id: 'plugins',           render: (ctx) => html`<${PluginsView} onOpenPluginConfig=${ctx.openPluginConfig} />` },
    { id: 'store',             render: () => html`<${StoreView} />` },
    { id: 'hotkeys',           render: () => html`<${HotkeysView} />` },
    { id: 'shortcuts',         render: () => html`<${ShortcutsView} />` },
    { id: 'task-runner',       render: () => html`<${TaskRunnerView} />` },
    { id: 'profile',           contentSized: true, render: (ctx) => html`<${ProfileView} syncStatus=${ctx.syncStatus} syncProviders=${ctx.syncProviders} onSyncStatusChange=${ctx.onSyncStatusChange} refreshSyncStatus=${ctx.refreshSyncStatus} />` },
    { id: 'logs',              render: () => html`<${LogsView} active=${true} />` },
    { id: 'dev',               devOnly: true, contentSized: true, render: () => html`<${DevView} />` },
    { id: 'hotkeys-editor',    render: () => html`<${HotkeyEditorSubPage} />` },
    { id: 'shortcuts-editor',  render: () => html`<${ShortcutEditorSubPage} />` },
    { id: 'logs-detail',       render: () => html`<${LogDetailSubPage} />` },
    { id: 'task-runner-editor', render: () => html`<${ActionEditorSubPage} />` },
    { id: 'task-runner-test-runner', render: () => html`<${TestRunnerSubPage} />` },
    { id: 'profile-backup-detail', render: () => html`<${BackupDetailSubPage} />` },
    { id: 'dev-log-filters',   devOnly: true, render: () => html`<${LogFiltersSubPage} />` },
    { id: 'plugins-uninstall-confirm', render: () => html`<${UninstallConfirmSubPage} />` },
    { id: 'plugins-actions',   render: () => html`<${PluginActionsSubPage} />` },
    { id: 'dev-plugin-actions', devOnly: true, render: () => html`<${PluginActionsSubPage} />` },
];

const PAGES_BY_ID = new Map(WORLD_PAGES.map(p => [p.id, p]));

export const CONTENT_SIZED_PAGES = new Set(WORLD_PAGES.filter(p => p.contentSized).map(p => p.id));

export function renderPageContent(pageId, ctx) {
    const page = PAGES_BY_ID.get(pageId);
    if (page) return page.devOnly && !ctx.devEnabled ? null : page.render(ctx);
    if (ctx.activePluginId && pageId.startsWith(`${ctx.activePluginId}-`)) {
        const sectionId = pageId.slice(ctx.activePluginId.length + 1);
        return html`<${PluginConfigSectionView} pluginId=${ctx.activePluginId} sectionId=${sectionId} onClose=${ctx.closePluginConfig} />`;
    }
    return null;
}

function WorldViewSlot({ entry, cameraLayer, confinedPages, diveDepth, onJumpTo, children }) {
    if (!entry) return null;
    const layerMatch = entry.layer === cameraLayer;
    const confined = confinedPages && confinedPages.length > 0;
    const ascending = entry.layer < 0 && (diveDepth ?? 0) === 0;
    const visible = layerMatch && !ascending && (!confined || confinedPages.includes(entry.id));
    const heightStyle = entry.contentSized ? '' : ` height:${entry.height}px;`;
    const style = `left:${entry.x}px; top:${entry.y}px; width:${entry.width}px;${heightStyle}${visible ? '' : ' display:none;'}`;
    const jumper = entry.layer === 0 && onJumpTo
        ? html`<button class="world-slot-jumper" tabindex="-1" aria-label=${`Jump to ${entry.id}`} onClick=${() => onJumpTo(entry.id)}></button>`
        : null;
    return html`<div class="world-view-slot" data-view-id=${entry.id} data-layer=${entry.layer} style=${style}>${jumper}${children}</div>`;
}

export function renderWorldViews(ctx) {
    const { registry, cameraLayer, confinedPages, diveDepth, activePluginId, onJumpTo } = ctx;
    const layer = cameraLayer != null ? cameraLayer : 0;
    const slotFor = (entry, content) => entry && content != null
        ? html`<${WorldViewSlot} key=${entry.id} entry=${entry} cameraLayer=${layer} confinedPages=${confinedPages} diveDepth=${diveDepth} onJumpTo=${onJumpTo}>${content}<//>`
        : null;

    return html`
        ${WORLD_PAGES.map(p => slotFor(registry.getEntry(p.id), renderPageContent(p.id, ctx)))}
        ${activePluginId && registry.getAllEntries()
            .filter(e => e.layer === -1 && e.id.startsWith(`${activePluginId}-`))
            .map(e => slotFor(e, renderPageContent(e.id, ctx)))}
    `;
}
