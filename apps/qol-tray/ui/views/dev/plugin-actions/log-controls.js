import { jsonRequest, readResponseText } from '../../../api/client.js';

export function createLogControlActions({ state, discoveryController, getMergedPluginById, onNeedsRender }) {
    const ctx = { state, discoveryController, getMergedPluginById, onNeedsRender };
    return {
        togglePluginLogs: id => togglePluginLogs(ctx, id),
        editPluginLogFilters: id => editPluginLogFilters(ctx, id)
    };
}

async function togglePluginLogs(ctx, pluginId) {
    const plugin = getCurrentPlugin(ctx.state, ctx.getMergedPluginById, pluginId);
    if (!plugin) return;
    try {
        await savePluginLogControl(pluginId, {
            muted: !plugin.logs_muted,
            suppress_patterns: Array.isArray(plugin.suppressed_log_patterns) ? plugin.suppressed_log_patterns : []
        });
        await refreshLogState(ctx.discoveryController);
    } catch (error) {
        ctx.state.error = error?.message || 'Failed to toggle plugin logs';
    }
    if (!ctx.state.linkingId) ctx.onNeedsRender();
}

async function editPluginLogFilters(ctx, pluginId) {
    const plugin = getCurrentPlugin(ctx.state, ctx.getMergedPluginById, pluginId);
    if (!plugin) return;
    const current = Array.isArray(plugin.suppressed_log_patterns) ? plugin.suppressed_log_patterns : [];
    const value = window.prompt(
        'Mute log lines containing these comma-separated substrings (leave empty to clear):',
        current.join(', ')
    );
    if (value === null) return;
    try {
        await savePluginLogControl(pluginId, { muted: !!plugin.logs_muted, suppress_patterns: normalizePatternsInput(value) });
        await refreshLogState(ctx.discoveryController);
    } catch (error) {
        ctx.state.error = error?.message || 'Failed to update plugin log filters';
    }
    if (!ctx.state.linkingId) ctx.onNeedsRender();
}

function getCurrentPlugin(state, getMergedPluginById, pluginId) {
    return state.plugins.find(plugin => plugin.id === pluginId)
        || getMergedPluginById(pluginId);
}

function normalizePatternsInput(raw) {
    if (!raw) return [];
    return raw.split(',').map(value => value.trim()).filter(Boolean);
}

async function savePluginLogControl(pluginId, control) {
    const response = await fetch(`/api/dev/log-controls/${encodeURIComponent(pluginId)}`, {
        ...jsonRequest('PUT', control)
    });
    if (response.ok) return;
    const message = await readResponseText(response);
    throw new Error(message || 'Failed to update plugin log control');
}

async function refreshLogState(discoveryController) {
    await Promise.all([
        discoveryController.loadPlugins(true),
        discoveryController.loadLogControls(true)
    ]);
}
