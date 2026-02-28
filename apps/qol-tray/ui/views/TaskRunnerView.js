import { html } from '../lib/html.js';
import { useState, useEffect, useCallback, useRef } from 'preact/hooks';
import { Modal } from '../components/ModalPreact.js';
import { renderShortcutLegend } from '../components/shortcut-legend.js';

const API_BASE = '/api/task-runner';
const CSS_ID = 'task-runner-css';

const SHORTCUTS = [
    { key: '↑↓', label: 'navigate' },
    { key: 'Enter', label: 'edit' },
    { key: 't', label: 'test' },
    { key: 'a', label: 'add' },
    { key: 'd', label: 'delete' },
    { key: 'c', label: 'copy API' }
];

function extractParams(command) {
    const matches = command.match(/\{\{(\w+)\}\}/g) || [];
    return [...new Set(matches.map(m => m.slice(2, -2)))];
}

function escapeHtml(value) {
    return String(value).replace(/[&<>"']/g, c =>
        ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c])
    );
}

export function TaskRunnerView() {
    const [actions, setActions] = useState({});
    const [actionIds, setActionIds] = useState([]);
    const [selectedIndex, setSelectedIndex] = useState(() => {
        const saved = parseInt(localStorage.getItem('taskrunner-selected-index') || '0', 10);
        return saved >= 0 ? saved : 0;
    });
    const taskRestoredRef = useRef(false);
    const [editModal, setEditModal] = useState(null); // null | { actionId, name, desc, command, timeout }
    const [testingId, setTestingId] = useState(null);
    const [testParams, setTestParams] = useState({});
    const [testResult, setTestResult] = useState(null);
    const [testRunning, setTestRunning] = useState(false);

    // Refs for stable callbacks
    const actionIdsRef = useRef(actionIds);
    actionIdsRef.current = actionIds;
    const selectedIndexRef = useRef(selectedIndex);
    selectedIndexRef.current = selectedIndex;
    const editModalRef = useRef(editModal);
    editModalRef.current = editModal;
    const testingIdRef = useRef(testingId);
    testingIdRef.current = testingId;
    const actionsRef = useRef(actions);
    actionsRef.current = actions;

    // Load stylesheet
    useEffect(() => {
        if (document.getElementById(CSS_ID)) return;
        const link = document.createElement('link');
        link.id = CSS_ID;
        link.rel = 'stylesheet';
        link.href = '/features/task-runner/style.css';
        document.head.appendChild(link);
    }, []);

    // Footer shortcuts
    useEffect(() => {
        const el = document.getElementById('content-footer');
        if (el) el.innerHTML = renderShortcutLegend(SHORTCUTS);
        return () => { if (el) el.innerHTML = ''; };
    }, []);

    // Load actions
    const loadActions = useCallback(async () => {
        try {
            const res = await fetch(`${API_BASE}/config`);
            if (res.ok) {
                const config = await res.json();
                const a = config.actions || {};
                setActions(a);
                const ids = Object.keys(a);
                setActionIds(ids);
                setSelectedIndex(prev => {
                    taskRestoredRef.current = true;
                    return prev >= 0 && prev < ids.length ? prev : 0;
                });
            }
        } catch {}
    }, []);

    useEffect(() => { loadActions(); }, [loadActions]);

    // Save selection
    useEffect(() => {
        if (!taskRestoredRef.current) return;
        localStorage.setItem('taskrunner-selected-index', String(selectedIndex));
    }, [selectedIndex]);

    // Persist
    const persistConfig = useCallback(async (acts) => {
        try {
            await fetch(`${API_BASE}/config`, {
                method: 'PUT',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ actions: acts })
            });
        } catch {}
    }, []);

    // Delete — stable via refs
    const deleteAction = useCallback(() => {
        const ids = actionIdsRef.current;
        const idx = selectedIndexRef.current;
        if (ids.length === 0 || idx < 0) return;
        const id = ids[idx];
        setActions(prev => { const next = { ...prev }; delete next[id]; persistConfig(next); return next; });
        setActionIds(prev => { const next = prev.filter(a => a !== id); return next; });
        setSelectedIndex(prev => Math.min(prev, Math.max(0, ids.length - 2)));
    }, [persistConfig]);

    // Open edit modal — stable via ref
    const openEditModal = useCallback((actionId = null) => {
        const action = actionId ? actionsRef.current[actionId] : null;
        setEditModal({
            actionId: actionId || '',
            isNew: !actionId,
            name: action?.name || '',
            description: action?.description || '',
            command: action?.command || '',
            timeout: action?.timeout || 60
        });
    }, []);

    // Save action
    const saveAction = useCallback(() => {
        if (!editModal) return;
        const id = editModal.actionId.trim().toLowerCase().replace(/[^a-z0-9-]/g, '-');
        const name = editModal.name.trim();
        const command = editModal.command.trim();
        if (!id || !name || !command) return;
        const action = { name, description: editModal.description.trim(), command, timeout: editModal.timeout };
        setActions(prev => { const next = { ...prev, [id]: action }; persistConfig(next); return next; });
        setActionIds(prev => {
            if (prev.includes(id)) return prev;
            const next = [...prev, id];
            setSelectedIndex(next.length - 1);
            return next;
        });
        setEditModal(null);
    }, [editModal, persistConfig]);

    // Test panel
    const openTestPanel = useCallback((actionId) => {
        setTestingId(actionId);
        setTestParams({});
        setTestResult(null);
        setTestRunning(false);
    }, []);

    const closeTestPanel = useCallback(() => {
        setTestingId(null);
        setTestParams({});
        setTestResult(null);
    }, []);

    const runTest = useCallback(async () => {
        if (!testingId || testRunning) return;
        setTestRunning(true);
        setTestResult(null);
        try {
            const res = await fetch(`${API_BASE}/execute`, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ action: testingId, params: testParams })
            });
            setTestResult(await res.json());
        } catch (e) {
            setTestResult({ success: false, error: e.message, exitCode: -1 });
        }
        setTestRunning(false);
    }, [testingId, testRunning, testParams]);

    // Copy API example — stable via refs
    const copyApiExample = useCallback(() => {
        const ids = actionIdsRef.current;
        const acts = actionsRef.current;
        const exampleAction = ids[0] || 'my-action';
        const params = acts[exampleAction] ? extractParams(acts[exampleAction].command) : ['param1'];
        const paramsObj = params.length > 0 ? params.reduce((acc, p) => ({ ...acc, [p]: '...' }), {}) : {};
        navigator.clipboard.writeText(JSON.stringify({ action: exampleAction, params: paramsObj }, null, 2));
    }, []);

    // Keyboard — stable via refs
    const handleKey = useCallback((e) => {
        if (editModalRef.current) {
            if (e.key === 'Escape') { e.preventDefault(); setEditModal(null); return; }
            if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) { e.preventDefault(); saveAction(); }
            return;
        }
        if (testingIdRef.current) {
            if (e.key === 'Escape') { e.preventDefault(); closeTestPanel(); }
            return;
        }
        const ids = actionIdsRef.current;
        const idx = selectedIndexRef.current;
        const handlers = {
            ArrowUp: () => setSelectedIndex(i => Math.max(0, i - 1)),
            ArrowDown: () => setSelectedIndex(i => Math.min(ids.length - 1, i + 1)),
            Enter: () => { if (ids.length > 0) openEditModal(ids[idx]); },
            t: () => { if (ids.length > 0) openTestPanel(ids[idx]); },
            T: () => { if (ids.length > 0) openTestPanel(ids[idx]); },
            a: () => openEditModal(),
            A: () => openEditModal(),
            d: deleteAction,
            D: deleteAction,
            c: copyApiExample,
            C: copyApiExample,
        };
        const handler = handlers[e.key];
        if (handler) { e.preventDefault(); handler(); }
    }, [saveAction, closeTestPanel, openEditModal, openTestPanel, deleteAction, copyApiExample]);

    const isBlocking = useCallback(() => editModalRef.current !== null || testingIdRef.current !== null, []);

    TaskRunnerView.handleKey = handleKey;
    TaskRunnerView.isBlocking = isBlocking;

    // API usage example
    const exampleAction = actionIds[0] || 'my-action';
    const exampleParams = actions[exampleAction] ? extractParams(actions[exampleAction].command) : ['param1'];
    const exampleParamsObj = exampleParams.length > 0 ? exampleParams.reduce((acc, p) => ({ ...acc, [p]: '...' }), {}) : {};
    const exampleJson = JSON.stringify({ action: exampleAction, params: exampleParamsObj }, null, 2);

    return html`
        <div class="view-container">
            <header>
                <h1>Task Runner</h1>
                <p>HTTP API for browser extensions to run local commands</p>
            </header>
            <div class="view-body">
                <div class="actions-list">
                    ${actionIds.length === 0 && html`
                        <div class="empty">No actions configured. Press <kbd>a</kbd> to add one.</div>
                    `}
                    ${actionIds.map((actionId, index) => {
                        const action = actions[actionId];
                        const isSelected = index === selectedIndex;
                        const isTesting = testingId === actionId;
                        const params = extractParams(action.command);

                        return html`
                            <div key=${actionId}
                                 class="action-card ${isSelected ? 'selected' : ''} ${isTesting ? 'testing' : ''}"
                                 data-index="${index}" data-id="${actionId}"
                                 onClick=${(e) => {
                                     if (e.target.closest('.test-panel')) return;
                                     if (index === selectedIndex) openEditModal(actionId);
                                     else setSelectedIndex(index);
                                 }}>
                                <div class="action-header">
                                    <span class="action-id">${actionId}</span>
                                    ${isSelected && html`<span class="action-hints"><kbd>Enter</kbd> edit <kbd>t</kbd> test <kbd>d</kbd> delete</span>`}
                                </div>
                                <div class="action-name">${action.name}</div>
                                ${action.description && html`<div class="action-desc">${action.description}</div>`}
                                <div class="action-command">$ ${action.command}</div>
                                ${params.length > 0 && html`
                                    <div class="action-params">Parameters: ${params.map(p => html`<code key=${p}>{{'${p}'}}</code> `)}</div>
                                `}
                                ${isTesting && html`
                                    <${TestPanel}
                                        actionId=${actionId}
                                        params=${params}
                                        testParams=${testParams}
                                        onParamChange=${(p, v) => setTestParams(prev => ({ ...prev, [p]: v }))}
                                        onRun=${runTest}
                                        onClose=${closeTestPanel}
                                        running=${testRunning}
                                        result=${testResult}
                                    />
                                `}
                            </div>
                        `;
                    })}
                </div>
                <div class="api-usage">
                    <div class="api-usage-header">
                        <span>API Usage</span>
                        <button class="btn-copy" onClick=${copyApiExample}>Copy</button>
                    </div>
                    <div class="api-usage-content">
                        <code>POST ${API_BASE}/execute</code>
                        <pre>${exampleJson}</pre>
                    </div>
                </div>
            </div>
            ${editModal && html`
                <${ActionEditModal}
                    modal=${editModal}
                    onUpdate=${(field, value) => setEditModal(prev => prev ? { ...prev, [field]: value } : prev)}
                    onClose=${() => setEditModal(null)}
                    onSave=${saveAction}
                />
            `}
        </div>
    `;
}

