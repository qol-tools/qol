import { getVal } from './config-paths.js';
import { getObjectArraySchema, guessSchemaFromKey, prettyLabel } from './heuristics.js';
import { appendStaticModChips } from './primitive-fields.js';
import { buildAddForm } from './object-array-form.js';
import { getFieldValue } from './normalized-config.js';

export function renderObjectArray(key, value, path, state) {
    return renderObjectArrayBlock({
        title: prettyLabel(key),
        value,
        path,
        state,
        embedded: false,
    });
}

export function renderEmbeddedObjectArray(field, state) {
    return renderObjectArrayBlock({
        title: '',
        value: getFieldValue(state, field) || [],
        path: field.config_key || field.id,
        state,
        embedded: true,
    });
}

function renderObjectArrayBlock(context) {
    const container = document.createElement(context.embedded ? 'div' : 'section');
    container.className = context.embedded ? 'object-array-panel' : 'card';

    if (context.title) {
        const heading = document.createElement('h2');
        heading.textContent = context.title;
        container.appendChild(heading);
    }

    const listEl = document.createElement('div');
    listEl.className = 'rules-list';
    container.appendChild(listEl);

    const schema = context.value.length > 0
        ? getObjectArraySchema(context.value)
        : guessSchemaFromKey(context.path);
    const addRow = document.createElement('div');
    addRow.className = 'add-rule-row';
    container.appendChild(addRow);

    const saveAndRender = () => {
        renderList(listEl, context.path, context.state, saveAndRender);
        context.state.save();
    };
    const renderOnly = () => renderList(listEl, context.path, context.state, saveAndRender);
    buildAddForm(addRow, schema, context.path, context.state, saveAndRender);
    renderOnly();
    return container;
}

function renderList(listEl, path, state, rerender) {
    const values = getVal(state.config, path) || [];
    listEl.replaceChildren();
    if (values.length === 0) {
        const emptyState = document.createElement('div');
        emptyState.className = 'empty-state';
        emptyState.textContent = 'No items.';
        listEl.appendChild(emptyState);
        return;
    }
    values.forEach((item, index) => listEl.appendChild(renderRuleRow(item, index, path, state, rerender)));
}

function renderRuleRow(item, index, path, state, rerender) {
    const row = document.createElement('div');
    row.className = 'rule-row';
    const hasMods = item.from_mods || item.to_mods;
    if (hasMods) appendModsContent(row, item);
    else appendGenericContent(row, item);
    if (item.global) {
        const badge = document.createElement('span');
        badge.className = 'global-badge';
        badge.textContent = 'global';
        row.appendChild(badge);
    }
    row.appendChild(createRemoveButton(index, path, state, rerender));
    return row;
}

function appendModsContent(row, item) {
    row.appendChild(createFromSide(item));
    const arrow = document.createElement('span');
    arrow.className = 'arrow';
    arrow.textContent = '→';
    row.appendChild(arrow);
    row.appendChild(createToSide(item));
}

function createFromSide(item) {
    const side = document.createElement('div');
    side.className = 'rule-side';
    if (item.from_mods) appendStaticModChips(side, item.from_mods);
    if (item.keys) appendKeyChips(side, item.keys);
    if (item.from_key) appendKeyLabel(side, item.from_key);
    if (item.button) appendKeyLabel(side, `${item.button} click`);
    return side;
}

function createToSide(item) {
    const side = document.createElement('div');
    side.className = 'rule-side';
    if (item.to_mods) appendStaticModChips(side, item.to_mods);
    if (item.to_key) appendKeyLabel(side, item.to_key);
    else if (item.keys) appendHintLabel(side, 'same key');
    if (item.to_char) appendKeyLabel(side, item.to_char, 'key-label char-output');
    return side;
}

function appendGenericContent(row, item) {
    for (const [fieldKey, fieldValue] of Object.entries(item)) {
        if (fieldKey === 'global') continue;
        const span = document.createElement('span');
        span.className = 'key-label';
        span.textContent = `${fieldKey}: ${Array.isArray(fieldValue) ? fieldValue.join(', ') : fieldValue}`;
        row.appendChild(span);
    }
}

function createRemoveButton(index, path, state, rerender) {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'btn-remove';
    button.textContent = '×';
    button.addEventListener('click', () => {
        getVal(state.config, path).splice(index, 1);
        rerender();
    });
    return button;
}

function appendKeyChips(container, keys) {
    if (container.childNodes.length > 0) container.appendChild(document.createTextNode(' '));
    keys.forEach((key, index) => {
        const chip = document.createElement('span');
        chip.className = 'key-chip';
        chip.textContent = key;
        container.appendChild(chip);
        if (index < keys.length - 1) container.appendChild(document.createTextNode(' '));
    });
}

function appendKeyLabel(container, value, className = 'key-label') {
    if (container.childNodes.length > 0) container.appendChild(document.createTextNode(' '));
    const label = document.createElement('span');
    label.className = className;
    label.textContent = value;
    container.appendChild(label);
}

function appendHintLabel(container, value) {
    if (container.childNodes.length > 0) container.appendChild(document.createTextNode(' '));
    const label = document.createElement('span');
    label.className = 'key-label-hint';
    label.textContent = value;
    container.appendChild(label);
}
