import { jsonRequest, readResponseText } from '../../api/client.js';

export function createPluginActionsController({
    state,
    discoveryController,
    getMergedPluginById,
    getActivePluginBuildState,
    closePluginMenu,
    onNeedsRender
}) {
    let reloadCooldownUntil = 0;

    function markReloadComplete() {
        reloadCooldownUntil = Date.now() + 1000;
    }

    function handleItemActivation() {
        const item = state.mergedList[state.selectedIndex];
        if (!item) return;
        closePluginMenu();
        if (getActivePluginBuildState(item)) return;

        if (item.status === 'linked') {
            void deleteLink(item.id);
            return;
        }

        if (item.path) {
            void quickLink(item.path, item.id);
            return;
        }

        showLinkInput();
    }

    function seedDiscoveredFromLinked(pluginId) {
        if (!pluginId) return;

        const linked = state.plugins.find(plugin => plugin.id === pluginId);
        const merged = state.mergedList.find(plugin => plugin.id === pluginId);
        const path = linked?.source || merged?.path || '';
        if (!path) return;

        const seeded = {
            id: pluginId,
            name: linked?.name || merged?.name || pluginId,
            path
        };

        const existingIndex = state.discovered.findIndex(plugin => plugin.id === pluginId);
        if (existingIndex >= 0) {
            state.discovered[existingIndex] = {
                ...state.discovered[existingIndex],
                ...seeded
            };
            return;
        }

        state.discovered.push(seeded);
    }

    async function quickLink(path, id) {
        if (state.linkingId) return;
        state.linkingId = id;
        onNeedsRender();

        try {
            const res = await fetch('/api/dev/links', {
                ...jsonRequest('POST', { path, id })
            });
            if (!res.ok) {
                console.error('Failed to link:', await readResponseText(res));
                return;
            }
            await triggerReload();
            await discoveryController.loadPlugins(true);
        } catch (error) {
            console.error('Failed to link:', error);
        } finally {
            state.linkingId = null;
            onNeedsRender();
        }
    }

    function showLinkInput() {
        state.showLinkInput = true;
        state.linkError = null;
        onNeedsRender();
    }

    function cancelLink() {
        state.showLinkInput = false;
        state.linkPath = '';
        state.linkError = null;
        onNeedsRender();
    }

    async function confirmLink() {
        if (!state.linkPath.trim()) {
            state.linkError = 'Enter a path';
            onNeedsRender();
            return;
        }

        try {
            const res = await fetch('/api/dev/links', {
                ...jsonRequest('POST', { path: state.linkPath })
            });
            if (!res.ok) {
                state.linkError = await readResponseText(res);
                onNeedsRender();
                return;
            }

            state.showLinkInput = false;
            state.linkPath = '';
            state.linkError = null;
            await triggerReload();
            await discoveryController.loadPlugins();
        } catch (error) {
            state.linkError = error.message;
            onNeedsRender();
        }
    }

    async function deleteLink(id) {
        if (state.linkingId) return;
        state.linkingId = id;
        seedDiscoveredFromLinked(id);
        onNeedsRender();

        try {
            const res = await fetch(`/api/dev/links/${id}`, { method: 'DELETE' });
            if (!res.ok) {
                console.error('Failed to delete link:', await readResponseText(res));
                return;
            }
            await triggerReload();
            await Promise.all([
                discoveryController.loadPlugins(true),
                discoveryController.refreshDiscoveryState()
            ]);
        } catch (error) {
            console.error('Failed to delete link:', error);
        } finally {
            state.linkingId = null;
            onNeedsRender();
        }
    }

    function getCurrentLinkedPlugin(pluginId) {
        return state.plugins.find(plugin => plugin.id === pluginId) || null;
    }

    function normalizePatternsInput(raw) {
        if (!raw) return [];
        return raw
            .split(',')
            .map(value => value.trim())
            .filter(Boolean);
    }

    async function savePluginLogControl(pluginId, control) {
        const res = await fetch(`/api/dev/log-controls/${encodeURIComponent(pluginId)}`, {
            ...jsonRequest('PUT', control)
        });
        if (res.ok) return;
        const message = await readResponseText(res);
        throw new Error(message || 'Failed to update plugin log control');
    }

    async function togglePluginLogs(pluginId) {
        const plugin = getCurrentLinkedPlugin(pluginId) || getMergedPluginById(pluginId);
        if (!plugin) return;

        try {
            await savePluginLogControl(pluginId, {
                muted: !plugin.logs_muted,
                suppress_patterns: Array.isArray(plugin.suppressed_log_patterns)
                    ? plugin.suppressed_log_patterns
                    : []
            });
            await Promise.all([
                discoveryController.loadPlugins(true),
                discoveryController.loadLogControls(true)
            ]);
        } catch (error) {
            state.error = error?.message || 'Failed to toggle plugin logs';
        }
        if (!state.linkingId) onNeedsRender();
    }

    async function editPluginLogFilters(pluginId) {
        const plugin = getCurrentLinkedPlugin(pluginId) || getMergedPluginById(pluginId);
        if (!plugin) return;

        const current = Array.isArray(plugin.suppressed_log_patterns)
            ? plugin.suppressed_log_patterns
            : [];
        const value = window.prompt(
            'Mute log lines containing these comma-separated substrings (leave empty to clear):',
            current.join(', ')
        );
        if (value === null) return;

        try {
            await savePluginLogControl(pluginId, {
                muted: !!plugin.logs_muted,
                suppress_patterns: normalizePatternsInput(value)
            });
            await Promise.all([
                discoveryController.loadPlugins(true),
                discoveryController.loadLogControls(true)
            ]);
        } catch (error) {
            state.error = error?.message || 'Failed to update plugin log filters';
        }
        if (!state.linkingId) onNeedsRender();
    }

    async function triggerReload() {
        const res = await fetch('/api/dev/reload', { method: 'POST' });
        if (res.ok || res.status === 409) return res;
        const message = await readResponseText(res);
        throw new Error(message || 'Failed to queue reload');
    }

    async function reloadPlugins() {
        if (state.building || Date.now() < reloadCooldownUntil) return;

        state.building = true;
        state.error = null;
        state.buildResults = null;
        state.buildProgress = {};
        onNeedsRender();

        try {
            const [reloadRes, discoverRes] = await Promise.all([
                fetch('/api/dev/reload', { method: 'POST' }),
                fetch('/api/dev/discover', { method: 'POST' })
            ]);

            if (reloadRes.status === 409) {
                state.building = false;
                return;
            }
            if (!reloadRes.ok) {
                state.building = false;
                state.error = await readResponseText(reloadRes) || 'Reload failed';
                return;
            }

            if (discoverRes.ok) {
                state.lastReload = new Date().toLocaleTimeString();
            }
            await discoveryController.loadPlugins();
        } catch (error) {
            state.building = false;
            state.error = error.message;
        } finally {
            onNeedsRender();
        }
    }

    return {
        cancelLink,
        confirmLink,
        deleteLink,
        editPluginLogFilters,
        handleItemActivation,
        markReloadComplete,
        quickLink,
        reloadPlugins,
        showLinkInput,
        togglePluginLogs
    };
}
