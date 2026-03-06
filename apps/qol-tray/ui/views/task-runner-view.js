import { html } from '../lib/html.js';
import { useEffect, useCallback } from 'preact/hooks';
import { useStateRef } from '../hooks/useStateRef.js';
import { usePersistedIndex } from '../hooks/usePersistedIndex.js';
import { PageHeader } from '../components/PageHeader.js';
import { useFooterShortcuts } from '../hooks/useFooterShortcuts.js';
import { withShiftVariants, dispatchKey } from '../utils/keys.js';
import {
    API_BASE,
    buildApiExample,
    buildSavedActions,
    createEditModalState,
    extractParams,
    loadTaskRunnerData,
    nextSelectedIndex,
    persistTaskRunnerConfig,
    removeSelectedAction,
    runTaskActionTest
} from './task-runner/data.js';
import { ActionEditModal, TestPanel } from './task-runner/panels.js';

const CSS_ID = 'task-runner-css';

const SHORTCUTS = [
    { key: '↑↓', label: 'navigate' },
    { key: 'Enter', label: 'edit' },
    { key: 't', label: 'test' },
    { key: 'a', label: 'add' },
    { key: 'd', label: 'delete' },
    { key: 'c', label: 'copy API' }
];

export function TaskRunnerView() {
    const [actions, setActions, actionsRef] = useStateRef({});
    const [actionIds, setActionIds, actionIdsRef] = useStateRef([]);
    const [selectedIndex, setSelectedIndex, selectedIndexRef, taskMarkRestored] = usePersistedIndex('taskrunner-selected-index');
    const [editModal, setEditModal, editModalRef] = useStateRef(null); // null | { actionId, name, desc, command, timeout }
    const [testingId, setTestingId, testingIdRef] = useStateRef(null);
    const [testParams, setTestParams] = useStateRef({});
    const [testResult, setTestResult] = useStateRef(null);
    const [testRunning, setTestRunning] = useStateRef(false);

    useEffect(() => {
        if (document.getElementById(CSS_ID)) return;
        const link = document.createElement('link');
        link.id = CSS_ID;
        link.rel = 'stylesheet';
        link.href = '/features/task-runner/style.css';
        document.head.appendChild(link);
    }, []);

    useFooterShortcuts(SHORTCUTS);

    const loadActions = useCallback(async () => {
        try {
            const loaded = await loadTaskRunnerData();
            setActions(loaded.actions);
            setActionIds(loaded.actionIds);
            setSelectedIndex(prev => {
                taskMarkRestored();
                return prev >= 0 && prev < loaded.actionIds.length ? prev : 0;
            });
        } catch {}
    }, []);

    useEffect(() => { loadActions(); }, [loadActions]);

    const deleteAction = useCallback(() => {
        const ids = actionIdsRef.current;
        const idx = selectedIndexRef.current;
        if (ids.length === 0 || idx < 0) return;
        const nextActions = removeSelectedAction(actionsRef.current, ids, idx);
        const nextIds = Object.keys(nextActions);
        setActions(nextActions);
        setActionIds(nextIds);
        setSelectedIndex(nextSelectedIndex(nextIds, idx));
        void persistTaskRunnerConfig(nextActions);
    }, []);

    const openEditModal = useCallback((actionId = null) => {
        setEditModal(createEditModalState(actionsRef.current, actionId));
    }, []);

    const saveAction = useCallback(() => {
        if (!editModal) return;
        const saved = buildSavedActions(actionsRef.current, editModal);
        if (!saved.actionId || !saved.actions[saved.actionId]?.name || !saved.actions[saved.actionId]?.command) return;
        const nextIds = Object.keys(saved.actions);
        setActions(saved.actions);
        setActionIds(nextIds);
        if (editModal.isNew) setSelectedIndex(nextIds.length - 1);
        void persistTaskRunnerConfig(saved.actions);
        setEditModal(null);
    }, [editModal]);

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
            setTestResult(await runTaskActionTest(testingId, testParams));
        } catch (e) {
            setTestResult({ success: false, error: e.message, exitCode: -1 });
        }
        setTestRunning(false);
    }, [testingId, testRunning, testParams]);

    const copyApiExample = useCallback(() => {
        navigator.clipboard.writeText(buildApiExample(actionsRef.current, actionIdsRef.current).json);
    }, []);

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
        dispatchKey(e, withShiftVariants({
            ArrowUp: () => setSelectedIndex(i => Math.max(0, i - 1)),
            ArrowDown: () => setSelectedIndex(i => Math.min(ids.length - 1, i + 1)),
            Enter: () => { if (ids.length > 0) openEditModal(ids[idx]); },
            t: () => { if (ids.length > 0) openTestPanel(ids[idx]); },
            a: () => openEditModal(),
            d: deleteAction,
            c: copyApiExample,
        }));
    }, [saveAction, closeTestPanel, openEditModal, openTestPanel, deleteAction, copyApiExample]);

    const isBlocking = useCallback(() => editModalRef.current !== null || testingIdRef.current !== null, []);

    TaskRunnerView.handleKey = handleKey;
    TaskRunnerView.isBlocking = isBlocking;

    const exampleJson = buildApiExample(actions, actionIds).json;

    return html`
        <div class="view-container">
            <${PageHeader}
                title="Task Runner"
                subtitle="HTTP API for browser extensions to run local commands"
            />
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
