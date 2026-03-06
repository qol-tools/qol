import { html } from '../lib/html.js';
import { useEffect, useCallback, useRef } from 'preact/hooks';
import { useStateRef } from '../hooks/useStateRef.js';
import { useScrollIntoView } from '../hooks/useScrollIntoView.js';
import { useRefreshOnFocus } from '../hooks/useRefreshOnFocus.js';
import { useSSEDebounce } from '../hooks/useSSEDebounce.js';
import { useInstalling } from '../hooks/useInstalling.js';
import { useFeedback } from '../hooks/useFeedback.js';
import { useGridNav } from '../hooks/useGridNav.js';
import { useAsyncToken } from '../hooks/useAsyncToken.js';
import { withShiftVariants, dispatchKey } from '../utils/keys.js';
import { useFooterShortcuts } from '../hooks/useFooterShortcuts.js';
import { Feedback } from '../components/FeedbackPreact.js';
import { PageHeader } from '../components/PageHeader.js';
import {
    buildGhostPlugins,
    findPluginById,
    loadInstalledPlugins,
    uninstallInstalledPlugin,
    updateInstalledPlugin
} from './plugins/data.js';
import { UninstallConfirmModal } from './plugins/confirm-modal.js';
import { PluginsGrid } from './plugins/grid.js';

const SHORTCUTS = [
    { key: '←↑↓→', label: 'navigate' },
    { key: 'Enter', label: 'settings' },
    { key: 'u', label: 'update' },
    { key: 'd', label: 'delete' },
    { key: 'm', label: 'menu' }
];

