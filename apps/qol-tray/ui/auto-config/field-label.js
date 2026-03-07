import { prettyLabel } from './heuristics.js';

export function createFieldLabel(key) {
    const text = prettyLabel(key);
    const label = document.createElement('div');
    label.className = labelClassName(text);
    label.textContent = text;
    label.title = text;
    return label;
}

function labelClassName(text) {
    if (text.length > 28) return 'field-label field-label-tight';
    if (text.length > 20) return 'field-label field-label-compact';
    return 'field-label';
}