function TestPanel({ actionId, params, testParams, onParamChange, onRun, onClose, running, result }) {
    const panelRef = useRef(null);

    useEffect(() => {
        const firstInput = panelRef.current?.querySelector('.test-param-input');
        if (firstInput) firstInput.focus();
        else panelRef.current?.focus();
    }, []);

    const handleKeydown = useCallback((e) => {
        if (e.key === 'Enter' && !running) { e.preventDefault(); onRun(); }
        if (e.key === 'Escape') { e.preventDefault(); onClose(); }
    }, [running, onRun, onClose]);

    return html`
        <div class="test-panel" ref=${panelRef} tabindex="0" onKeydown=${handleKeydown}>
            <div class="test-panel-header">
                <span>Test: ${actionId}</span>
                <span class="test-hints"><kbd>Enter</kbd> run <kbd>Esc</kbd> close</span>
            </div>
            ${params.length > 0
                ? html`<div class="test-params">
                    ${params.map(p => html`
                        <div key=${p} class="test-param-row">
                            <label>${p}</label>
                            <input type="text" class="test-param-input" data-param="${p}"
                                   value=${testParams[p] || ''}
                                   onInput=${(e) => onParamChange(p, e.target.value)}
                                   placeholder="Enter value..." />
                        </div>
                    `)}
                </div>`
                : html`<div class="test-no-params">No parameters required. Press <kbd>Enter</kbd> to run.</div>`
            }
            ${running && html`<div class="test-running">Running...</div>`}
            ${result && html`
                <div class="test-result ${result.success ? 'success' : 'error'}">
                    <div class="test-result-status">${result.success ? `Success (exit ${result.exitCode})` : `Failed (exit ${result.exitCode})`}</div>
                    ${result.stdout && html`<div class="test-result-output"><strong>stdout:</strong><pre>${result.stdout}</pre></div>`}
                    ${result.stderr && html`<div class="test-result-output"><strong>stderr:</strong><pre>${result.stderr}</pre></div>`}
                    ${result.error && html`<div class="test-result-error">${result.error}</div>`}
                </div>
            `}
        </div>
    `;
}