export function PluginsView({ onOpenPluginConfig }) {
    const [plugins, setPlugins, pluginsRef] = useStateRef([]);
    const [selectedIndex, setSelectedIndex, selectedIndexRef] = useStateRef(0);
    const [contextMenuOpen, setContextMenuOpen, contextMenuOpenRef] = useStateRef(false);
    const [confirmPluginId, setConfirmPluginId, confirmPluginIdRef] = useStateRef(null);
    const [updating, setUpdating, updatingRef] = useStateRef(new Set());
    const { feedback, setFeedback, clearFeedback } = useFeedback();
    const { items: installingItems, has: isInstalling } = useInstalling();

    const [nextToken, isCurrentToken] = useAsyncToken();
    const latestRevisionRef = useRef(0);
    const restoredRef = useRef(false);

    useFooterShortcuts(SHORTCUTS);

    const refreshPlugins = useCallback(async (opts = {}) => {
        const { showErrorFeedback = false, restoreSelection = false, minRevision = 0 } = opts;
        const token = nextToken();
        try {
            const payload = await loadInstalledPlugins();
            if (!isCurrentToken(token)) return;
            if (payload.revision < minRevision || payload.revision < latestRevisionRef.current) return;
            latestRevisionRef.current = payload.revision;
            setPlugins(payload.plugins);
            setSelectedIndex(prev => {
                if (restoreSelection && !restoredRef.current) {
                    restoredRef.current = true;
                    const saved = parseInt(localStorage.getItem('plugins-selected-index') || '0', 10);
                    if (saved >= 0 && saved < payload.plugins.length) return saved;
                }
                return Math.min(prev, Math.max(0, payload.plugins.length - 1));
            });
        } catch (error) {
            if (!isCurrentToken(token)) return;
            if (showErrorFeedback) setFeedback('error', `Failed to load plugins: ${error.message}`);
        }
    }, [setFeedback]);

    useEffect(() => { refreshPlugins({ showErrorFeedback: true, restoreSelection: true }); }, [refreshPlugins]);

    useRefreshOnFocus(refreshPlugins);

    useSSEDebounce('plugins_changed', useCallback((event) => {
        const revision = Number.isInteger(event.revision) ? event.revision : latestRevisionRef.current;
        latestRevisionRef.current = Math.max(latestRevisionRef.current, revision);
        refreshPlugins({ minRevision: revision });
    }, [refreshPlugins]));

    useEffect(() => {
        if (!restoredRef.current) return;
        localStorage.setItem('plugins-selected-index', String(selectedIndex));
    }, [selectedIndex]);

    useScrollIntoView('.plugin-card.selected', [selectedIndex]);

    const updatePlugin = useCallback(async (pluginId) => {
        if (updatingRef.current.has(pluginId)) return;
        clearFeedback();
        setUpdating(prev => new Set(prev).add(pluginId));
        try {
            await updateInstalledPlugin(pluginId);
            setFeedback('success', `Updated ${pluginId}`);
        } catch (error) {
            setFeedback('error', `Failed to update ${pluginId}: ${error.message}`);
        } finally {
            setUpdating(prev => { const s = new Set(prev); s.delete(pluginId); return s; });
            refreshPlugins();
        }
    }, [clearFeedback, setFeedback, refreshPlugins]);

    const confirmUninstall = useCallback(async () => {
        const pluginId = confirmPluginIdRef.current;
        setConfirmPluginId(null);
        if (!pluginId) return;
        clearFeedback();
        try {
            await uninstallInstalledPlugin(pluginId);
            setFeedback('success', `Uninstalled ${pluginId}`);
            await refreshPlugins();
        } catch (error) {
            setFeedback('error', `Failed to uninstall ${pluginId}: ${error.message}`);
        }
    }, [clearFeedback, setFeedback, refreshPlugins]);

    const openSelected = useCallback(() => {
        const plugin = pluginsRef.current[selectedIndexRef.current];
        if (!plugin) return;
        if (plugin.loaded === false) {
            setFeedback('error', `Plugin ${plugin.name} is not loaded${plugin.load_error ? `: ${plugin.load_error}` : ''}`);
            return;
        }
        if (plugin.has_ui) {
            localStorage.setItem('plugins-selected-index', String(selectedIndexRef.current));
            if (onOpenPluginConfig) onOpenPluginConfig(plugin.id);
            return;
        }
        setFeedback('info', `No settings UI available for ${plugin.name}`);
    }, [setFeedback, onOpenPluginConfig]);

    const closeAllContextMenus = useCallback(() => setContextMenuOpen(false), []);

    const navigateInGrid = useGridNav('#plugins-grid .plugin-card:not(.ghost)', selectedIndexRef, setSelectedIndex);

    const handleKey = useCallback((e) => {
        if (confirmPluginIdRef.current !== null) {
            if (e.key === 'Escape') { e.preventDefault(); setConfirmPluginId(null); }
            else if (e.key === 'Enter') { e.preventDefault(); confirmUninstall(); }
            return;
        }
        if (contextMenuOpenRef.current) {
            e.preventDefault();
            if (e.key === 'Escape') { closeAllContextMenus(); return; }
            if (e.key === 'Enter') {
                const plugin = pluginsRef.current[selectedIndexRef.current];
                if (plugin) { closeAllContextMenus(); setConfirmPluginId(plugin.id); }
            }
            return;
        }
        dispatchKey(e, withShiftVariants({
            ArrowUp: () => navigateInGrid('up'),
            ArrowDown: () => navigateInGrid('down'),
            ArrowLeft: () => navigateInGrid('left'),
            ArrowRight: () => navigateInGrid('right'),
            Enter: openSelected,
            d: () => { const p = pluginsRef.current[selectedIndexRef.current]; if (p) setConfirmPluginId(p.id); },
            u: () => { const p = pluginsRef.current[selectedIndexRef.current]; if (p?.update_available) updatePlugin(p.id); },
            m: () => setContextMenuOpen(prev => !prev),
        }));
    }, [confirmUninstall, closeAllContextMenus, navigateInGrid, openSelected, updatePlugin]);

    const isBlocking = useCallback(() => confirmPluginIdRef.current !== null || contextMenuOpenRef.current, []);

    // Expose imperative handle for App keyboard routing
    PluginsView.handleKey = handleKey;
    PluginsView.isBlocking = isBlocking;

    const handleCardClick = useCallback((e, index, pluginId) => {
        const updateBtn = e.target.closest('.plugin-update:not([disabled])');
        if (updateBtn) { e.stopPropagation(); updatePlugin(pluginId); return; }

        const cogBtn = e.target.closest('.plugin-cog');
        if (cogBtn) { e.stopPropagation(); setSelectedIndex(index); setContextMenuOpen(prev => !prev); return; }

        const ctxUpdate = e.target.closest('.context-update');
        if (ctxUpdate) { e.stopPropagation(); closeAllContextMenus(); updatePlugin(pluginId); return; }

        const ctxDelete = e.target.closest('.context-delete');
        if (ctxDelete) { e.stopPropagation(); closeAllContextMenus(); setConfirmPluginId(pluginId); return; }

        if (index !== selectedIndexRef.current) setSelectedIndex(index);
        else openSelected();
    }, [updatePlugin, closeAllContextMenus, openSelected]);

    const handleBackdropClick = useCallback(() => {
        if (contextMenuOpen) closeAllContextMenus();
    }, [contextMenuOpen, closeAllContextMenus]);

    const ghostPlugins = buildGhostPlugins(plugins, installingItems);
    const confirmPlugin = findPluginById(plugins, confirmPluginId);

    return html`
        <div class="view-container" onClick=${handleBackdropClick}>
            <${PageHeader} title="Plugins" />
            <div class="view-body">
                <${Feedback} feedback=${feedback} />
                <${PluginsGrid}
                    plugins=${plugins}
                    ghostPlugins=${ghostPlugins}
                    selectedIndex=${selectedIndex}
                    contextMenuOpen=${contextMenuOpen}
                    updating=${updating}
                    onCardClick=${handleCardClick}
                />
            </div>
            <${UninstallConfirmModal}
                plugin=${confirmPlugin}
                pluginId=${confirmPluginId}
                onClose=${() => setConfirmPluginId(null)}
                onConfirm=${confirmUninstall}
            />
        </div>
    `;
}
