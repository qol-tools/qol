import { html } from '../../lib/html.js';
import { useEffect, useState, useRef, useCallback } from 'preact/hooks';
import { PageHeader } from '../../components/PageHeader.js';
import { SurfaceContainer } from '../../lib/components/SurfaceContainer.js';
import { Button } from '../../lib/components/Button.js';
import { createSharedSlot } from '../../lib/shared-slot.js';
import { extractParams } from './data.js';

export const testRunnerSlot = createSharedSlot({
    actionId: null,
    action: null,
    testParams: {},
    onParamChange: null,
    onRun: null,
    running: false,
    result: null,
});

function dispatchEscape() {
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
}

export function TestRunnerSubPage() {
    const [, bump] = useState(0);
    useEffect(() => testRunnerSlot.subscribe(() => bump(t => t + 1)), []);
    const slot = testRunnerSlot.get();
    const firstInputRef = useRef(null);

    useEffect(() => {
        if (slot.actionId && firstInputRef.current) {
            firstInputRef.current.focus({ preventScroll: true });
        }
    }, [slot.actionId]);

    const onCancel = useCallback(() => dispatchEscape(), []);
    const onRun = useCallback(() => {
        const fn = testRunnerSlot.get().onRun;
        if (fn) fn();
    }, []);

    if (!slot.actionId || !slot.action) {
        return html`<div class="view-container content-shell">
            <${PageHeader} title="Test Action" subtitle="Select an action to test" />
        </div>`;
    }

    const params = extractParams(slot.action.command);
    const onKey = (e) => {
        if (e.key === 'Enter' && !slot.running) { e.preventDefault(); onRun(); }
    };

    return html`
        <div class="view-container content-shell">
            <${PageHeader} title="Test Action" subtitle=${slot.actionId} />
            <div class="view-body content-shell-body">
                <div class="content-shell-inner">
                    <${SurfaceContainer} className="content-frame test-runner-frame" onKeyDown=${onKey}>
                        <${TestParams}
                            params=${params}
                            testParams=${slot.testParams}
                            onParamChange=${slot.onParamChange}
                            firstInputRef=${firstInputRef} />
                        ${slot.running && html`<div class="test-running">Running...</div>`}
                        <${TestResult} result=${slot.result} />
                        <div class="test-runner-actions">
                            <${Button} variant="btn-ghost" onActivate=${onCancel}>
                                Close <kbd>Esc</kbd>
                            <//>
                            <${Button} variant="btn-primary" onActivate=${onRun} disabled=${slot.running}>
                                Run <kbd>Enter</kbd>
                            <//>
                        </div>
                    <//>
                </div>
            </div>
        </div>
    `;
}

function TestParams({ params, testParams, onParamChange, firstInputRef }) {
    if (params.length === 0) {
        return html`<div class="test-no-params">No parameters required. Press <kbd>Enter</kbd> to run.</div>`;
    }
    return html`
        <div class="test-params">
            ${params.map((param, i) => html`
                <div key=${param} class="test-param-row">
                    <label>${param}</label>
                    <input
                        ref=${i === 0 ? firstInputRef : null}
                        type="text"
                        class="test-param-input"
                        data-param=${param}
                        value=${testParams?.[param] || ''}
                        onInput=${(e) => onParamChange?.(param, e.target.value)}
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
