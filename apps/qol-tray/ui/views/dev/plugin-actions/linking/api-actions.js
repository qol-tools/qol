import { jsonRequest, readResponseText } from '../../../../api/client.js';
import { seedDiscoveredFromLinked } from './discovered-seeding.js';

export function createLinkingApiActions({ state, discoveryController, onNeedsRender, triggerReload, linkInputState }) {
    return {
        confirmLink: () => confirmLink(linkInputState, triggerReload, discoveryController),
        quickLink: (path, id) => quickLink(state, onNeedsRender, path, id, triggerReload, discoveryController),
        deleteLink: id => deleteLink(state, onNeedsRender, id, triggerReload, discoveryController)
    };
}

async function confirmLink(linkInputState, triggerReload, discoveryController) {
    const path = linkInputState.readLinkPath();
    if (!path) { return; }
    try {
        const response = await postLink({ path });
        if (!response.ok) { linkInputState.failLink(await readResponseText(response)); return; }
        const pluginId = (await readResponseText(response)) || undefined;
        linkInputState.clearLinkInput();
        await triggerReload(pluginId);
        await discoveryController.loadPlugins();
    } catch (error) {
        linkInputState.failLink(error.message);
    }
}

async function quickLink(state, onNeedsRender, path, id, triggerReload, discoveryController) {
    await runWithLinkingId(state, onNeedsRender, id, null, async () => {
        try {
            const response = await postLink({ path, id });
            if (!response.ok) { console.error('Failed to link:', await readResponseText(response)); return; }
            await triggerReload(id);
            await discoveryController.loadPlugins(true);
        } catch (error) {
            console.error('Failed to link:', error);
        }
    });
}

async function deleteLink(state, onNeedsRender, id, triggerReload, discoveryController) {
    await runWithLinkingId(state, onNeedsRender, id, () => seedDiscoveredFromLinked(state, id), async () => {
        try {
            const response = await fetch(`/api/dev/links/${id}`, { method: 'DELETE' });
            if (!response.ok) { console.error('Failed to delete link:', await readResponseText(response)); return; }
            await Promise.all([discoveryController.loadPlugins(true), discoveryController.refreshDiscoveryState()]);
        } catch (error) {
            console.error('Failed to delete link:', error);
        }
    });
}

async function runWithLinkingId(state, onNeedsRender, id, beforeStart, task) {
    if (state.linkingId) { return; }
    state.linkingId = id;
    if (typeof beforeStart === 'function') { beforeStart(); }
    onNeedsRender(true);
    try {
        if (typeof task === 'function') { await task(); }
    } finally {
        state.linkingId = null;
        onNeedsRender(true);
    }
}

async function postLink(payload) {
    return fetch('/api/dev/links', { ...jsonRequest('POST', payload) });
}
