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
import { Modal } from '../components/ModalPreact.js';
import { apiJson } from '../api/client.js';
import { parseInstalledPayload } from '../utils/plugins.js';

const PLACEHOLDER_SVG = 'data:image/svg+xml,' + encodeURIComponent(
    '<svg xmlns="http://www.w3.org/2000/svg" width="300" height="200">' +
    '<rect fill="#2f3644" width="300" height="200"/>' +
    '<text fill="#67748f" x="50%" y="50%" text-anchor="middle" dy=".3em" font-family="sans-serif" font-size="14">No Cover</text>' +
    '</svg>'
);

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

    // Load + refresh
    const refreshPlugins = useCallback(async (opts = {}) => {
        const { showErrorFeedback = false, restoreSelection = false, minRevision = 0 } = opts;
        const token = nextToken();
        try {
            const payload = parseInstalledPayload(await apiJson('/api/installed'));
            if (!isCurrentToken(token)) return;
            if (payload.revision < minRevision || payload.revision < latestRevisionRef.current) return;
            latestRevisionRef.current = payload.revision;
            const sorted = payload.plugins.sort((a, b) => a.name.localeCompare(b.name));
            setPlugins(sorted);
            setSelectedIndex(prev => {
                if (restoreSelection && !restoredRef.current) {
                    restoredRef.current = true;
                    const saved = parseInt(localStorage.getItem('plugins-selected-index') || '0', 10);
                    if (saved >= 0 && saved < sorted.length) return saved;
                }
                return Math.min(prev, Math.max(0, sorted.length - 1));
            });
        } catch (error) {
            if (!isCurrentToken(token)) return;
            if (showErrorFeedback) setFeedback('error', `Failed to load plugins: ${error.message}`);
        }
    }, [setFeedback]);

    // Initial load
    useEffect(() => { refreshPlugins({ showErrorFeedback: true, restoreSelection: true }); }, [refreshPlugins]);

    useRefreshOnFocus(refreshPlugins);

    useSSEDebounce('plugins_changed', useCallback((event) => {
        const revision = Number.isInteger(event.revision) ? event.revision : latestRevisionRef.current;
        latestRevisionRef.current = Math.max(latestRevisionRef.current, revision);
        refreshPlugins({ minRevision: revision });
    }, [refreshPlugins]));

    // Save selection (skip until initial restore completes to avoid overwriting stored value)
    useEffect(() => {
        if (!restoredRef.current) return;
        localStorage.setItem('plugins-selected-index', String(selectedIndex));
    }, [selectedIndex]);

    useScrollIntoView('.plugin-card.selected', [selectedIndex]);

    // Actions — all use refs so callbacks are stable
    const updatePlugin = useCallback(async (pluginId) => {
        if (updatingRef.current.has(pluginId)) return;
        clearFeedback();
        setUpdating(prev => new Set(prev).add(pluginId));
        try {
            const result = await apiJson(`/api/update/${pluginId}`, { method: 'POST' });
            if (!result.success) throw new Error(result.message);
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
            const result = await apiJson(`/api/uninstall/${pluginId}`, { method: 'POST' });
            if (!result.success) throw new Error(result.message);
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

    // Keyboard — stable: reads all mutable state via refs
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

    // Click handlers — stable via refs
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

    // Ghost plugins (installing but not yet in list)
    const installedIds = new Set(plugins.map(p => p.id));
    const ghostPlugins = installingItems.filter(p => !installedIds.has(p.id));

    const confirmPlugin = confirmPluginId ? plugins.find(p => p.id === confirmPluginId) : null;

    return html`
        <div class="view-container" onClick=${handleBackdropClick}>
            <header><h1>Plugins</h1></header>
            <div class="view-body">
                <${Feedback} feedback=${feedback} />
                <div id="plugins-grid" class="plugin-grid grid-cards grid-cards--zoom">
                    ${plugins.length === 0 && ghostPlugins.length === 0 && html`
                        <div class="empty">No plugins installed. Press Tab to open the store.</div>
                    `}
                    ${ghostPlugins.map(p => html`
                        <div key=${'ghost-' + p.id} class="plugin-card ghost">
                            <span class="refresh-btn spinning"></span>
                            <div class="plugin-name">${p.name}</div>
                        </div>
                    `)}
                    ${plugins.map((plugin, index) => html`
                        <div key=${plugin.id}
                             class="plugin-card ${plugin.has_ui ? '' : 'no-ui'} ${plugin.update_available ? 'has-update' : ''} ${plugin.loaded === false ? 'not-loaded' : ''} ${index === selectedIndex ? 'selected' : ''}"
                             data-index="${index}" data-plugin-id="${plugin.id}"
                             onClick=${(e) => handleCardClick(e, index, plugin.id)}>
                            <img src=${plugin.has_cover ? `/api/cover/${plugin.id}` : PLACEHOLDER_SVG}
                                 alt=${plugin.name}
                                 onError=${(e) => { e.target.src = PLACEHOLDER_SVG; }} />
                            <div class="plugin-name">${plugin.name}</div>
                            ${plugin.loaded === false && html`<div class="plugin-load-state">Not loaded</div>`}
                            ${plugin.update_available && html`
                                <button class="plugin-update ${updating.has(plugin.id) ? 'updating' : ''}"
                                        aria-label="Update plugin"
                                        disabled=${updating.has(plugin.id)}>
                                    ${updating.has(plugin.id)
                                        ? html`<span class="refresh-btn spinning update-spinner"></span>`
                                        : `↑ ${plugin.available_version}`}
                                </button>
                            `}
                            <button class="plugin-cog" aria-label="Plugin options">
                                <svg class="plugin-cog-icon" viewBox="0 0 12 20" fill="currentColor" aria-hidden="true" focusable="false">
                                    <circle cx="6" cy="3.5" r="1.8"></circle>
                                    <circle cx="6" cy="10" r="1.8"></circle>
                                    <circle cx="6" cy="16.5" r="1.8"></circle>
                                </svg>
                            </button>
                            <div class="plugin-context-menu ${contextMenuOpen && index === selectedIndex ? 'open' : ''}">
                                ${plugin.update_available && html`<button class="context-update">Update</button>`}
                                <button class="context-delete">Delete</button>
                            </div>
                        </div>
                    `)}
                </div>
            </div>
            <${Modal} open=${confirmPluginId !== null} onClose=${() => setConfirmPluginId(null)} className="confirm-modal">
                <div class="confirm-modal-content">
                    <h3>Delete "${confirmPlugin?.name || confirmPluginId}"?</h3>
                    <p>This will uninstall the plugin and remove all its data.</p>
                    <div class="confirm-modal-buttons">
                        <button class="btn btn-ghost confirm-cancel" onClick=${() => setConfirmPluginId(null)}>Cancel (Esc)</button>
                        <button class="btn btn-danger confirm-delete" onClick=${confirmUninstall}>Delete (Enter)</button>
                    </div>
                </div>
            <//>
        </div>
    `;
}
