import { getVal, setVal } from './config-paths.js';
import { prettyLabel } from './heuristics.js';

export function renderEmbeddedObjectMap(field, state) {
    const container = document.createElement('div');
    container.className = 'object-array-panel';

    const list = document.createElement('div');
    list.className = 'rules-list';
    container.appendChild(list);

    const addRow = document.createElement('div');
    addRow.className = 'add-rule-row object-map-add-row';
    container.appendChild(addRow);

    const saveAndRender = () => {
        renderMap(list, field, state, saveAndRender);
        state.save();
    };
    const renderOnly = () => renderMap(list, field, state, saveAndRender);
    buildAddRow(addRow, field, state, saveAndRender);
    renderOnly();
    return container;
}

function renderMap(list, field, state, rerender) {
    const entries = Object.entries(getVal(state.config, field.config_key || field.id) || {});
    list.replaceChildren();
    if (entries.length === 0) {
        const empty = document.createElement('div');
        empty.className = 'empty-state';
        empty.textContent = 'No items.';
        list.appendChild(empty);
        return;
    }
    for (const [entryKey, entryValue] of entries) {
        list.appendChild(renderEntry(field, entryKey, entryValue, state, rerender));
    }
}

function renderEntry(field, entryKey, entryValue, state, rerender) {
    const row = document.createElement('div');
    row.className = 'rule-row object-map-row';

    const key = document.createElement('span');
    key.className = 'key-label';
    key.textContent = `${field.key_label || 'Key'}: ${entryKey}`;
    row.appendChild(key);

    for (const [name, value] of Object.entries(entryValue)) {
        row.appendChild(renderValue(name, value));
    }

    const remove = document.createElement('button');
    remove.type = 'button';
    remove.className = 'btn-remove';
    remove.textContent = '×';
    remove.addEventListener('click', () => {
        const next = getVal(state.config, field.config_key || field.id) || {};
        delete next[entryKey];
        rerender();
    });
    row.appendChild(remove);
    return row;
}

function renderValue(name, value) {
    const chip = document.createElement('span');
    chip.className = 'key-label';
    chip.textContent = `${prettyLabel(name)}: ${formatValue(value)}`;
    return chip;
}

function formatValue(value) {
    if (Array.isArray(value)) {
        return value.join(', ');
    }
    return `${value}`;
}

function buildAddRow(container, field, state, rerender) {
    container.replaceChildren();
    const keyInput = document.createElement('input');
    keyInput.type = 'text';
    keyInput.className = 'key-input';
    keyInput.placeholder = field.key_label || 'Key';
    container.appendChild(keyInput);

    const entryInputs = {};
    for (const [name, kind] of Object.entries(field.entry_fields || {})) {
        entryInputs[name] = buildInput(name, kind, container);
    }

    const add = document.createElement('button');
    add.type = 'button';
    add.className = 'btn btn-ghost btn-sm btn-add';
    add.textContent = '+ Add';
    add.addEventListener('click', () => addEntry(field, state, keyInput, entryInputs, rerender));
    container.appendChild(add);
}

function buildInput(name, kind, container) {
    const input = document.createElement('input');
    input.type = kind === 'number' ? 'number' : 'text';
    input.className = kind === 'string_array' ? 'key-input keys-input' : 'key-input';
    input.placeholder = prettyLabel(name);
    container.appendChild(input);
    return { kind, input };
}

function addEntry(field, state, keyInput, entryInputs, rerender) {
    const entryKey = keyInput.value.trim();
    if (!entryKey) {
        return;
    }

    const entry = {};
    for (const [name, info] of Object.entries(entryInputs)) {
        const value = readInputValue(info);
        if (value === null) {
            continue;
        }
        entry[name] = value;
    }

    const path = field.config_key || field.id;
    const current = getVal(state.config, path) || {};
    current[entryKey] = entry;
    setVal(state.config, path, current);
    keyInput.value = '';
    for (const info of Object.values(entryInputs)) {
        info.input.value = '';
    }
    rerender();
}

function readInputValue(info) {
    const raw = info.input.value.trim();
    if (!raw) {
        if (info.kind === 'string_array') {
            return [];
        }
        return null;
    }
    if (info.kind === 'number') {
        return Number(raw);
    }
    if (info.kind === 'string_array') {
        return raw.split(',').map(value => value.trim()).filter(Boolean);
    }
    return raw;
}
