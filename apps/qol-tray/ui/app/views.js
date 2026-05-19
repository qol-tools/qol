import { html } from '../lib/html.js';
import { isSlotVisible, slotStyle } from '../lib/world-slot-style.js';
import { PluginsView } from '../views/plugins-view.js';
import { PluginConfigSectionView } from '../views/plugin-config/view.js';
import { StoreView } from '../views/store-view.js';
import { HotkeysView, HotkeyEditorSubPage, hotkeyEditorSlot } from '../views/hotkeys-view.js';
import { ShortcutsView, ShortcutEditorSubPage, shortcutEditorSlot } from '../views/shortcuts-view.js';
import { TaskRunnerView, ActionEditorSubPage, actionEditorSlot } from '../views/task-runner-view.js';
import { TestRunnerSubPage, testRunnerSlot } from '../views/task-runner/test-runner-subpage.js';
import { ProfileView, BackupDetailSubPage, prodBackupDetailConfig } from '../views/profile/view.js';
import { backupPreviewSlot } from '../views/profile/use-backups.js';
import { DevView } from '../views/dev/view.js';
import { LogsView, LogDetailSubPage, detailSlot as logDetailSlot } from '../views/logs-view.js';
import { LogFiltersSubPage, logFiltersSlot } from '../views/dev/log-filters-subpage.js';
import { GalleryShowcasePage } from '../views/dev/gallery-showcase-page.js';
import { GalleryLogRowDetailSubPage } from '../views/dev/gallery-log-row-detail-subpage.js';
import { GalleryBackupRowDetailSubPage } from '../views/dev/gallery-backup-row-detail-subpage.js';
import { GalleryHotkeyEditorSubPage } from '../views/dev/gallery-hotkey-editor-subpage.js';
import { GalleryShortcutEditorSubPage } from '../views/dev/gallery-shortcut-editor-subpage.js';
import { SHOWCASE_KEYS } from '../views/dev/components/ComponentsCatalog.js';
import { UninstallConfirmSubPage, uninstallConfirmSlot } from '../views/plugins/uninstall-confirm-subpage.js';
import { PluginActionsSubPage, pluginActionsSlot } from '../views/plugins/plugin-actions-subpage.js';

export { VIEW_LABELS, getViewLabel, resolveViewLabel } from './view-labels.js';

const BASE_ORDER = ['plugins', 'store', 'hotkeys', 'shortcuts', 'task-runner', 'profile', 'logs'];

export function buildViewOrder(devEnabled) {
    return devEnabled ? [...BASE_ORDER, 'dev'] : [...BASE_ORDER];
}

const WORLD_PAGES = [
    { id: 'plugins',           contentSized: true, render: (ctx) => html`<${PluginsView} onOpenPluginConfig=${ctx.openPluginConfig} />` },
    { id: 'store',             contentSized: true, render: () => html`<${StoreView} />` },
    { id: 'hotkeys',           contentSized: true, render: () => html`<${HotkeysView} />` },
    { id: 'shortcuts',         contentSized: true, render: () => html`<${ShortcutsView} />` },
    { id: 'task-runner',       contentSized: true, render: () => html`<${TaskRunnerView} />` },
    { id: 'profile',           contentSized: true, render: (ctx) => html`<${ProfileView} syncStatus=${ctx.syncStatus} syncProviders=${ctx.syncProviders} onSyncStatusChange=${ctx.onSyncStatusChange} refreshSyncStatus=${ctx.refreshSyncStatus} />` },
    { id: 'logs',              contentSized: true, render: () => html`<${LogsView} active=${true} />` },
    { id: 'dev',               devOnly: true, contentSized: true, render: () => html`<${DevView} />` },
    { id: 'hotkeys-editor',    render: () => html`<${HotkeyEditorSubPage} slot=${hotkeyEditorSlot} />` },
    { id: 'shortcuts-editor',  render: () => html`<${ShortcutEditorSubPage} slot=${shortcutEditorSlot} />` },
    { id: 'logs-detail',       render: () => html`<${LogDetailSubPage} slot=${logDetailSlot} />` },
    { id: 'task-runner-editor', render: () => html`<${ActionEditorSubPage} slot=${actionEditorSlot} />` },
    { id: 'task-runner-test-runner', render: () => html`<${TestRunnerSubPage} slot=${testRunnerSlot} />` },
    { id: 'profile-backup-detail', render: () => html`<${BackupDetailSubPage} slot=${backupPreviewSlot} config=${prodBackupDetailConfig} />` },
    { id: 'dev-log-filters',   devOnly: true, render: () => html`<${LogFiltersSubPage} slot=${logFiltersSlot} />` },
    { id: 'dev-gallery-log-row-detail', devOnly: true, render: () => html`<${GalleryLogRowDetailSubPage} />` },
    { id: 'dev-gallery-backup-row-detail', devOnly: true, render: () => html`<${GalleryBackupRowDetailSubPage} />` },
    { id: 'dev-gallery-hotkey-row-editor', devOnly: true, render: () => html`<${GalleryHotkeyEditorSubPage} />` },
    { id: 'dev-gallery-shortcut-row-editor', devOnly: true, render: () => html`<${GalleryShortcutEditorSubPage} />` },
    { id: 'plugins-uninstall-confirm', render: () => html`<${UninstallConfirmSubPage} slot=${uninstallConfirmSlot} />` },
    { id: 'plugins-actions',   render: () => html`<${PluginActionsSubPage} slot=${pluginActionsSlot} />` },
    { id: 'dev-plugin-actions', devOnly: true, render: () => html`<${PluginActionsSubPage} slot=${pluginActionsSlot} />` },
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
    if (pageId.startsWith('dev-gallery-') && ctx.devEnabled) {
        const key = pageId.slice('dev-gallery-'.length);
        if (SHOWCASE_KEYS.includes(key)) return html`<${GalleryShowcasePage} showcaseId=${key} />`;
    }
    return null;
}

function WorldViewSlot({ entry, cameraLayer, confinedPages, diveDepth, onJumpTo, children }) {
    if (!entry) return null;
    const visible = isSlotVisible(entry, cameraLayer, confinedPages ?? [], diveDepth);
    const style = slotStyle(entry, visible);
    const jumper = entry.layer === 0 && onJumpTo
        ? html`<button class="world-slot-jumper" tabindex="-1" aria-label=${`Jump to ${entry.id}`} onClick=${() => onJumpTo(entry.id)}></button>`
        : null;
    return html`<div class="world-view-slot" tabindex="-1" data-view-id=${entry.id} data-layer=${entry.layer} style=${style}>${jumper}${children}</div>`;
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
        ${ctx.devEnabled && SHOWCASE_KEYS.map(key => {
            const id = `dev-gallery-${key}`;
            return slotFor(registry.getEntry(id), renderPageContent(id, ctx));
        })}
    `;
}
