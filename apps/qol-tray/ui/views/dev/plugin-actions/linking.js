import { jsonRequest, readResponseText } from '../../../api/client.js';

export function createLinkingActions({
    state,
    discoveryController,
    getActivePluginBuildState,
    closePluginMenu,
    onNeedsRender,
    triggerReload
}) {
    function handleItemActivation() {
        const item = state.mergedList[state.selectedIndex];
        if (!item) {
            return;
        }

        closePluginMenu();
        if (getActivePluginBuildState(item)) {
            return;
        }

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
            const response = await fetch('/api/dev/links', {
                ...jsonRequest('POST', { path: state.linkPath })
            });
            if (!response.ok) {
                state.linkError = await readResponseText(response);
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

    async function quickLink(path, id) {
        if (state.linkingId) {
            return;
        }

        state.linkingId = id;
        onNeedsRender();

        try {
            const response = await fetch('/api/dev/links', {
                ...jsonRequest('POST', { path, id })
            });
            if (!response.ok) {
                console.error('Failed to link:', await readResponseText(response));
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

    async function deleteLink(id) {
        if (state.linkingId) {
            return;
        }

        state.linkingId = id;
        seedDiscoveredFromLinked(state, id);
        onNeedsRender();

        try {
            const response = await fetch(`/api/dev/links/${id}`, { method: 'DELETE' });
            if (!response.ok) {
                console.error('Failed to delete link:', await readResponseText(response));
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

    return {
        cancelLink,
        confirmLink,
        deleteLink,
        handleItemActivation,
        quickLink,
        showLinkInput
    };
}

function seedDiscoveredFromLinked(state, pluginId) {
    if (!pluginId) {
        return;
    }

    const linked = state.plugins.find(plugin => plugin.id === pluginId);
    const merged = state.mergedList.find(plugin => plugin.id === pluginId);
    const path = linked?.source || merged?.path || '';
    if (!path) {
        return;
    }

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
