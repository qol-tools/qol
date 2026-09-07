import { prettyLabel } from './heuristics.js';

export function createFieldLabel(key) {
    return createTextFieldLabel(prettyLabel(key));
}

export function createTextFieldLabel(text, description = '') {
    const wrapper = document.createElement('div');
    wrapper.className = 'field-label-group';
    wrapper.appendChild(createLabelText(text));
    if (!description) {
        return wrapper;
    }
    const help = document.createElement('div');
    help.className = 'field-help';
    help.textContent = description;
    wrapper.appendChild(help);
    return wrapper;
}

function createLabelText(text) {
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
