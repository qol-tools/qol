export const id = 'task-runner';

const API_BASE = '/api/task-runner';
const CSS_ID = 'task-runner-css';

function loadStyles() {
    if (document.getElementById(CSS_ID)) return;
    const link = document.createElement('link');
    link.id = CSS_ID;
    link.rel = 'stylesheet';
    link.href = '/features/task-runner/style.css';
    document.head.appendChild(link);
}

const state = {
    actions: {},
    actionIds: [],
    selectedIndex: 0,
    editModalOpen: false,
    editingActionId: null,
    testingActionId: null,
    testParams: {},
    testResult: null,
    testRunning: false
};

let container = null;

export function render(containerEl) {
    loadStyles();
    container = containerEl;
    container.innerHTML = `
        <div class="view-container">
            <header>
                <h1>Task Runner</h1>
                <p>HTTP API for browser extensions to run local commands</p>
            </header>
            <div id="actions-list" class="actions-list"></div>
            <div id="api-usage" class="api-usage"></div>
            <footer class="help">
                ↑↓ navigate &bull; <kbd>a</kbd> add action
            </footer>
        </div>
    `;
    loadActions();
}

async function loadActions() {
    try {
        const res = await fetch(`${API_BASE}/config`);
        if (res.ok) {
            const config = await res.json();
            state.actions = config.actions || {};
            state.actionIds = Object.keys(state.actions);
        }
    } catch (e) {
        console.error('Failed to load actions:', e);
    }
    renderActions();
    renderApiUsage();
}

function renderActions() {
    const listEl = document.getElementById('actions-list');
    if (!listEl) return;

    listEl.removeEventListener('click', handleListClick);
    listEl.addEventListener('click', handleListClick);

    if (state.actionIds.length === 0) {
        listEl.innerHTML = `
            <div class="empty">
                No actions configured. Press <kbd>a</kbd> to add one.
            </div>
        `;
        return;
    }

    listEl.innerHTML = state.actionIds.map((actionId, index) => {
        const action = state.actions[actionId];
        const isSelected = index === state.selectedIndex;
        const isTesting = state.testingActionId === actionId;
        const params = extractParams(action.command);

        return `
            <div class="action-card ${isSelected ? 'selected' : ''} ${isTesting ? 'testing' : ''}" data-index="${index}" data-id="${actionId}">
                <div class="action-header">
                    <span class="action-id">${actionId}</span>
                    ${isSelected ? '<span class="action-hints"><kbd>Enter</kbd> edit <kbd>t</kbd> test <kbd>d</kbd> delete</span>' : ''}
                </div>
                <div class="action-name">${action.name}</div>
                ${action.description ? `<div class="action-desc">${action.description}</div>` : ''}
                <div class="action-command">$ ${escapeHtml(action.command)}</div>
                ${params.length > 0 ? `<div class="action-params">Parameters: ${params.map(p => `<code>{{${p}}}</code>`).join(', ')}</div>` : ''}
                ${isTesting ? renderTestPanel(actionId, action) : ''}
            </div>
        `;
    }).join('');

    if (state.testingActionId) {
        setupTestInputs();
    }
}

function renderTestPanel(actionId, action) {
    const params = extractParams(action.command);

    return `
        <div class="test-panel">
            <div class="test-panel-header">
                <span>Test: ${actionId}</span>
                <span class="test-hints"><kbd>Enter</kbd> run <kbd>Esc</kbd> close</span>
            </div>
            ${params.length > 0 ? `
                <div class="test-params">
                    ${params.map(p => `
                        <div class="test-param-row">
                            <label>${p}</label>
                            <input type="text" class="test-param-input" data-param="${p}"
                                   value="${escapeHtml(state.testParams[p] || '')}"
                                   placeholder="Enter value...">
                        </div>
                    `).join('')}
                </div>
            ` : '<div class="test-no-params">No parameters required. Press <kbd>Enter</kbd> to run.</div>'}
            ${state.testRunning ? '<div class="test-running">Running...</div>' : ''}
            ${state.testResult ? renderTestResult() : ''}
        </div>
    `;
}

function renderTestResult() {
    const r = state.testResult;
    const statusClass = r.success ? 'success' : 'error';
    const statusText = r.success ? `Success (exit ${r.exitCode})` : `Failed (exit ${r.exitCode})`;

    return `
        <div class="test-result ${statusClass}">
            <div class="test-result-status">${statusText}</div>
            ${r.stdout ? `<div class="test-result-output"><strong>stdout:</strong><pre>${escapeHtml(r.stdout)}</pre></div>` : ''}
            ${r.stderr ? `<div class="test-result-output"><strong>stderr:</strong><pre>${escapeHtml(r.stderr)}</pre></div>` : ''}
            ${r.error ? `<div class="test-result-error">${escapeHtml(r.error)}</div>` : ''}
        </div>
    `;
}

