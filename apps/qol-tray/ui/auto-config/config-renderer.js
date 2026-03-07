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

export function renderConfig(container, obj, state) {
    const renderState = { config: state.config, path: '', depth: 0 };
    renderEntries(container, obj, renderState);
}

function renderEntries(container, obj, renderState) {
    for (const [key, value] of Object.entries(obj)) {
        const nextState = nextRenderState(renderState, key);
        container.appendChild(renderEntry(key, value, nextState));
    }
}

function renderEntry(key, value, renderState) {
    if (typeof value === 'boolean') return renderBoolean(key, renderState.path, renderState);
    if (typeof value === 'number') return renderNumber(key, value, renderState.path, renderState);
    if (isColorField(key, value)) return renderColor(key, renderState.path, renderState);
    if (typeof value === 'string') return renderString(key, renderState.path, renderState);
    if (isModArray(key, value)) return renderModArrayStandalone(key, renderState.path, renderState);
    if (isStringArray(value)) return renderStringList(key, renderState.path, renderState);
    if (isObjectArray(value) || isEmptyObjectArray(key, value)) {
        return renderObjectArray(key, value, renderState.path, renderState);
    }
    if (isPlainObject(value)) return renderSection(key, value, renderState);
    return document.createDocumentFragment();
}

function renderSection(key, value, renderState) {
    const section = createSection(renderState.depth);
    section.appendChild(createHeading(key, renderState.depth));
    renderEntries(section, value, nestedState(renderState));
    return section;
}

function createSection(depth) {
    const section = document.createElement('section');
    section.className = depth === 1 ? 'card config-section' : 'nested-section config-subsection';
    return section;
}

function createHeading(key, depth) {
    const heading = document.createElement(depth === 1 ? 'h2' : 'h3');
    const text = prettyLabel(key);
    heading.className = headingClassName(text);
    heading.textContent = text;
    heading.title = text;
    return heading;
}

function nextRenderState(renderState, key) {
    return {
        config: renderState.config,
        path: renderState.path ? `${renderState.path}.${key}` : key,
        depth: renderState.depth + 1,
    };
}

function nestedState(renderState) {
    return {
        config: renderState.config,
        path: renderState.path,
        depth: renderState.depth,
    };
}

function headingClassName(text) {
    if (text.length > 28) return 'section-title-tight';
    if (text.length > 20) return 'section-title-compact';
    return '';
}
