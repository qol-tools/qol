import { html } from '../../lib/html.js';
import { useEffect, useCallback, useRef } from 'preact/hooks';
import { Modal } from '../../components/ModalPreact.js';

export function TestPanel({
    actionId,
    params,
    testParams,
    onParamChange,
    onRun,
    onClose,
    running,
    result
}) {
    const panelRef = useRef(null);

    useEffect(() => {
        const firstInput = panelRef.current?.querySelector('.test-param-input');
        if (firstInput) firstInput.focus();
        if (!firstInput) panelRef.current?.focus();
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
                    ${params.map(param => html`
                        <div key=${param} class="test-param-row">
                            <label>${param}</label>
                            <input type="text" class="test-param-input" data-param="${param}"
                                   value=${testParams[param] || ''}
                                   onInput=${(e) => onParamChange(param, e.target.value)}
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

export function ActionEditModal({ modal, onUpdate, onClose, onSave }) {
    const title = modal.isNew ? 'New Action' : 'Edit Action';

    useEffect(() => {
        const targetId = modal.isNew ? 'action-id' : 'action-name';
        setTimeout(() => document.getElementById(targetId)?.focus(), 0);
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
