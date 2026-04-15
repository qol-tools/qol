import { html } from '../../lib/html.js';
import { useEffect, useCallback, useRef } from 'preact/hooks';
import { ModalActions } from '../../lib/components/ModalPreact.js';

export function TestPanel({ actionId, params, testParams, onParamChange, onRun, onClose, running, result }) {
    const panelRef = useRef(null);
    useEffect(() => {
        const el = panelRef.current?.querySelector('.test-param-input');
        (el || panelRef.current)?.focus();
    }, []);
    const handleKeydown = useCallback((e) => {
        if (e.key === 'Enter' && !running) { e.preventDefault(); onRun(); }
        if (e.key === 'Escape') { e.preventDefault(); onClose(); }
    }, [running, onRun, onClose]);
    return html`
        <div class="test-panel" ref=${panelRef} tabindex="0" onKeydown=${handleKeydown}>
            <div class="test-panel-header"><span>Test: ${actionId}</span><span class="test-hints"><kbd>Enter</kbd> run <kbd>Esc</kbd> close</span></div>
            <${TestParamsList} params=${params} testParams=${testParams} onParamChange=${onParamChange} />
            ${running && html`<div class="test-running">Running...</div>`}
            <${TestResult} result=${result} />
        </div>
    `;
}

function TestParamsList({ params, testParams, onParamChange }) {
    if (params.length === 0) return html`<div class="test-no-params">No parameters required. Press <kbd>Enter</kbd> to run.</div>`;
    return html`
        <div class="test-params">
            ${params.map(param => html`
                <div key=${param} class="test-param-row">
                    <label>${param}</label>
                    <input type="text" class="test-param-input" data-param="${param}"
                           value=${testParams[param] || ''}
                           onInput=${(e) => onParamChange(param, e.target.value)}
                           placeholder="Enter value..." />
                </div>
            `)}
        </div>
    `;
}

function TestResult({ result }) {
    if (!result) return null;
    return html`
        <div class="test-result ${result.success ? 'success' : 'error'}">
            <div class="test-result-status">${result.success ? `Success (exit ${result.exitCode})` : `Failed (exit ${result.exitCode})`}</div>
            ${result.stdout && html`<div class="test-result-output"><strong>stdout:</strong><pre>${result.stdout}</pre></div>`}
            ${result.stderr && html`<div class="test-result-output"><strong>stderr:</strong><pre>${result.stderr}</pre></div>`}
            ${result.error && html`<div class="test-result-error">${result.error}</div>`}
        </div>
    `;
}

export function ActionEditForm({ modal, fieldProps, handlers }) {
    const { updateField, onClose, onSave } = handlers;
    let fi = 0;
    return html`
        <div class="edit-modal-content">
            <div class="form-group" ...${fieldProps(fi++)}>
                <label>ID <span class="hint">(used in API calls)</span></label>
                <input type="text" value=${modal.actionId} placeholder="e.g., open-vscode"
                       disabled=${!modal.isNew} onInput=${(e) => updateField('actionId', e.target.value)} />
            </div>
            <div class="form-group" ...${fieldProps(fi++)}>
                <label>Name</label>
                <input type="text" value=${modal.name} placeholder="e.g., Open in VS Code"
                       onInput=${(e) => updateField('name', e.target.value)} />
            </div>
            <div class="form-group" ...${fieldProps(fi++)}>
                <label>Description <span class="hint">(optional)</span></label>
                <input type="text" value=${modal.description} placeholder="e.g., Opens a path in Visual Studio Code"
                       onInput=${(e) => updateField('description', e.target.value)} />
            </div>
            <div class="form-group" ...${fieldProps(fi++)}>
                <label>Command <span class="hint">(use ${'{{'}param${'}}'}  for parameters)</span></label>
                <input type="text" value=${modal.command} placeholder="e.g., code {{path}}"
                       onInput=${(e) => updateField('command', e.target.value)} />
            </div>
            <div class="form-group" ...${fieldProps(fi++)}>
                <label>Timeout <span class="hint">(seconds)</span></label>
                <input type="number" value=${modal.timeout} min="1" max="3600"
                       onInput=${(e) => updateField('timeout', parseInt(e.target.value, 10) || 60)} />
            </div>
            <${ModalActions} onClose=${onClose} onSave=${onSave} />
        </div>
    `;
}

