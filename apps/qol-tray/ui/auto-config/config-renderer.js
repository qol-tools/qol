import {
    isColorField,
    isEmptyObjectArray,
    isModArray,
    isObjectArray,
    isPlainObject,
    isStringArray,
    prettyLabel,
} from './heuristics.js';
import {
    renderBoolean,
    renderColor,
    renderModArrayStandalone,
    renderNumber,
    renderString,
} from './primitive-fields.js';
import { renderStringList } from './string-list-renderer.js';
import { renderObjectArray } from './object-array-renderer.js';

export function renderConfig(container, obj, state, path = '') {
    for (const [key, value] of Object.entries(obj)) {
        const fullPath = path ? `${path}.${key}` : key;

        if (typeof value === 'boolean') {
            container.appendChild(renderBoolean(key, fullPath, state));
        } else if (typeof value === 'number') {
            container.appendChild(renderNumber(key, value, fullPath, state));
        } else if (isColorField(key, value)) {
            container.appendChild(renderColor(key, fullPath, state));
        } else if (typeof value === 'string') {
            container.appendChild(renderString(key, fullPath, state));
        } else if (isModArray(key, value)) {
            container.appendChild(renderModArrayStandalone(key, fullPath, state));
        } else if (isStringArray(value)) {
            container.appendChild(renderStringList(key, fullPath, state));
        } else if (isObjectArray(value) || isEmptyObjectArray(key, value)) {
            container.appendChild(renderObjectArray(key, value, fullPath, state));
        } else if (isPlainObject(value)) {
            container.appendChild(renderSection(key, value, fullPath, state));
        }
    }
}

export function renderSection(key, value, path, state) {
    const card = document.createElement('section');
    card.className = 'card';

    const heading = document.createElement('h2');
    heading.textContent = prettyLabel(key);
    card.appendChild(heading);

    renderConfig(card, value, state, path);
    return card;
}