function renderApiUsage() {
    const usageEl = document.getElementById('api-usage');
    if (!usageEl) return;

    const exampleAction = state.actionIds[0] || 'my-action';
    const exampleParams = state.actions[exampleAction]
        ? extractParams(state.actions[exampleAction].command)
        : ['param1'];

    const paramsObj = exampleParams.length > 0
        ? exampleParams.reduce((acc, p) => ({ ...acc, [p]: '...' }), {})
        : {};

    const example = JSON.stringify({ action: exampleAction, params: paramsObj }, null, 2);

    usageEl.innerHTML = `
        <div class="api-usage-header">
            <span>API Usage</span>
            <button class="btn-copy" data-action="copy">Copy</button>
        </div>
        <div class="api-usage-content">
            <code>POST http://127.0.0.1:42700/api/task-runner/execute</code>
            <pre id="api-example">${escapeHtml(example)}</pre>
        </div>
    `;
}

function setupTestInputs() {
    const panel = container.querySelector('.test-panel');
    if (!panel) return;

    panel.addEventListener('keydown', handleTestKeydown);

    const inputs = container.querySelectorAll('.test-param-input');
    inputs.forEach(input => {
        input.addEventListener('input', (e) => {
            state.testParams[e.target.dataset.param] = e.target.value;
        });
    });

    const firstInput = container.querySelector('.test-param-input');
    if (firstInput) {
        firstInput.focus();
    } else {
        panel.setAttribute('tabindex', '0');
        panel.focus();
    }
}

function handleTestKeydown(e) {
    if (e.key === 'Enter' && !state.testRunning) {
        e.preventDefault();
        runTest();
    }
    if (e.key === 'Escape') {
        e.preventDefault();
        closeTestPanel();
    }
}

function handleListClick(e) {
    const btn = e.target.closest('button[data-action]');
    if (btn) {
        const action = btn.dataset.action;
        if (action === 'run-test') runTest();
        else if (action === 'close-test') closeTestPanel();
        else if (action === 'copy') copyApiExample();
        return;
    }

    const card = e.target.closest('.action-card');
    if (card && !e.target.closest('.test-panel')) {
        const index = parseInt(card.dataset.index, 10);
        if (index === state.selectedIndex) {
            openEditModal(state.actionIds[index]);
        } else {
            state.selectedIndex = index;
            renderActions();
        }
    }
}

function extractParams(command) {
    const matches = command.match(/\{\{(\w+)\}\}/g) || [];
    return [...new Set(matches.map(m => m.slice(2, -2)))];
}

function escapeHtml(str) {
    if (!str) return '';
    return str.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

function openTestPanel(actionId) {
    state.testingActionId = actionId;
    state.testParams = {};
    state.testResult = null;
    state.testRunning = false;
    renderActions();
}

function closeTestPanel() {
    state.testingActionId = null;
    state.testParams = {};
    state.testResult = null;
    renderActions();
}

async function runTest() {
    if (!state.testingActionId || state.testRunning) return;

    state.testRunning = true;
    state.testResult = null;
    renderActions();

    try {
        const res = await fetch(`${API_BASE}/execute`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                action: state.testingActionId,
                params: state.testParams
            })
        });

        const data = await res.json();
        state.testResult = data;
    } catch (e) {
        state.testResult = { success: false, error: e.message, exitCode: -1 };
    }

    state.testRunning = false;
    renderActions();
    setupTestInputs();
}

function copyApiExample() {
    const pre = document.getElementById('api-example');
    if (pre) {
        navigator.clipboard.writeText(pre.textContent);
    }
}

