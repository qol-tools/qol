import { useEffect, useRef, useMemo } from 'preact/hooks';
import { html } from '../lib/html.js';
import { useRegisterCommands } from '../palette/useRegisterCommands.js';
import { useRegisterViewKeyboard } from '../components/app/view-keyboard-context.js';
import { PageHeader } from '../components/PageHeader.js';
import { SurfaceContainer } from '../components/SurfaceContainer.js';

import { API_BASE, buildApiExample } from './task-runner/data.js';
import { ActionEditModal } from './task-runner/panels.js';
import { useTaskData } from './task-runner/use-task-data.js';
import { useEditModal } from './task-runner/use-edit-modal.js';
import { useTestPanel } from './task-runner/use-test-panel.js';
import { useTaskKeyHandler } from './task-runner/key-router.js';
import { ActionList } from './task-runner/action-list.js';

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
    const { handleKey, isBlocking, modalNav } = useTaskKeyHandler(data, edit, test);
    useRegisterViewKeyboard('task-runner', handleKey, isBlocking);

    const editRef = useRef(edit);
    editRef.current = edit;
    const dataRef = useRef(data);
    dataRef.current = data;
    const testRef = useRef(test);
    testRef.current = test;
    const commands = useMemo(() => [
        { id: 'tasks:add', label: 'Add new action', run: () => editRef.current.openEditModal() },
        { id: 'tasks:delete', label: 'Delete selected action', run: () => dataRef.current.deleteAction() },
        { id: 'tasks:test', label: 'Test selected action', run: () => { const ids = dataRef.current.actionIdsRef.current; if (ids.length > 0) testRef.current.openTestPanel(ids[dataRef.current.selectedIndexRef.current]); } },
        { id: 'tasks:copy', label: 'Copy API example', run: () => dataRef.current.copyApiExample() },
    ], []);
    useRegisterCommands('task-runner', commands);

    return html`<div class="view-container content-shell">
        <${PageHeader} title="Task Runner" subtitle="HTTP API for browser extensions to run local commands" />
        <${SurfaceContainer} className="view-body">
            <${ApiUsage} actions=${data.actions} actionIds=${data.actionIds} copyApiExample=${data.copyApiExample} />
            <${ActionList} data=${data} edit=${edit} test=${test} />
        <//>
        ${edit.editModal && html`<${ActionEditModal}
            modal=${edit.editModal} fieldProps=${modalNav.fieldProps} onUpdate=${edit.updateField}
            onClose=${edit.close} onSave=${edit.saveAction} />`}
    </div>`;
}
