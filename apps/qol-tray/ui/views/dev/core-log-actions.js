import { jsonRequest, readResponseText } from '../../api/client.js';
import { setDebugEnabled } from '../../lib/debug.js';

export function createCoreLogActions({ state, discoveryController, onNeedsRender }) {
    return {
        toggleCoreLogs: id => toggleCoreLogs(state, discoveryController, onNeedsRender, id),
        editCoreLogFilters: id => editCoreLogFilters(state, discoveryController, onNeedsRender, id)
    };
}

async function toggleCoreLogs(state, discoveryController, onNeedsRender, sectionId) {
    const control = state.coreLogControls[sectionId] || {};
    const newMuted = !control.muted;
    try {
        await saveCoreLogControl(sectionId, {
            muted: newMuted,
            suppress_patterns: Array.isArray(control.suppress_patterns) ? control.suppress_patterns : []
        });
        await discoveryController.loadCoreLogControls(true);
        if (sectionId === 'frontend-debug') setDebugEnabled(!newMuted);
    } catch (error) {
        state.error = error?.message || 'Failed to toggle core logs';
    }
    onNeedsRender();
}

async function editCoreLogFilters(state, discoveryController, onNeedsRender, sectionId) {
    const control = state.coreLogControls[sectionId] || {};
    const current = Array.isArray(control.suppress_patterns) ? control.suppress_patterns : [];
    const value = window.prompt(
        'Mute log lines containing these comma-separated substrings (leave empty to clear):',
        current.join(', ')
    );
    if (value === null) return;
    try {
        await saveCoreLogControl(sectionId, {
            muted: !!control.muted,
            suppress_patterns: normalizePatternsInput(value)
        });
        await discoveryController.loadCoreLogControls(true);
    } catch (error) {
        state.error = error?.message || 'Failed to update core log filters';
    }
    onNeedsRender();
}

function normalizePatternsInput(raw) {
    if (!raw) return [];
    return raw.split(',').map(v => v.trim()).filter(Boolean);
}

async function saveCoreLogControl(sectionId, control) {
    const response = await fetch(`/api/dev/core-log-controls/${encodeURIComponent(sectionId)}`, {
        ...jsonRequest('PUT', control)
    });
    if (response.ok) return;
    const message = await readResponseText(response);
    throw new Error(message || 'Failed to update core log control');
}
