const API_BASE = '/api/task-runner';

export { API_BASE };

export async function loadTaskRunnerData() {
    const res = await fetch(`${API_BASE}/config`);
    if (!res.ok) {
        return emptyTaskRunnerData();
    }

    const config = await res.json();
    const actions = config.actions || {};
    return {
        actions,
        actionIds: Object.keys(actions)
    };
}

export async function persistTaskRunnerConfig(actions) {
    await fetch(`${API_BASE}/config`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ actions })
    });
}

export function createEditModalState(actions, actionId = null) {
    const action = actionId ? actions[actionId] : null;
    return {
        actionId: actionId || '',
        isNew: !actionId,
        name: action?.name || '',
        description: action?.description || '',
        command: action?.command || '',
        timeout: action?.timeout || 60
    };
}

export function buildSavedActions(actions, modal) {
    const actionId = normalizedActionId(modal.actionId);
    return {
        actionId,
        actions: {
            ...actions,
            [actionId]: actionFromModal(modal)
        }
    };
}

export function removeSelectedAction(actions, actionIds, index) {
    const actionId = actionIds[index];
    const next = { ...actions };
    delete next[actionId];
    return next;
}

export function nextSelectedIndex(actionIds, currentIndex) {
    return Math.min(currentIndex, Math.max(0, actionIds.length - 1));
}

export function extractParams(command) {
    const matches = command.match(/\{\{(\w+)\}\}/g) || [];
    return [...new Set(matches.map(m => m.slice(2, -2)))];
}

export function buildApiExample(actions, actionIds) {
    const actionId = actionIds[0] || 'my-action';
    const params = actions[actionId] ? extractParams(actions[actionId].command) : ['param1'];
    const body = {
        action: actionId,
        params: paramsObject(params)
    };

    return {
        actionId,
        params,
        json: JSON.stringify(body, null, 2)
    };
}

export async function runTaskActionTest(actionId, params) {
    const res = await fetch(`${API_BASE}/execute`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ action: actionId, params })
    });

    return res.json();
}

function emptyTaskRunnerData() {
    return {
        actions: {},
        actionIds: []
    };
}

function normalizedActionId(actionId) {
    return actionId.trim().toLowerCase().replace(/[^a-z0-9-]/g, '-');
}

function actionFromModal(modal) {
    return {
        name: modal.name.trim(),
        description: modal.description.trim(),
        command: modal.command.trim(),
        timeout: modal.timeout
    };
}

function paramsObject(params) {
    if (params.length === 0) {
        return {};
    }

    return params.reduce((acc, param) => ({ ...acc, [param]: '...' }), {});
}