function openEditModal(actionId = null) {
    state.editModalOpen = true;
    state.editingActionId = actionId;

    const action = actionId ? state.actions[actionId] : null;
    const isNew = !actionId;
    const title = isNew ? 'New Action' : 'Edit Action';

    const modal = document.createElement('div');
    modal.className = 'edit-modal';
    modal.innerHTML = `
        <div class="edit-modal-content">
            <h3>${title}</h3>

            <div class="form-group">
                <label>ID <span class="hint">(used in API calls)</span></label>
                <input type="text" id="action-id" value="${actionId || ''}"
                       placeholder="e.g., open-vscode" ${actionId ? 'disabled' : ''}>
            </div>

            <div class="form-group">
                <label>Name</label>
                <input type="text" id="action-name" value="${escapeHtml(action?.name || '')}"
                       placeholder="e.g., Open in VS Code">
            </div>

            <div class="form-group">
                <label>Description <span class="hint">(optional)</span></label>
                <input type="text" id="action-desc" value="${escapeHtml(action?.description || '')}"
                       placeholder="e.g., Opens a path in Visual Studio Code">
            </div>

            <div class="form-group">
                <label>Command <span class="hint">(use {{param}} for parameters)</span></label>
                <input type="text" id="action-command" value="${escapeHtml(action?.command || '')}"
                       placeholder="e.g., code {{path}}">
            </div>

            <div class="form-group">
                <label>Timeout <span class="hint">(seconds)</span></label>
                <input type="number" id="action-timeout" value="${action?.timeout || 60}" min="1" max="3600">
            </div>

            <div class="modal-buttons">
                <button class="modal-cancel">Cancel</button>
                <button class="modal-save">Save</button>
            </div>
        </div>
    `;

    container.appendChild(modal);
    modal.addEventListener('click', handleModalClick);

    const firstInput = isNew
        ? document.getElementById('action-id')
        : document.getElementById('action-name');
    setTimeout(() => firstInput?.focus(), 0);
}

function handleModalClick(e) {
    if (e.target.classList.contains('edit-modal')) {
        closeEditModal();
        return;
    }
    if (e.target.closest('.modal-cancel')) {
        closeEditModal();
        return;
    }
    if (e.target.closest('.modal-save')) {
        saveAction();
        return;
    }
}

function closeEditModal() {
    const modal = container.querySelector('.edit-modal');
    if (modal) modal.remove();
    state.editModalOpen = false;
    state.editingActionId = null;
}

async function saveAction() {
    const idInput = document.getElementById('action-id');
    const nameInput = document.getElementById('action-name');
    const descInput = document.getElementById('action-desc');
    const commandInput = document.getElementById('action-command');
    const timeoutInput = document.getElementById('action-timeout');

    const actionId = idInput.value.trim().toLowerCase().replace(/[^a-z0-9-]/g, '-');
    const name = nameInput.value.trim();
    const command = commandInput.value.trim();

    if (!actionId || !name || !command) {
        return;
    }

    const action = {
        name,
        description: descInput.value.trim(),
        command,
        timeout: parseInt(timeoutInput.value, 10) || 60
    };

    state.actions[actionId] = action;
    if (!state.actionIds.includes(actionId)) {
        state.actionIds.push(actionId);
        state.selectedIndex = state.actionIds.length - 1;
    }

    await persistConfig();
    closeEditModal();
    renderActions();
    renderApiUsage();
}

async function deleteAction() {
    if (state.actionIds.length === 0 || state.selectedIndex < 0) return;

    const actionId = state.actionIds[state.selectedIndex];
    delete state.actions[actionId];
    state.actionIds.splice(state.selectedIndex, 1);
    state.selectedIndex = Math.min(state.selectedIndex, Math.max(0, state.actionIds.length - 1));

    await persistConfig();
    renderActions();
    renderApiUsage();
}

async function persistConfig() {
    try {
        await fetch(`${API_BASE}/config`, {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ actions: state.actions })
        });
    } catch (e) {
        console.error('Failed to save config:', e);
    }
}

export function handleKey(e) {
    if (state.editModalOpen) {
        handleModalKey(e);
        return;
    }

    if (state.testingActionId) {
        if (e.key === 'Escape') {
            e.preventDefault();
            closeTestPanel();
        }
        return;
    }

    const handler = keyHandlers[e.key];
    if (handler) {
        e.preventDefault();
        handler();
    }
}

function handleModalKey(e) {
    if (e.key === 'Escape') {
        e.preventDefault();
        closeEditModal();
        return;
    }
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
        e.preventDefault();
        saveAction();
        return;
    }
}

const keyHandlers = {
    ArrowUp: () => navigate(-1),
    ArrowDown: () => navigate(1),
    Enter: () => {
        if (state.actionIds.length > 0) {
            openEditModal(state.actionIds[state.selectedIndex]);
        }
    },
    t: () => {
        if (state.actionIds.length > 0) {
            openTestPanel(state.actionIds[state.selectedIndex]);
        }
    },
    T: () => keyHandlers.t(),
    a: () => openEditModal(),
    A: () => openEditModal(),
    d: deleteAction,
    D: deleteAction
};

function navigate(delta) {
    if (state.actionIds.length === 0) return;
    state.selectedIndex = Math.max(0, Math.min(state.actionIds.length - 1, state.selectedIndex + delta));
    renderActions();
}

export function isBlocking() {
    return state.editModalOpen || state.testingActionId !== null;
}

export function onFocus() {
    loadActions();
}

export function onBlur() {}
