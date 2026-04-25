import { html } from '../../lib/html.js';
import { extractParams } from './data.js';
import { Card } from '../../lib/components/Card.js';

function ActionCard({ actionId, action, isSelected, index, onSelect, onEdit }) {
    const params = extractParams(action.command);
    const classes = `action-card ${isSelected ? 'selected' : ''}`;
    return html`
        <${Card} className=${classes} data-id="${actionId}"
             index=${index} selected=${isSelected} onSelect=${onSelect}
             data-dive-target="task-runner-editor"
             onActivate=${() => { if (!isSelected) onSelect(index); onEdit(actionId); }}>
            <div class="action-header" data-selected-text="">
                <span class="action-id">${actionId}</span>
                ${isSelected && html`<span class="action-hints"><kbd>Enter</kbd> edit <kbd>d</kbd> delete</span>`}
            </div>
            <div class="action-name" data-selected-text="">${action.name}</div>
            ${action.description && html`<div class="action-desc" data-selected-text="">${action.description}</div>`}
            <div class="action-command" data-selected-text="">$ ${action.command}</div>
            ${params.length > 0 && html`
                <div class="action-params" data-selected-text="">Parameters: ${params.map(p => html`<code key=${p}>{{'${p}'}}</code> `)}</div>
            `}
        <//>
    `;
}

export function ActionList({ data, edit }) {
    if (data.actionIds.length === 0) {
        return html`<div class="actions-list">
            <div class="empty">No actions configured. Press <kbd>a</kbd> to add one.</div>
        </div>`;
    }
    return html`<div class="actions-list">
        ${data.actionIds.map((actionId, index) => html`<${ActionCard} key=${actionId}
            actionId=${actionId} action=${data.actions[actionId]}
            isSelected=${index === data.selectedIndex} index=${index}
            onSelect=${data.setSelectedIndex} onEdit=${edit.openEditModal} />`)}
    </div>`;
}
