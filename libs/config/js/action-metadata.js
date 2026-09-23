import { prettyLabel } from './heuristics.js';

export function sectionActions(sectionData) {
    const actions = sectionData.actions || [];
    if (actions.length === 0) {
        return actions;
    }
    if (sectionData.fields.length === 0) {
        return actions;
    }
    if (actions.length > 1) {
        return actions;
    }
    return [];
}

export function createActionBadges(actions) {
    const row = document.createElement('div');
    row.className = 'meta-chip-row section-actions';
    row.appendChild(createActionLabel());
    for (const action of actions) {
        row.appendChild(createActionBadge(action));
    }
    return row;
}

function createActionLabel() {
    const label = document.createElement('span');
    label.className = 'meta-chip-label section-actions-label';
    label.textContent = 'Used by';
    return label;
}

function createActionBadge(action) {
    const badge = document.createElement('span');
    badge.className = 'meta-chip section-action-badge';
    badge.textContent = prettyLabel(action);
    badge.title = action;
    return badge;
}
