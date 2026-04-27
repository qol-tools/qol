import { jsonRequest, readResponseText } from '../../api/client.js';
import { diveViaSelector } from '../../lib/world-navigation-singleton.js';
import { logFiltersSlot } from './log-filters-subpage.js';
import { findFrontendLogSection } from './frontend-log-sections.js';

export function createCoreLogActions({ state, discoveryController, onNeedsRender }) {
    return {
        toggleCoreLogs: id => toggleCoreLogs(state, discoveryController, onNeedsRender, id),
        editCoreLogFilters: id => editCoreLogFilters(state, discoveryController, onNeedsRender, id)
    };
}

async function toggleCoreLogs(state, discoveryController, onNeedsRender, sectionId) {
    const control = state.coreLogControls[sectionId] || {};
    const newMuted = !control.muted;
    // Frontend-only sections (see frontend-log-sections.js) persist client-side;
    // the backend rejects these section names, so don't round-trip through the API.
    const frontendSection = findFrontendLogSection(sectionId);
    if (frontendSection) {
        frontendSection.setMuted(newMuted);
        state.coreLogControls = {
            ...state.coreLogControls,
            [sectionId]: { muted: newMuted, suppress_patterns: [] }
        };
        onNeedsRender();
        return;
    }
    try {
        await saveCoreLogControl(sectionId, {
            muted: newMuted,
            suppress_patterns: Array.isArray(control.suppress_patterns) ? control.suppress_patterns : []
        });
        await discoveryController.loadCoreLogControls(true);
    } catch (error) {
        state.error = error?.message || 'Failed to toggle core logs';
    }
    onNeedsRender();
}

function editCoreLogFilters(state, discoveryController, onNeedsRender, sectionId) {
    const control = state.coreLogControls[sectionId] || {};
    const current = Array.isArray(control.suppress_patterns) ? control.suppress_patterns : [];
    logFiltersSlot.set({
        scope: 'core',
        pluginId: null,
        sectionId,
        label: sectionId,
        current,
        save: async (patterns) => {
            try {
                await saveCoreLogControl(sectionId, {
                    muted: !!control.muted,
                    suppress_patterns: patterns,
                });
                await discoveryController.loadCoreLogControls(true);
            } catch (error) {
                state.error = error?.message || 'Failed to update core log filters';
            }
            onNeedsRender();
        },
    });
    diveViaSelector('[data-view-id="dev"]');
}

async function saveCoreLogControl(sectionId, control) {
    const response = await fetch(`/api/dev/core-log-controls/${encodeURIComponent(sectionId)}`, {
        ...jsonRequest('PUT', control)
    });
    if (response.ok) return;
    const message = await readResponseText(response);
    throw new Error(message || 'Failed to update core log control');
}
