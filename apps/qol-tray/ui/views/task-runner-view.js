import { useEffect, useRef, useMemo, useState } from 'preact/hooks';
import { html } from '../lib/html.js';
import { useRegisterCommands } from '../palette/useRegisterCommands.js';
import { useRegisterViewKeyboard } from '../app/view-keyboard-context.js';
import { PageHeader } from '../components/PageHeader.js';
import { SurfaceContainer } from '../lib/components/SurfaceContainer.js';

import { API_BASE, buildApiExample } from './task-runner/data.js';
import { ActionEditForm } from './task-runner/panels.js';
import { useTaskData } from './task-runner/use-task-data.js';
import { useEditModal } from './task-runner/use-edit-modal.js';
import { useTestPanel } from './task-runner/use-test-panel.js';
import { useTaskKeyHandler } from './task-runner/key-router.js';
import { ActionList } from './task-runner/action-list.js';
import { testRunnerSlot } from './task-runner/test-runner-subpage.js';
import { diveViaSelector } from '../lib/world-navigation-singleton.js';

import { createSharedSlot } from '../lib/shared-slot.js';
const editSlot = createSharedSlot({ modal: null, fieldProps: () => ({}), handlers: {} });
const TEST_RUNNER_DIVE_SELECTOR = '[data-dive-source="task-runner-test-runner"]';

const CSS_ID = 'task-runner-css';

function ensureCssLoaded() {
    if (document.getElementById(CSS_ID)) return;
    const link = document.createElement('link');
    link.id = CSS_ID;
    link.rel = 'stylesheet';
    link.href = '/features/task-runner/style.css';
    document.head.appendChild(link);
}

function ApiUsage({ actions, actionIds, copyApiExample }) {
    const hasActions = actionIds.length > 0;
    const exampleJson = buildApiExample(actions, actionIds).json;
    return html`<div class="api-usage">
        <div class="api-usage-header">
            <span>API Usage</span>
            ${hasActions && html`<button class="btn btn-ghost btn-sm" onClick=${copyApiExample}>Copy</button>`}
        </div>
        <div class="api-usage-content">
            <code>POST ${API_BASE}/execute</code>
            <pre>${exampleJson}</pre>
        </div>
    </div>`;
}

export function TaskRunnerView() {
    useEffect(ensureCssLoaded, []);

    const data = useTaskData();
    const edit = useEditModal(data.actionsRef, data.setActions, data.setActionIds, data.setSelectedIndex);
    const test = useTestPanel();
    const { handleKey, isBlocking, modalNav } = useTaskKeyHandler(data, edit);
    useRegisterViewKeyboard('task-runner', handleKey, isBlocking);

    useEffect(() => {
        editSlot.set({
            modal: edit.editModal,
            fieldProps: modalNav.fieldProps,
            handlers: {
                updateField: edit.updateField,
                onClose: edit.close,
                onSave: edit.saveAction,
            },
        });
    }, [edit.editModal]);

    useEffect(() => {
        const action = test.testingId ? data.actions[test.testingId] : null;
        testRunnerSlot.set({
            actionId: test.testingId,
            action,
            testParams: test.testParams,
            onParamChange: (param, value) => test.setTestParams(prev => ({ ...prev, [param]: value })),
            onRun: test.runTest,
            running: test.testRunning,
            result: test.testResult,
        });
    }, [test.testingId, test.testParams, test.testRunning, test.testResult, data.actions]);

    const editRef = useRef(edit);
    editRef.current = edit;
    const dataRef = useRef(data);
    dataRef.current = data;
    const testRef = useRef(test);
    testRef.current = test;
    const commands = useMemo(() => [
        { id: 'tasks:add', label: 'Add new action', run: () => editRef.current.openEditModal() },
        { id: 'tasks:delete', label: 'Delete selected action', run: () => dataRef.current.deleteAction() },
        { id: 'tasks:test', label: 'Test selected action', run: () => {
            const ids = dataRef.current.actionIdsRef.current;
            if (ids.length === 0) return;
            testRef.current.openTestPanel(ids[dataRef.current.selectedIndexRef.current]);
            diveViaSelector(TEST_RUNNER_DIVE_SELECTOR);
        } },
        { id: 'tasks:copy', label: 'Copy API example', run: () => dataRef.current.copyApiExample() },
    ], []);
    useRegisterCommands('task-runner', commands);

    return html`<div class="view-container content-shell">
        <${PageHeader} subtitle="HTTP API for browser extensions to run local commands" />
        <${SurfaceContainer} className="view-body">
            <${ApiUsage} actions=${data.actions} actionIds=${data.actionIds} copyApiExample=${data.copyApiExample} />
            <${ActionList} data=${data} edit=${edit} />
        <//>
    </div>`;
}

export function ActionEditorSubPage() {
    const [, bump] = useState(0);
    useEffect(() => editSlot.subscribe(() => bump(t => t + 1)), []);

    const { modal, fieldProps, handlers } = editSlot.get();
    if (!modal) {
        return html`<div class="view-container content-shell">
            <${PageHeader} title="Action Editor" subtitle="Select an action to edit" />
        </div>`;
    }
    return html`
        <div class="view-container content-shell">
            <${PageHeader} title=${modal.isNew ? 'Add Action' : 'Edit Action'}
                subtitle=${modal.name || modal.actionId || 'new action'} />
            <div class="view-body content-shell-body">
                <div class="content-shell-inner">
                    <${SurfaceContainer} className="content-frame">
                        <${ActionEditForm} modal=${modal} fieldProps=${fieldProps} handlers=${handlers} />
                    <//>
                </div>
            </div>
        </div>
    `;
}