function ActionEditModal({ modal, onUpdate, onClose, onSave }) {
    const title = modal.isNew ? 'New Action' : 'Edit Action';

    useEffect(() => {
        const el = modal.isNew ? document.getElementById('action-id') : document.getElementById('action-name');
        setTimeout(() => el?.focus(), 0);
    }, []);

    return html`
        <${Modal} open=${true} onClose=${onClose} className="edit-modal">
            <div class="edit-modal-content">
                <h3>${title}</h3>
                <div class="form-group">
                    <label>ID <span class="hint">(used in API calls)</span></label>
                    <input type="text" id="action-id" value=${modal.actionId}
                           placeholder="e.g., open-vscode" disabled=${!modal.isNew}
                           onInput=${(e) => onUpdate('actionId', e.target.value)} />
                </div>
                <div class="form-group">
                    <label>Name</label>
                    <input type="text" id="action-name" value=${modal.name}
                           placeholder="e.g., Open in VS Code"
                           onInput=${(e) => onUpdate('name', e.target.value)} />
                </div>
                <div class="form-group">
                    <label>Description <span class="hint">(optional)</span></label>
                    <input type="text" id="action-desc" value=${modal.description}
                           placeholder="e.g., Opens a path in Visual Studio Code"
                           onInput=${(e) => onUpdate('description', e.target.value)} />
                </div>
                <div class="form-group">
                    <label>Command <span class="hint">(use ${'{{'}param${'}}'}  for parameters)</span></label>
                    <input type="text" id="action-command" value=${modal.command}
                           placeholder="e.g., code {{path}}"
                           onInput=${(e) => onUpdate('command', e.target.value)} />
                </div>
                <div class="form-group">
                    <label>Timeout <span class="hint">(seconds)</span></label>
                    <input type="number" id="action-timeout" value=${modal.timeout} min="1" max="3600"
                           onInput=${(e) => onUpdate('timeout', parseInt(e.target.value, 10) || 60)} />
                </div>
                <div class="modal-buttons">
                    <button class="modal-cancel" onClick=${onClose}>Cancel</button>
                    <button class="modal-save" onClick=${onSave}>Save</button>
                </div>
            </div>
        <//>
    `;
}
