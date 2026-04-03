import { html } from '../../lib/html.js';
import { extractParams } from './data.js';
import { TestPanel } from './panels.js';

function handleCardClick(e, isSelected, actionId, index, onEdit, onSelect) {
    if (e.target.closest('.test-panel')) return;
    if (isSelected) onEdit(actionId);
    else onSelect(index);
}

function ActionCard({ actionId, action, isSelected, index, onSelect, onEdit, testSlot }) {
    const params = extractParams(action.command);
    const classes = `action-card ${isSelected ? 'selected' : ''} ${testSlot ? 'testing' : ''}`;
    return html`
        <div key=${actionId} class=${classes} data-selected-surface="" tabIndex="-1" data-selected=${isSelected ? 'true' : 'false'} data-index="${index}" data-id="${actionId}"
             onFocus=${() => onSelect(index)}
             onClick=${e => handleCardClick(e, isSelected, actionId, index, onEdit, onSelect)}>
            <div class="action-header" data-selected-text="">
                <span class="action-id">${actionId}</span>
                ${isSelected && html`<span class="action-hints"><kbd>Enter</kbd> edit <kbd>t</kbd> test <kbd>d</kbd> delete</span>`}
            </div>
            <div class="action-name" data-selected-text="">${action.name}</div>
            ${action.description && html`<div class="action-desc" data-selected-text="">${action.description}</div>`}
            <div class="action-command" data-selected-text="">$ ${action.command}</div>
            ${params.length > 0 && html`
                <div class="action-params" data-selected-text="">Parameters: ${params.map(p => html`<code key=${p}>{{'${p}'}}</code> `)}</div>
            `}
            ${testSlot}
        </div>
    `;
}

function buildTestSlot(actionId, test, action) {
    if (test.testingId !== actionId) return null;
    return html`<${TestPanel}
        actionId=${actionId} params=${extractParams(action.command)}
        testParams=${test.testParams} onParamChange=${(p, v) => test.setTestParams(prev => ({ ...prev, [p]: v }))}
        onRun=${test.runTest} onClose=${test.closeTestPanel}
        running=${test.testRunning} result=${test.testResult} />`;
}

export function ActionList({ data, edit, test }) {
    if (data.actionIds.length === 0) {
        return html`<div class="actions-list">
            <div class="empty">No actions configured. Press <kbd>a</kbd> to add one.</div>
        </div>`;
    }
    return html`<div class="actions-list">
        ${data.actionIds.map((actionId, index) => html`<${ActionCard} key=${actionId}
            actionId=${actionId} action=${data.actions[actionId]}
            isSelected=${index === data.selectedIndex} index=${index}
            onSelect=${data.setSelectedIndex} onEdit=${edit.openEditModal}
            testSlot=${buildTestSlot(actionId, test, data.actions[actionId])} />`)}
    </div>`;
}
